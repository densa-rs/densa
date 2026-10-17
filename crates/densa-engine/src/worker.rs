use crate::{Clock, Pacer};
use crossbeam_utils::CachePadded;
use densa_alloc::thread_stats;
use densa_data::{MarketTable, Tick, TickGenerator, ViewportSnapshot};
use densa_sync::{triple_buffer, Reader, Writer};
use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};

/// Ingestion worker settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerConfig {
    pub symbols: u32,
    pub seed: u64,
    /// Ticks generated and applied per loop iteration.
    pub batch_size: usize,
    /// Target ticks per second; `None` runs as fast as possible.
    pub rate: Option<u64>,
    /// Minimum time between snapshot publishes; 0 publishes after every batch.
    pub publish_interval_ns: u64,
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            symbols: 10_000,
            seed: 1,
            batch_size: 64,
            rate: Some(200_000),
            publish_interval_ns: 100_000,
        }
    }
}

impl WorkerConfig {
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.symbols == 0 {
            return Err(ConfigError::NoSymbols);
        }
        if self.batch_size == 0 {
            return Err(ConfigError::EmptyBatch);
        }
        if self.rate == Some(0) {
            return Err(ConfigError::ZeroRate);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigError {
    NoSymbols,
    EmptyBatch,
    ZeroRate,
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NoSymbols => "symbol count must be at least 1",
            Self::EmptyBatch => "batch size must be at least 1",
            Self::ZeroRate => "tick rate must be positive (omit it to run unbounded)",
        })
    }
}

impl std::error::Error for ConfigError {}

#[derive(Debug)]
pub enum SpawnError {
    Config(ConfigError),
    Thread(std::io::Error),
}

impl fmt::Display for SpawnError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Config(e) => write!(f, "invalid worker config: {e}"),
            Self::Thread(e) => write!(f, "failed to spawn worker thread: {e}"),
        }
    }
}

impl std::error::Error for SpawnError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Config(e) => Some(e),
            Self::Thread(e) => Some(e),
        }
    }
}

/// The worker thread panicked; its metrics up to that point are lost.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkerPanicked;

impl fmt::Display for WorkerPanicked {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ingestion worker thread panicked")
    }
}

impl std::error::Error for WorkerPanicked {}

/// Cumulative worker counters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WorkerMetrics {
    pub ticks_applied: u64,
    pub ticks_rejected: u64,
    pub batches: u64,
    pub publishes: u64,
    pub lag_resets: u64,
    /// Heap operations (alloc + dealloc + realloc) on the worker thread since its loop
    /// started. Always 0 unless `densa_alloc::CountingAllocator` is the global allocator.
    pub heap_ops: u64,
}

// ORDERING (all counters): each counter has a single writer (the worker) and is read only
// for reporting. No other data is published through them, so `Relaxed` suffices; values
// read after `JoinHandle::join` are exact because join synchronizes with the thread's end.
#[derive(Debug, Default)]
struct Counters {
    ticks_applied: AtomicU64,
    ticks_rejected: AtomicU64,
    batches: AtomicU64,
    publishes: AtomicU64,
    lag_resets: AtomicU64,
    heap_ops: AtomicU64,
}

impl Counters {
    fn store(&self, m: &WorkerMetrics) {
        self.ticks_applied.store(m.ticks_applied, Ordering::Relaxed);
        self.ticks_rejected
            .store(m.ticks_rejected, Ordering::Relaxed);
        self.batches.store(m.batches, Ordering::Relaxed);
        self.publishes.store(m.publishes, Ordering::Relaxed);
        self.lag_resets.store(m.lag_resets, Ordering::Relaxed);
        self.heap_ops.store(m.heap_ops, Ordering::Relaxed);
    }

    fn load(&self) -> WorkerMetrics {
        WorkerMetrics {
            ticks_applied: self.ticks_applied.load(Ordering::Relaxed),
            ticks_rejected: self.ticks_rejected.load(Ordering::Relaxed),
            batches: self.batches.load(Ordering::Relaxed),
            publishes: self.publishes.load(Ordering::Relaxed),
            lag_resets: self.lag_resets.load(Ordering::Relaxed),
            heap_ops: self.heap_ops.load(Ordering::Relaxed),
        }
    }
}

#[derive(Debug, Default)]
struct Shared {
    stop: CachePadded<AtomicBool>,
    viewport_first_row: CachePadded<AtomicU32>,
    counters: CachePadded<Counters>,
}

/// Control and monitoring handle for a running worker.
#[derive(Debug)]
pub struct WorkerHandle {
    shared: Arc<Shared>,
    thread: JoinHandle<()>,
}

impl WorkerHandle {
    /// Requests that future snapshots start at `first_row` (clamped by the worker).
    pub fn set_viewport(&self, first_row: u32) {
        // ORDERING: Relaxed. A hint with no dependent data; each snapshot records the
        // `first_row` it was actually built from.
        self.shared
            .viewport_first_row
            .store(first_row, Ordering::Relaxed);
    }

    /// Current counters (may be a few batches behind).
    pub fn metrics(&self) -> WorkerMetrics {
        self.shared.counters.load()
    }

    /// Stops the worker, waits for it to exit and returns its final counters.
    pub fn stop(self) -> Result<WorkerMetrics, WorkerPanicked> {
        // ORDERING: Relaxed. Only termination is signalled; `join` below provides the
        // happens-before edge for reading the final counters.
        self.shared.stop.store(true, Ordering::Relaxed);
        self.thread.join().map_err(|_| WorkerPanicked)?;
        Ok(self.shared.counters.load())
    }
}

/// Spawns the ingestion worker thread.
///
/// All buffers are allocated here, before the loop starts; the loop itself never
/// allocates. The returned [`Reader`] belongs to the render thread.
pub fn spawn_worker<const N: usize>(
    config: WorkerConfig,
    clock: Clock,
) -> Result<(WorkerHandle, Reader<ViewportSnapshot<N>>), SpawnError> {
    config.validate().map_err(SpawnError::Config)?;

    let (writer, reader) = triple_buffer(ViewportSnapshot::<N>::new());
    let shared = Arc::new(Shared::default());
    let worker = Worker {
        generator: TickGenerator::new(config.symbols, config.seed),
        table: MarketTable::new(config.symbols),
        batch: vec![Tick::default(); config.batch_size].into_boxed_slice(),
        writer,
        shared: Arc::clone(&shared),
        clock,
        config,
    };
    let thread = thread::Builder::new()
        .name("densa-ingest".into())
        .spawn(move || worker.run())
        .map_err(SpawnError::Thread)?;
    Ok((WorkerHandle { shared, thread }, reader))
}

struct Worker<const N: usize> {
    config: WorkerConfig,
    generator: TickGenerator,
    table: MarketTable,
    batch: Box<[Tick]>,
    writer: Writer<ViewportSnapshot<N>>,
    shared: Arc<Shared>,
    clock: Clock,
}

impl<const N: usize> Worker<N> {
    fn run(self) {
        let Self {
            config,
            mut generator,
            mut table,
            mut batch,
            mut writer,
            shared,
            clock,
        } = self;
        // ORDERING (stop): Relaxed, see `WorkerHandle::stop`.
        let stopping = || shared.stop.load(Ordering::Relaxed);

        let heap_baseline = thread_stats();
        let mut pacer = config
            .rate
            .map(|rate| Pacer::new(rate, config.batch_size, clock.now_ns()));
        let mut metrics = WorkerMetrics::default();
        let mut seq = 0u64;
        let mut last_publish_ns = 0u64;

        'run: while !stopping() {
            if let Some(pacer) = &pacer {
                while clock.now_ns() < pacer.deadline_ns() {
                    if stopping() {
                        break 'run;
                    }
                    std::hint::spin_loop();
                }
            }

            let now = clock.now_ns();
            generator.fill(&mut batch, now);
            let applied = table.apply_batch(&batch);
            metrics.ticks_applied += applied.applied;
            metrics.ticks_rejected += applied.rejected;
            metrics.batches += 1;
            if let Some(pacer) = &mut pacer {
                pacer.complete_batch(now);
                metrics.lag_resets = pacer.lag_resets();
            }

            if seq == 0 || now.saturating_sub(last_publish_ns) >= config.publish_interval_ns {
                seq += 1;
                // ORDERING: Relaxed, see `WorkerHandle::set_viewport`.
                let first_row = shared.viewport_first_row.load(Ordering::Relaxed);
                writer.write(|snapshot| {
                    table.snapshot_into(first_row, snapshot);
                    snapshot.seq = seq;
                    snapshot.published_at_ns = clock.now_ns();
                });
                writer.publish();
                last_publish_ns = now;
                metrics.publishes += 1;
            }

            let heap = thread_stats().since(heap_baseline);
            metrics.heap_ops = heap.allocs + heap.deallocs + heap.reallocs;
            shared.counters.store(&metrics);
        }
        shared.counters.store(&metrics);
    }
}

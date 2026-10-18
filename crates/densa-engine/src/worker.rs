use crate::{Clock, Pacer};
use crossbeam_utils::CachePadded;
use densa_alloc::thread_stats;
use densa_data::{MarketTable, Tick, TickGenerator, ViewportSnapshot};
use densa_platform::{
    cpu_registry, current_cpu, logical_cpus, pin_thread, set_thread_priority, thread_priority,
    CpuClaim, CpuError, CpuId, LogicalCpu, ThreadPriority,
};
use densa_sync::{triple_buffer, Reader, Writer};
use std::fmt;
use std::num::{NonZeroU32, NonZeroU64};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};

/// Ingestion worker settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerConfig {
    /// Number of instruments; must be at least 1.
    pub symbols: u32,
    /// Seed of the synthetic feed; equal seeds replay the same ticks.
    pub seed: u64,
    /// Ticks generated and applied per loop iteration.
    pub batch_size: usize,
    /// Target ticks per second; `None` runs as fast as possible.
    pub rate: Option<u64>,
    /// Minimum time between snapshot publishes; 0 publishes after every batch.
    pub publish_interval_ns: u64,
    /// Where to run the worker thread.
    pub pin: Pin,
    /// Scheduling priority of the worker thread. Raise it together with pinning: a pinned
    /// worker at normal priority still time-slices with other threads on its core.
    ///
    /// With [`Pin::Auto`] a raised priority applies only if a core was claimed: an
    /// unpinned busy-spinning thread above normal priority would starve every other thread
    /// on the machine (seen on a 2-CPU CI runner: the render loop fell to 0.4 fps). With an
    /// explicit [`Pin::None`] or [`Pin::Cpu`] it is applied as asked.
    pub priority: ThreadPriority,
}

/// Placement policy for the worker thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Pin {
    /// Leave placement to the OS scheduler.
    #[default]
    None,
    /// The fastest CPU on a physical core not claimed by another pinned thread of this
    /// process (see `densa_platform::CpuRegistry`). Runs unpinned where the OS reports no
    /// topology or every core is claimed.
    Auto,
    /// Exactly this CPU, even if another thread already uses it.
    Cpu(LogicalCpu),
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            symbols: 10_000,
            seed: 1,
            batch_size: 64,
            rate: Some(200_000),
            publish_interval_ns: 100_000,
            pin: Pin::None,
            priority: ThreadPriority::Normal,
        }
    }
}

impl WorkerConfig {
    /// Checks the settings without spawning anything.
    ///
    /// # Errors
    ///
    /// The first [`ConfigError`] found.
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

/// Why a [`WorkerConfig`] is invalid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigError {
    /// `symbols` is 0.
    NoSymbols,
    /// `batch_size` is 0.
    EmptyBatch,
    /// `rate` is `Some(0)`; use `None` to run unbounded.
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

/// Why [`spawn_worker`] failed.
#[derive(Debug)]
pub enum SpawnError {
    /// The configuration is invalid.
    Config(ConfigError),
    /// The OS refused to create the thread.
    Thread(std::io::Error),
    /// Pinning to `WorkerConfig::pin` failed; the worker was stopped.
    Pin(CpuError),
    /// Setting `WorkerConfig::priority` failed; the worker was stopped.
    Priority(CpuError),
}

impl fmt::Display for SpawnError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Config(e) => write!(f, "invalid worker config: {e}"),
            Self::Thread(e) => write!(f, "failed to spawn worker thread: {e}"),
            Self::Pin(e) => write!(f, "failed to pin worker thread: {e}"),
            Self::Priority(e) => write!(f, "failed to set worker thread priority: {e}"),
        }
    }
}

impl std::error::Error for SpawnError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Config(e) => Some(e),
            Self::Thread(e) => Some(e),
            Self::Pin(e) | Self::Priority(e) => Some(e),
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
    /// Ticks written into the table.
    pub ticks_applied: u64,
    /// Ticks dropped as invalid (unknown symbol, non-finite price).
    pub ticks_rejected: u64,
    /// Loop iterations, one batch each.
    pub batches: u64,
    /// Snapshots published to the reader.
    pub publishes: u64,
    /// Times the pacer restarted its schedule after falling over 10 ms behind.
    pub lag_resets: u64,
    /// Heap operations (alloc + dealloc + realloc) on the worker thread since its loop
    /// started. Always 0 unless `densa_alloc::CountingAllocator` is the global allocator.
    pub heap_ops: u64,
    /// CPU the worker ran its most recent batch on (`None` where the OS can't tell).
    pub last_cpu: Option<CpuId>,
    /// Times the scheduler moved the worker to another CPU between batches.
    pub cpu_migrations: u64,
}

/// `CpuId` packed into a `u32` for atomic storage; `u32::MAX` means unknown.
const NO_CPU: u32 = u32::MAX;

fn pack_cpu(cpu: Option<CpuId>) -> u32 {
    cpu.map_or(NO_CPU, |c| (u32::from(c.group) << 8) | u32::from(c.index))
}

fn unpack_cpu(packed: u32) -> Option<CpuId> {
    (packed != NO_CPU).then_some(CpuId {
        // Lossless: packed from a u16 group and a u8 index.
        group: (packed >> 8) as u16,
        index: (packed & 0xff) as u8,
    })
}

// ORDERING (all counters): each counter has a single writer (the worker) and is read only
// for reporting. No other data is published through them, so `Relaxed` suffices; values
// read after `JoinHandle::join` are exact because join synchronizes with the thread's end.
#[derive(Debug)]
struct Counters {
    ticks_applied: AtomicU64,
    ticks_rejected: AtomicU64,
    batches: AtomicU64,
    publishes: AtomicU64,
    lag_resets: AtomicU64,
    heap_ops: AtomicU64,
    last_cpu: AtomicU32,
    cpu_migrations: AtomicU64,
}

impl Default for Counters {
    fn default() -> Self {
        Self {
            ticks_applied: AtomicU64::new(0),
            ticks_rejected: AtomicU64::new(0),
            batches: AtomicU64::new(0),
            publishes: AtomicU64::new(0),
            lag_resets: AtomicU64::new(0),
            heap_ops: AtomicU64::new(0),
            last_cpu: AtomicU32::new(NO_CPU),
            cpu_migrations: AtomicU64::new(0),
        }
    }
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
        self.last_cpu.store(pack_cpu(m.last_cpu), Ordering::Relaxed);
        self.cpu_migrations
            .store(m.cpu_migrations, Ordering::Relaxed);
    }

    fn load(&self) -> WorkerMetrics {
        WorkerMetrics {
            ticks_applied: self.ticks_applied.load(Ordering::Relaxed),
            ticks_rejected: self.ticks_rejected.load(Ordering::Relaxed),
            batches: self.batches.load(Ordering::Relaxed),
            publishes: self.publishes.load(Ordering::Relaxed),
            lag_resets: self.lag_resets.load(Ordering::Relaxed),
            heap_ops: self.heap_ops.load(Ordering::Relaxed),
            last_cpu: unpack_cpu(self.last_cpu.load(Ordering::Relaxed)),
            cpu_migrations: self.cpu_migrations.load(Ordering::Relaxed),
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
///
/// Dropping the handle stops and joins the worker, so a busy-spinning thread is never
/// left behind; use [`WorkerHandle::stop`] to also get its final counters.
#[derive(Debug)]
pub struct WorkerHandle {
    shared: Arc<Shared>,
    /// `None` once joined.
    thread: Option<JoinHandle<()>>,
    /// Held for the thread's lifetime; released only after it has exited.
    claim: Option<CpuClaim<'static>>,
}

impl WorkerHandle {
    /// The CPU the worker is pinned to, or `None` if it runs unpinned.
    pub fn pinned_cpu(&self) -> Option<LogicalCpu> {
        self.claim.as_ref().map(CpuClaim::cpu)
    }

    /// The worker thread's priority as the OS reports it now (a system call), or `None`
    /// if it cannot be read on this platform or the worker has stopped.
    pub fn priority(&self) -> Option<ThreadPriority> {
        self.thread.as_ref().and_then(thread_priority)
    }

    /// Changes the worker thread's priority. Raising it above normal on an unpinned worker
    /// lets its busy loop starve other threads; see [`WorkerConfig::priority`].
    ///
    /// # Errors
    /// [`CpuError`] if the OS refuses or has no priorities (only Windows has them so far),
    /// or `CpuError::Unsupported` if the worker has stopped.
    pub fn set_priority(&self, priority: ThreadPriority) -> Result<(), CpuError> {
        let thread = self.thread.as_ref().ok_or(CpuError::Unsupported)?;
        set_thread_priority(thread, priority)
    }

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
    ///
    /// # Errors
    ///
    /// [`WorkerPanicked`] if the worker thread panicked.
    pub fn stop(mut self) -> Result<WorkerMetrics, WorkerPanicked> {
        self.shutdown()?;
        Ok(self.shared.counters.load())
    }

    fn shutdown(&mut self) -> Result<(), WorkerPanicked> {
        // ORDERING: Relaxed. Only termination is signalled; `join` below provides the
        // happens-before edge for reading the final counters.
        self.shared.stop.store(true, Ordering::Relaxed);
        let Some(thread) = self.thread.take() else {
            return Ok(());
        };
        let joined = thread.join().map_err(|_| WorkerPanicked);
        self.claim = None;
        joined
    }
}

impl Drop for WorkerHandle {
    fn drop(&mut self) {
        // Nowhere to report a panic from here; `stop` is the way to observe it.
        let _ = self.shutdown();
    }
}

/// Resolves the placement policy into a claim on the process-wide CPU registry.
fn claim_cpu(pin: Pin) -> Option<CpuClaim<'static>> {
    match pin {
        Pin::None => None,
        Pin::Cpu(cpu) => Some(cpu_registry().claim(cpu)),
        // No topology on this OS, or every core claimed: run unpinned, visible as
        // `pinned_cpu() == None`.
        Pin::Auto => logical_cpus()
            .ok()
            .and_then(|cpus| cpu_registry().claim_auto(&cpus)),
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
    let symbols =
        NonZeroU32::new(config.symbols).ok_or(SpawnError::Config(ConfigError::NoSymbols))?;
    let claim = claim_cpu(config.pin);
    let auto_without_core = config.pin == Pin::Auto && claim.is_none();
    let priority = if auto_without_core {
        ThreadPriority::Normal
    } else {
        config.priority
    };
    let worker = Worker {
        generator: TickGenerator::new(symbols, config.seed),
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
    let handle = WorkerHandle {
        shared,
        thread: Some(thread),
        claim,
    };

    // Pinned from outside through the thread handle so failure is reported synchronously.
    // The worker may run a few microseconds unpinned before the OS moves it.
    if let (Some(thread), Some(cpu)) = (&handle.thread, handle.pinned_cpu()) {
        if let Err(e) = pin_thread(thread, cpu) {
            drop(handle); // Stops the worker and releases the claim.
            return Err(SpawnError::Pin(e));
        }
    }
    if let (Some(thread), true) = (&handle.thread, priority != ThreadPriority::Normal) {
        if let Err(e) = set_thread_priority(thread, priority) {
            drop(handle);
            return Err(SpawnError::Priority(e));
        }
    }
    Ok((handle, reader))
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
        // A zero rate was already rejected by `WorkerConfig::validate`.
        let mut pacer = config
            .rate
            .and_then(NonZeroU64::new)
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
            let cpu = current_cpu();
            if metrics.last_cpu.is_some() && cpu != metrics.last_cpu {
                metrics.cpu_migrations += 1;
            }
            metrics.last_cpu = cpu;
            shared.counters.store(&metrics);
        }
        shared.counters.store(&metrics);
    }
}

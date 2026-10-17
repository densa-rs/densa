use crate::PinRequest;
use densa_engine::WorkerConfig;
use std::fmt;

pub const USAGE: &str = "\
Usage: densa-poc [OPTIONS]

Modes:
  (default)                  Open a window rendering the live grid (wgpu, V-Sync)
  --headless                 Run the same frame loop without a window (prints metrics)

Window controls: mouse wheel, Up/Down, PageUp/PageDown, Home/End scroll; Esc quits.

Options:
  --seconds <N>              Run time in seconds [default: 10 headless, until closed in a window]
  --rate <TICKS_PER_SEC>     Target tick rate [default: 200000]
  --unbounded                Generate ticks as fast as possible (conflicts with --rate)
  --symbols <N>              Number of instruments [default: 10000]
  --batch <N>                Ticks per worker batch [default: 64]
  --publish-interval-us <N>  Minimum time between snapshot publishes [default: 100]
  --fps <N>                  Frame rate of the render loop [default: 120]
  --seed <N>                 Random seed of the synthetic feed [default: 1]
  --pin <auto|none|CPU>      Pin the worker thread: `auto` picks a performance core,
                             `none` leaves placement to the OS [default: auto]
  --list-cpus                Print the CPU topology (core kinds, --pin choice) and exit
  -h, --help                 Print this help
";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Window,
    Headless,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Args {
    pub mode: Mode,
    pub help: bool,
    /// `None`: headless runs `DEFAULT_HEADLESS_SECONDS`, a window runs until closed.
    pub seconds: Option<u64>,
    pub fps: u32,
    pub worker: WorkerConfig,
    /// Resolved into `worker.pin` against the live CPU topology at startup.
    pub pin: PinRequest,
    pub list_cpus: bool,
}

/// A user-facing command-line error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArgsError(String);

impl fmt::Display for ArgsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ArgsError {}

fn error(message: impl Into<String>) -> ArgsError {
    ArgsError(message.into())
}

impl Args {
    pub fn parse(args: impl IntoIterator<Item = String>) -> Result<Self, ArgsError> {
        let mut parsed = Self {
            mode: Mode::Window,
            help: false,
            seconds: None,
            fps: 120,
            worker: WorkerConfig::default(),
            // On hybrid CPUs an unpinned busy-spinning worker migrates hundreds of times
            // per second and runs 2-4x slower whenever it lands on an efficiency core
            // (scripts/pinning_experiment.ps1).
            pin: PinRequest::Auto,
            list_cpus: false,
        };
        let mut rate = None;
        let mut unbounded = false;

        let mut args = args.into_iter();
        while let Some(arg) = args.next() {
            let mut value = || {
                args.next()
                    .ok_or_else(|| error(format!("`{arg}` needs a value")))
            };
            match arg.as_str() {
                "--headless" => parsed.mode = Mode::Headless,
                "-h" | "--help" => parsed.help = true,
                "--unbounded" => unbounded = true,
                "--list-cpus" => parsed.list_cpus = true,
                "--pin" => parsed.pin = parse_pin(&value()?)?,
                "--seconds" => parsed.seconds = Some(parse_u64(&arg, &value()?)?),
                "--rate" => rate = Some(parse_u64(&arg, &value()?)?),
                "--symbols" => parsed.worker.symbols = parse_bounded(&arg, &value()?)?,
                "--batch" => parsed.worker.batch_size = parse_bounded(&arg, &value()?)?,
                "--fps" => parsed.fps = parse_bounded(&arg, &value()?)?,
                "--seed" => parsed.worker.seed = parse_u64(&arg, &value()?)?,
                "--publish-interval-us" => {
                    let micros = parse_u64(&arg, &value()?)?;
                    parsed.worker.publish_interval_ns = micros
                        .checked_mul(1_000)
                        .ok_or_else(|| error(format!("`{arg}` is too large")))?;
                }
                _ => return Err(error(format!("unknown argument `{arg}`"))),
            }
        }

        if parsed.help {
            return Ok(parsed);
        }
        if rate.is_some() && unbounded {
            return Err(error("`--rate` and `--unbounded` are mutually exclusive"));
        }
        if unbounded {
            parsed.worker.rate = None;
        } else if rate.is_some() {
            parsed.worker.rate = rate;
        }
        if parsed.seconds == Some(0) {
            return Err(error("`--seconds` must be at least 1"));
        }
        if parsed.fps == 0 {
            return Err(error("`--fps` must be at least 1"));
        }
        parsed.worker.validate().map_err(|e| error(e.to_string()))?;
        Ok(parsed)
    }
}

fn parse_pin(raw: &str) -> Result<PinRequest, ArgsError> {
    match raw {
        "auto" => Ok(PinRequest::Auto),
        "none" => Ok(PinRequest::None),
        _ => raw.parse().map(PinRequest::Index).map_err(|_| {
            error(format!(
                "`--pin` expects `auto`, `none` or a CPU index (0-255), got `{raw}`"
            ))
        }),
    }
}

fn parse_u64(flag: &str, raw: &str) -> Result<u64, ArgsError> {
    raw.parse().map_err(|_| {
        error(format!(
            "`{flag}` expects a non-negative integer, got `{raw}`"
        ))
    })
}

fn parse_bounded<T: TryFrom<u64> + fmt::Display + Bounded>(
    flag: &str,
    raw: &str,
) -> Result<T, ArgsError> {
    let wide = parse_u64(flag, raw)?;
    T::try_from(wide).map_err(|_| error(format!("`{flag}` is too large (max {})", T::MAX)))
}

/// Integer types narrower than `u64` that an option can target.
trait Bounded {
    const MAX: Self;
}

impl Bounded for u32 {
    const MAX: Self = u32::MAX;
}

impl Bounded for usize {
    const MAX: Self = usize::MAX;
}

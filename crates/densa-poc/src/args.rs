use densa_engine::WorkerConfig;
use std::fmt;

pub const USAGE: &str = "\
Usage: densa-poc [OPTIONS]

Modes:
  --headless                 Run the render loop without a window (prints metrics)
                             (window mode is the default once the renderer lands)

Options:
  --seconds <N>              Run time in seconds [default: 10]
  --rate <TICKS_PER_SEC>     Target tick rate [default: 200000]
  --unbounded                Generate ticks as fast as possible (conflicts with --rate)
  --symbols <N>              Number of instruments [default: 10000]
  --batch <N>                Ticks per worker batch [default: 64]
  --publish-interval-us <N>  Minimum time between snapshot publishes [default: 100]
  --fps <N>                  Frame rate of the render loop [default: 120]
  --seed <N>                 Random seed of the synthetic feed [default: 1]
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
    pub seconds: u64,
    pub fps: u32,
    pub worker: WorkerConfig,
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
            seconds: 10,
            fps: 120,
            worker: WorkerConfig::default(),
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
                "--seconds" => parsed.seconds = parse_u64(&arg, &value()?)?,
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
        if parsed.seconds == 0 {
            return Err(error("`--seconds` must be at least 1"));
        }
        if parsed.fps == 0 {
            return Err(error("`--fps` must be at least 1"));
        }
        parsed.worker.validate().map_err(|e| error(e.to_string()))?;
        Ok(parsed)
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

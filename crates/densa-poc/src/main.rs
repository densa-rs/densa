use densa_alloc::{is_active, CountingAllocator};
use densa_platform::logical_cpus;
use densa_poc::{run_headless, write_cpu_table, Args, Mode, USAGE};
use std::process::ExitCode;

/// Counts heap operations per thread so the run can prove its hot paths allocate nothing.
/// Overhead is one thread-local increment per allocation.
#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator::system();

fn main() -> ExitCode {
    let args = match Args::parse(std::env::args().skip(1)) {
        Ok(args) => args,
        Err(e) => {
            eprintln!("error: {e}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    if args.help {
        print!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    if args.list_cpus {
        let listed = logical_cpus().map_err(|e| e.to_string()).and_then(|cpus| {
            write_cpu_table(&mut std::io::stdout().lock(), &cpus).map_err(|e| e.to_string())
        });
        return match listed {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("error: cannot list CPUs: {e}");
                ExitCode::FAILURE
            }
        };
    }
    if !is_active() {
        eprintln!("warning: allocation counting is inactive; heap-op metrics will read 0");
    }

    match args.mode {
        Mode::Window => {
            eprintln!("error: window mode is not implemented yet; run with --headless\n\n{USAGE}");
            ExitCode::from(2)
        }
        Mode::Headless => match run_headless(&args, &mut std::io::stdout().lock()) {
            Ok(_) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("error: {e}");
                ExitCode::FAILURE
            }
        },
    }
}

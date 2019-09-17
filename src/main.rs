#![cfg_attr(feature = "clippy", feature(plugin))]
#![cfg_attr(feature = "clippy", plugin(clippy))]

use {
    console::style,
    rs_tracing::{
        close_trace_file, close_trace_file_internal, open_trace_file, trace_scoped,
        trace_scoped_internal, trace_to_file_internal,
    },
    structopt::StructOpt,
};

mod gtest;

#[derive(StructOpt, Debug)]
struct Opt {
    /// Number of parallel jobs
    ///
    /// This flag controls how many parallel jobs are used to execute test shards. We do not
    /// execute more jobs than there are tests (also see `progress`). Depending on the exact test
    /// workload, test execution typically becomes faster with more jobs until it reaches a plateau
    /// or even decreases when too many parallel executions compete for system resources (e.g.,
    /// file system access; scheduling by the processor).
    ///
    /// This flag can be controlled with an environment variable and by default is set to the
    /// number of processors available to the runner process.
    #[structopt(long, short, env = "GTEST_RUNNER_JOBS")]
    jobs: Option<usize>,

    /// Runner verbosity
    ///
    /// This flag controls the verbosity with which the test runner reports execution progress and results.
    ///
    /// v=0: Do not provide any output during test execution. Report failed tests at the end.
    /// v=1: Report global test progress. Report failed tests at the end.
    /// v=2: Report currently executing tests. Report failed tests at the end.
    /// v>2: Pass through and report all test output.
    ///
    /// This flag can be controlled with an environment variable and has a default value
    #[structopt(long, short, default_value = "2", env = "GTEST_RUNNER_VERBOSITY")]
    verbosity: u64,

    /// Dump chrome://tracing trace to current directory
    ///
    /// If this flag is present a chrome://tracing execution trace
    /// (http://dev.chromium.org/developers/how-tos/trace-event-profiling-tool) will be dumped to
    /// the current directory as `<pid>.trace` which can be used to analyze e.g., temporal
    /// relations between tests or their duration. The resulting file can e.g., directly be loaded
    /// into Google Chrome under chrome://tracing, or converted to HTML with `trace2html`.
    // We explicitly do not declare `env` for this flag as clap implicitly sets
    // `Arg::takes_value(true)` which turns this from a flag to an option, see
    // https://github.com/TeXitoi/structopt/issues/176.
    #[structopt(long, short)]
    trace: bool,

    /// Repeat failed tests
    ///
    /// If this flag is given a non-zero value, failed tests will be repeated up to `repeat` times.
    #[structopt(long, short, default_value = "0", env = "GTEST_RUNNER_REPEAT")]
    repeat: u64,

    /// GTest executable(s)
    ///
    /// The test runner can execute tests from the same executable in parallel, but will currently
    /// not run different test executables in parallel. In order for tests to be executable in
    /// parallel they likely should not depend on system information (e.g., the ability to bind to
    /// fixed ports; the presence or absence of especially test-created files in fixed file system
    /// locations, etc.).
    #[structopt(required = true)]
    test_executables: Vec<String>,
}

fn main() -> Result<(), String> {
    let opt = Opt::from_args();

    if opt.trace {
        open_trace_file!(".").unwrap();
    }

    let mut ret_vec = Vec::new();
    for exe in &opt.test_executables {
        if opt.test_executables.len() > 1 && opt.verbosity > 0 {
            println!("{}", style(format!("Running {}", exe)).bold());
        }
        trace_scoped!(&exe);
        ret_vec.push(gtest::run(
            exe,
            None,
            opt.jobs.unwrap_or_else(num_cpus::get),
            opt.verbosity,
            opt.repeat,
        )?);
    }

    close_trace_file!();

    std::process::exit(ret_vec.iter().sum::<usize>() as i32);
}

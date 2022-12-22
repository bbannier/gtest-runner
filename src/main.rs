#![cfg_attr(feature = "clippy", feature(plugin))]
#![cfg_attr(feature = "clippy", plugin(clippy))]

use {
    anyhow::{anyhow, Result},
    clap::Parser,
    console::style,
    rs_tracing::{
        close_trace_file, close_trace_file_internal, open_trace_file, trace_scoped,
        trace_scoped_internal, trace_to_file_internal,
    },
    std::convert::TryFrom,
};

mod gtest;

#[derive(Parser, Debug, Default)]
pub struct Opt {
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
    #[clap(long, short, env = "GTEST_RUNNER_JOBS")]
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
    #[clap(long, short, default_value = "2", env = "GTEST_RUNNER_VERBOSITY")]
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
    #[clap(long, short)]
    trace: bool,

    /// Repeat failed tests
    ///
    /// If this flag is given a non-zero value, failed tests will be repeated up to `repeat` times.
    #[clap(long, short, default_value = "0", env = "GTEST_RUNNER_REPEAT")]
    repeat: u64,

    /// GTest executable(s)
    ///
    /// The test runner can execute tests from the same executable in parallel, but will currently
    /// not run different test executables in parallel. In order for tests to be executable in
    /// parallel they likely should not depend on system information (e.g., the ability to bind to
    /// fixed ports; the presence or absence of especially test-created files in fixed file system
    /// locations, etc.).
    #[clap(required = true)]
    test_executables: Vec<String>,
}

pub fn exec(opt: &Opt) -> Result<i32> {
    if opt.trace {
        open_trace_file!(".").unwrap();
    }

    let available_parallelism = std::thread::available_parallelism()?.into();

    let mut ret_vec = Vec::new();
    for exe in &opt.test_executables {
        if opt.test_executables.len() > 1 && opt.verbosity > 0 {
            println!("{}", style(format!("Running {}", exe)).bold());
        }
        trace_scoped!(exe);
        ret_vec.push(gtest::run(
            exe,
            None,
            opt.jobs.unwrap_or(available_parallelism),
            opt.verbosity,
            opt.repeat,
        )?);
    }

    close_trace_file!();

    i32::try_from(ret_vec.iter().sum::<usize>()).map_err(|e| anyhow!(e.to_string()))
}

#[test]
fn test_trace() {
    let opt = Opt {
        trace: true,
        test_executables: vec![gtest::test_executable().to_str().unwrap().to_string()],
        ..Default::default()
    };

    let cwd = std::env::current_dir().expect("Could not get current directory");

    let get_traces = |dir: &std::path::PathBuf| -> std::collections::HashSet<_> {
        std::fs::read_dir(&dir)
            .expect("Could not list directory")
            .map(|entry| entry.expect("Could not get directory entry").path())
            .filter(|path| {
                std::path::Path::new(&path)
                    .extension()
                    .and_then(std::ffi::OsStr::to_str)
                    .map(|ext| ext == "trace")
                    .is_some()
            })
            .collect()
    };

    let traces1 = get_traces(&cwd);
    exec(&opt).expect("Could not execute test executable");
    let traces2 = get_traces(&cwd);

    let traces = traces2.difference(&traces1).collect::<Vec<_>>();
    assert_eq!(
        traces.len(),
        1,
        "Expected exactly one trace file to be created"
    );

    let trace = traces[0];
    let size = trace
        .metadata()
        .expect("Could not get trace metadata")
        .len();
    assert!(size > 100, "Unexpected of small size of trace file");
    std::fs::remove_file(trace).expect("Could not remove test trace");
}

fn main() -> Result<()> {
    let opt = Opt::parse();

    std::process::exit(exec(&opt)?);
}

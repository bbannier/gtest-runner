use clap::Parser;

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
    pub jobs: Option<usize>,

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
    pub verbosity: u64,

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
    pub trace: bool,

    /// Repeat failed tests
    ///
    /// If this flag is given a non-zero value, failed tests will be repeated up to `repeat` times.
    #[clap(long, short, default_value = "0", env = "GTEST_RUNNER_REPEAT")]
    pub repeat: u64,

    #[clap(flatten)]
    pub mode: RunMode,
}

#[derive(clap::Args, Default, Debug)]
#[group(required = true, multiple = false)]
pub struct RunMode {
    /// GTest executable(s)
    ///
    /// The test runner can execute tests from the same executable in parallel, but will currently
    /// not run different test executables in parallel. In order for tests to be executable in
    /// parallel they likely should not depend on system information (e.g., the ability to bind to
    /// fixed ports; the presence or absence of especially test-created files in fixed file system
    /// locations, etc.).
    #[clap(required = true)]
    pub test_executables: Option<Vec<String>>,

    /// Provide sample GTest data for testing.
    #[clap(long, env = "GTEST_RUNNER_SAMPLE_DATA")]
    pub sample_data: Option<bool>,
}

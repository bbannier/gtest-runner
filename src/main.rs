#![cfg_attr(feature = "clippy", feature(plugin))]
#![cfg_attr(feature = "clippy", plugin(clippy))]

#[macro_use]
extern crate clap;
#[macro_use]
extern crate rs_tracing;

#[cfg(test)]
extern crate itertools;

extern crate console;
extern crate crossbeam;
extern crate indicatif;
extern crate num_cpus;

use clap::{App, Arg};
use console::style;

mod gtest;

fn main() -> Result<(), String> {
    let clap_settings = &[clap::AppSettings::ColorAuto, clap::AppSettings::ColoredHelp];

    let default_jobs = num_cpus::get().to_string();

    let matches = App::new("gtest-runner")
        .settings(clap_settings)
        .version(crate_version!())
        .about(crate_description!())
        .arg(
            Arg::with_name("jobs")
                .long("jobs")
                .short("j")
                .env("GTEST_RUNNER_JOBS")
                .takes_value(true)
                .default_value(&default_jobs)
                .help("Number of parallel jobs")
                .long_help("Number of parallel jobs.\n\nThis flag controls how many parallel jobs are used to execute test shards. We do not execute more jobs than there are tests (also see `progress`). Depending on the exact test workload, test execution typically becomes faster with more jobs until it reaches a plateau or even decreases when too many parallel executions compete for system resources (e.g., file system access; scheduling by the processor).\n\nThis flag can be controlled with an environment variable and by default is set to the number of processors available to the runner process"),
        )
        .arg(
            Arg::with_name("test_executable")
                .required(true)
                .multiple(true)
                .takes_value(false)
                .help("GTest executable(s)")
                .long_help("One or more GTest executables.\n\nThe test runner can execute tests from the same executable in parallel, but will currently not run different test executables in parallel. In order for tests to be executable in parallel they likely should not depend on system information (e.g., the ability to bind to fixed ports; the presence or absence of especially test-created files in fixed file system locations, etc.).",
                ),
        )
        .arg(
            Arg::with_name("verbosity")
                .long("verbosity")
                .short("v")
                .env("GTEST_RUNNER_VERBOSITY")
                .takes_value(true)
                .default_value("2")
                .help("Runner verbosity")
                .long_help("Runner verbosity.\n\nThis flag controls the verbosity with which the test runner reports execution progress and results.\n\nv=0: Do not provide any output during test execution. Report failed tests at the end.\nv=1: Report global test progress. Report failed tests at the end.\nv=2: Report currently executing tests. Report failed tests at the end.\nv>2: Pass through and report all test output.\n\nThis flag can be controlled with an environment variable and has a default value"),
        )
        .arg(
            Arg::with_name("trace")
                .long("trace")
                .short("t")
                .takes_value(false)
                .help("Dumps chrome://tracing trace to current directory")
                .long_help("Control tracing output.\n\nIf this flag is present a chrome://tracing execution trace (http://dev.chromium.org/developers/how-tos/trace-event-profiling-tool) will be dumped to the current directory as `<pid>.trace` which can be used to analyze e.g., temporal relations between tests or their duration. The resulting file can e.g., directly be loaded into Google Chrome under chrome://tracing, or converted to HTML with `trace2html`."),
        )
        .get_matches();

    let jobs = matches
        .value_of("jobs")
        .ok_or("Expected the 'jobs' parameter to be set")?
        .parse::<u64>()
        .map_err(|e| e.to_string())?;

    let verbosity = matches
        .value_of("verbosity")
        .ok_or("Expected the 'verbosity' parameter to be set")?
        .parse::<u64>()
        .map_err(|e| e.to_string())?;

    let trace = matches.is_present("trace");

    let test_executables = matches
        .values_of("test_executable")
        .ok_or("Expected the 'test_executable' parameter to be set")?;
    let multiple_tests = test_executables.len() > 1;

    if trace {
        open_trace_file!(".").unwrap();
    }

    let mut ret_vec = Vec::new();
    for exe in test_executables {
        if multiple_tests && verbosity > 0 {
            println!("{}", style(format!("Running {}", exe)).bold());
        }
        trace_scoped!(&exe);
        ret_vec.push(gtest::run(exe, jobs, verbosity)?);
    }

    close_trace_file!();

    std::process::exit(ret_vec.iter().sum::<u64>() as i32);
}

#![cfg_attr(feature = "clippy", feature(plugin))]
#![cfg_attr(feature = "clippy", plugin(clippy))]

#[macro_use]
extern crate clap;
#[macro_use]
extern crate rs_tracing;

extern crate console;
extern crate indicatif;
extern crate itertools;
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
                .default_value(&default_jobs),
        )
        .arg(
            Arg::with_name("test_executable")
                .required(true)
                .multiple(true)
                .takes_value(false),
        )
        .arg(
            Arg::with_name("verbosity")
                .long("verbosity")
                .short("v")
                .env("GTEST_RUNNER_VERBOSITY")
                .takes_value(true)
                .default_value("2"),
        )
        .arg(
            Arg::with_name("trace")
                .long("trace")
                .short("t")
                .env("GTEST_RUNNER_TRACE")
                .takes_value(false)
                .help("Dumps chrome://tracing trace to current directory")
        )
        .get_matches();

    let jobs = matches
        .value_of("jobs")
        .ok_or("Expected the 'jobs' parameter to be set")?
        .parse::<usize>()
        .map_err(|e| e.to_string())?;

    let verbosity = matches
        .value_of("verbosity")
        .ok_or("Expected the 'verbosity' parameter to be set")?
        .parse::<usize>()
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

    std::process::exit(ret_vec.iter().sum::<usize>() as i32);
}

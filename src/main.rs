#![cfg_attr(feature = "clippy", feature(plugin))]
#![cfg_attr(feature = "clippy", plugin(clippy))]

#[macro_use]
extern crate clap;

extern crate console;
extern crate indicatif;
extern crate itertools;
extern crate num_cpus;
extern crate regex;

use clap::{App, Arg};
use console::style;

mod gtest_runner;

fn main() {
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
                .default_value("2"),
        )
        .get_matches();

    let jobs = matches.value_of("jobs").unwrap().parse::<usize>().unwrap();

    let verbosity = matches
        .value_of("verbosity")
        .unwrap()
        .parse::<usize>()
        .unwrap();

    let test_executables = matches.values_of("test_executable").unwrap();
    let multiple_tests = test_executables.len() > 1;

    let mut ret_vec = Vec::new();
    for exe in test_executables {
        if multiple_tests {
            if verbosity > 0 {
                println!("{}", style(format!("Running {}", exe)).bold());
            }
        }
        ret_vec.push(gtest_runner::run(
            std::path::PathBuf::from(exe).as_path(),
            jobs,
            verbosity,
        ));
    }

    std::process::exit(ret_vec.iter().sum::<usize>() as i32);
}

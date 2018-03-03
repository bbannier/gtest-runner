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
use console::{strip_ansi_codes, style};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use itertools::Itertools;
use regex::Regex;

use std::collections::HashSet;
use std::iter::FromIterator;
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

#[derive(Debug, PartialEq)]
enum GTestStatus {
    STARTING,
    RUNNING,
    OK,
    FAILED,
}
use GTestStatus::*;

#[derive(Debug)]
struct GTestResult {
    pub testcase: String,
    pub log: Vec<String>,
    pub status: GTestStatus,
}

struct GTestParser<T: Iterator> {
    testcase: String,
    log: Vec<String>,
    reader: T,
}

impl<T> GTestParser<T>
where
    T: Iterator<Item = String>,
{
    fn new(reader: T) -> GTestParser<T> {
        GTestParser {
            testcase: String::new(),
            log: vec![],
            reader,
        }
    }
}

impl<T> Iterator for GTestParser<T>
where
    T: Iterator<Item = String>,
{
    type Item = GTestResult;

    fn next(&mut self) -> Option<GTestResult> {
        let starting = Regex::new(r"^\[ RUN      \] .*").unwrap();
        let ok = Regex::new(r"^\[       OK \] .* \(\d* .*\)").unwrap();
        let failed = Regex::new(r"^\[  FAILED  \] .* \(\d* .*\)").unwrap();

        if let Some(line) = self.reader.next() {
            let status = {
                let line = strip_ansi_codes(&line);

                if ok.is_match(&line) {
                    OK
                } else if failed.is_match(&line) {
                    FAILED
                } else if starting.is_match(&line) {
                    STARTING
                } else {
                    RUNNING
                }
            };

            match status {
                STARTING => {
                    self.testcase = String::from(
                        strip_ansi_codes(&line).to_string()[12..]
                            .split_whitespace()
                            .next()
                            .unwrap(),
                    );
                    self.log = vec![line];
                }
                _ => {
                    self.log.push(line);
                }
            };

            if self.testcase.is_empty() {
                return self.next();
            }

            return Some(GTestResult {
                testcase: self.testcase.clone(),
                log: self.log.clone(),
                status,
            });
        }

        None
    }
}

#[test]
fn test_parse_one() {
    let output = "Note: Google Test filter = *NOPE*-
[==========] Running 3 tests from 1 test case.
[----------] Global test environment set-up.
[----------] 3 tests from NOPE
[ RUN      ] NOPE.NOPE1
[       OK ] NOPE.NOPE1 (1 ms)
[ RUN      ] NOPE.NOPE2
../3rdparty/libprocess/src/tests/future_tests.cpp:886: Failure
Value of: false
  Actual: false
Expected: true
[  FAILED  ] NOPE.NOPE2 (0 ms)
[ RUN      ] NOPE.NOPE3
../3rdparty/libprocess/src/tests/future_tests.cpp:892: Failure
Value of: false
  Actual: false
Expected: true
[  FAILED  ] NOPE.NOPE3 (0 ms)
[----------] 3 tests from NOPE (1 ms total)

[----------] Global test environment tear-down
[==========] 3 tests from 1 test case ran. (1 ms total)
[  PASSED  ] 1 test.
[  FAILED  ] 2 tests, listed below:
[  FAILED  ] NOPE.NOPE2
[  FAILED  ] NOPE.NOPE3

 2 FAILED TESTS";

    assert_eq!(
        vec!["NOPE.NOPE1"],
        Vec::from_iter(
            GTestParser::new(output.split('\n').map(|line| String::from(line)))
                .filter(|result| result.status == OK)
                .map(|result| result.testcase),
        )
    );

    assert_eq!(
        vec!["NOPE.NOPE2", "NOPE.NOPE3"],
        Vec::from_iter(
            GTestParser::new(output.split('\n').map(|line| String::from(line)))
                .filter(|result| result.status == FAILED)
                .map(|result| result.testcase),
        )
    );

    assert_eq!(
        vec!["NOPE.NOPE1", "NOPE.NOPE2", "NOPE.NOPE3"],
        Vec::from_iter(
            GTestParser::new(output.split('\n').map(|line| String::from(line)))
                .filter(|result| result.status == STARTING)
                .map(|result| result.testcase)
                .dedup(),
        )
    );
}

fn get_tests(test_executable: &Path) -> Result<HashSet<String>, &str> {
    let result = Command::new(test_executable)
        .args(&["--gtest_list_tests"])
        .output()
        .expect("Failed to execute process");

    if !result.status.success() {
        return Err("Failed to run program");
    }

    let output = String::from_utf8_lossy(&result.stdout);

    let mut tests = HashSet::new();

    let mut current_test: Option<&str> = None;
    for line in output.lines() {
        if !line.starts_with(' ') {
            current_test = line.split_whitespace().next();
        } else {
            let case = &line.split_whitespace().next().unwrap();
            tests.insert(match current_test {
                Some(t) => [t, case].concat(),
                None => panic!("Couldn't determine test name"),
            });
        }
    }

    Ok(tests)
}

fn run(test_executable: &Path, jobs: usize) {
    let pb = ProgressBar::new(100);
    pb.set_style(ProgressStyle::default_spinner().template("{msg}"));
    pb.set_message("Determining number of tests ...");
    let tests = get_tests(test_executable).unwrap();

    pb.finish_and_clear();

    let m = MultiProgress::new();

    let progress_global = Arc::new(m.add(ProgressBar::new(tests.len() as u64)));
    progress_global.set_style(
        ProgressStyle::default_spinner()
            .template("{spinner:.green} {msg} {bar} [{pos}/{len}] {elapsed_precise}"),
    );
    progress_global.set_message("Running tests ...");

    let spinner_style = ProgressStyle::default_spinner().template("{spinner} {wide_msg}");

    let (tx, rx) = mpsc::channel::<GTestResult>();

    for job in 0..jobs {
        let progress_shard = m.add(ProgressBar::new(100));
        progress_shard.set_style(spinner_style.clone());

        let output = Command::new(&test_executable)
            .env("GTEST_SHARD_INDEX", job.to_string())
            .env("GTEST_TOTAL_SHARDS", jobs.to_string())
            .env("GTEST_COLOR", "YES")
            .stderr(Stdio::null())
            .stdout(Stdio::piped())
            .spawn()
            .expect("Could not launch")
            .stdout
            .ok_or_else(|| "Could not capture output")
            .unwrap();

        // FIXME(bbannier): collect return value.

        let reader = BufReader::new(output);
        let progress_global = progress_global.clone();
        let thread_tx = tx.clone();

        let _ = thread::spawn(move || {
            let lines = reader.lines().map(|line| match line {
                Ok(line) => line,
                Err(err) => panic!(err),
            });
            for t in GTestParser::new(lines) {
                progress_shard.inc(1);

                match t.status {
                    OK => {
                        progress_global.inc(1);
                    }
                    FAILED => {
                        progress_global.inc(1);
                        progress_shard.set_message(&format!("{}", style(&t.testcase).red()));
                        thread::sleep(Duration::from_millis(500)); // FIXME(bbannier): this is silly, add a global counter somewhere.
                        thread_tx.send(t).unwrap();
                    }
                    STARTING => {
                        progress_shard.set_message(&format!("{}", t.testcase));
                    }
                    _ => {}
                }
            }

            progress_shard.finish();
        });
    }

    let _ = thread::spawn(move || {
        while Arc::strong_count(&progress_global) > 1 {
            std::thread::sleep(Duration::from_millis(10));
        }
        progress_global.finish_with_message("All done");
    });

    m.join_and_clear().unwrap();

    let failures = Vec::from_iter(rx.try_iter());
    if failures.is_empty() {
        println!(
            "{}",
            style(format!("{} tests passed", tests.len()))
                .bold()
                .green()
        );
    } else {
        println!(
            "{}",
            style(format!(
                "{} out of {} tests failed\n",
                failures.len(),
                tests.len()
            )).bold()
                .red()
        );
        println!(
            "{}",
            failures.iter().map(|f| f.log.iter().join("\n")).join("\n")
        );
    }
}

fn main() {
    let clap_settings = &[clap::AppSettings::ColorAuto, clap::AppSettings::ColoredHelp];

    let default_jobs = num_cpus::get().to_string();

    let matches = App::new("mesos-gtest-runner")
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
        .get_matches();

    let jobs = matches.value_of("jobs").unwrap().parse::<usize>().unwrap();

    let test_executables = matches.values_of("test_executable").unwrap();
    let multiple_tests = test_executables.len() > 1;

    for exe in test_executables {
        if multiple_tests {
            println!("{}", style(format!("Running {}", exe)).bold());
        }
        run(std::path::PathBuf::from(exe).as_path(), jobs);
    }
}

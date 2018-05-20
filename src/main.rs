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
use std::io::{BufRead, BufReader};
use std::iter::FromIterator;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

#[derive(Clone, Debug, PartialEq)]
enum GTestStatus {
    STARTING,
    RUNNING,
    OK,
    FAILED,
    ABORTED,
}
use GTestStatus::*;

impl GTestStatus {
    fn is_terminal(&self) -> bool {
        match self {
            STARTING | RUNNING => false,
            ABORTED | OK | FAILED => true,
        }
    }
}

#[derive(Debug, Clone)]
struct GTestResult {
    pub testcase: String,
    pub log: Vec<String>,
    pub status: GTestStatus,
}

struct GTestParser<T: Iterator> {
    testcase: Option<String>,
    log: Vec<String>,
    reader: T,
}

impl<T> GTestParser<T>
where
    T: Iterator<Item = String>,
{
    fn new(reader: T) -> GTestParser<T> {
        GTestParser {
            testcase: None,
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
                    self.testcase = Some(String::from(
                        strip_ansi_codes(&line).to_string()[12..]
                            .split_whitespace()
                            .next()
                            .unwrap(),
                    ));
                    self.log = vec![line];
                }
                _ => {
                    self.log.push(line);
                }
            };

            // Do not report until we have found a test case.
            if self.testcase.is_none() {
                return self.next();
            }

            let result = GTestResult {
                testcase: self.testcase.clone().unwrap(),
                log: self.log.clone(),
                status: status.clone(),
            };

            // Unset the current test case for terminal transitions.
            // This allows us to detect aborts.
            if status.is_terminal() {
                self.testcase = None;
            }

            return Some(result);
        }

        // If we still have a non-terminal test case at this point we aborted.
        if self.testcase.is_some() {
            let result = GTestResult {
                testcase: self.testcase.clone().unwrap(),
                log: self.log.clone(),
                status: ABORTED,
            };

            self.testcase = None;

            return Some(result);
        }

        None
    }
}

#[test]
fn test_parse_one() {
    let output = r#"Note: Google Test filter = *NOPE*-
[==========] Running 3 tests from 1 test case.
[----------] Global test environment set-up.
[----------] 3 tests from NOPE
[ RUN      ] NOPE.NOPE1
[       OK ] NOPE.NOPE1 (0 ms)
[ RUN      ] NOPE.NOPE2
../3rdparty/libprocess/src/tests/future_tests.cpp:886: Failure
Value of: false
  Actual: false
Expected: true
[  FAILED  ] NOPE.NOPE2 (0 ms)
[ RUN      ] NOPE.NOPE3
WARNING: Logging before InitGoogleLogging() is written to STDERR
F0303 10:01:07.804791 2590810944 future_tests.cpp:892] Check failed: false
*** Check failure stack trace: ***
*** Aborted at 1520067667 (unix time) try "date -d @1520067667" if you are using GNU date ***
PC: @     0x7fff617c3e3e __pthread_kill
*** SIGABRT (@0x7fff617c3e3e) received by PID 8086 (TID 0x7fff9a6ca340) stack trace: ***
    @     0x7fff618f5f5a _sigtramp
    @     0x7ffee1d4c228 (unknown)
    @     0x7fff61720312 abort
    @        0x10ebe76b9 google::logging_fail()
    @        0x10ebe76aa google::LogMessage::Fail()
    @        0x10ebe67ba google::LogMessage::SendToLog()
    @        0x10ebe6dec google::LogMessage::Flush()
    @        0x10ebeafdf google::LogMessageFatal::~LogMessageFatal()
    @        0x10ebe7a49 google::LogMessageFatal::~LogMessageFatal()
    @        0x10df7db11 NOPE_NOPE3_Test::TestBody()
    @        0x10e217b24 testing::internal::HandleExceptionsInMethodIfSupported<>()
    @        0x10e217a6d testing::Test::Run()
    @        0x10e218ea0 testing::TestInfo::Run()
    @        0x10e219827 testing::TestCase::Run()
    @        0x10e223197 testing::internal::UnitTestImpl::RunAllTests()
    @        0x10e222ab4 testing::internal::HandleExceptionsInMethodIfSupported<>()
    @        0x10e222a10 testing::UnitTest::Run()
    @        0x10deb7551 main
    @     0x7fff61674115 start
    @                0x2 (unknown)"#;

    assert_eq!(
        vec!["NOPE.NOPE1", "NOPE.NOPE2", "NOPE.NOPE3"],
        Vec::from_iter(
            GTestParser::new(output.split('\n').map(String::from))
                .filter(|result| result.status == STARTING)
                .map(|result| result.testcase)
                .dedup(),
        )
    );

    assert_eq!(
        vec!["NOPE.NOPE1"],
        Vec::from_iter(
            GTestParser::new(output.split('\n').map(String::from))
                .filter(|result| result.status == OK)
                .map(|result| result.testcase),
        )
    );

    assert_eq!(
        vec!["NOPE.NOPE2"],
        Vec::from_iter(
            GTestParser::new(output.split('\n').map(String::from))
                .filter(|result| result.status == FAILED)
                .map(|result| result.testcase),
        )
    );

    let aborted = Vec::from_iter(
        GTestParser::new(output.split('\n').map(String::from))
            .filter(|result| result.status == ABORTED),
    );
    assert_eq!(1, aborted.len());
    assert_eq!(
        vec!["NOPE.NOPE3"],
        aborted
            .iter()
            .map(|result| &result.testcase)
            .collect::<Vec<_>>()
    );
    assert_eq!(
        r#"[ RUN      ] NOPE.NOPE3
WARNING: Logging before InitGoogleLogging() is written to STDERR
F0303 10:01:07.804791 2590810944 future_tests.cpp:892] Check failed: false
*** Check failure stack trace: ***
*** Aborted at 1520067667 (unix time) try "date -d @1520067667" if you are using GNU date ***
PC: @     0x7fff617c3e3e __pthread_kill
*** SIGABRT (@0x7fff617c3e3e) received by PID 8086 (TID 0x7fff9a6ca340) stack trace: ***
    @     0x7fff618f5f5a _sigtramp
    @     0x7ffee1d4c228 (unknown)
    @     0x7fff61720312 abort
    @        0x10ebe76b9 google::logging_fail()
    @        0x10ebe76aa google::LogMessage::Fail()
    @        0x10ebe67ba google::LogMessage::SendToLog()
    @        0x10ebe6dec google::LogMessage::Flush()
    @        0x10ebeafdf google::LogMessageFatal::~LogMessageFatal()
    @        0x10ebe7a49 google::LogMessageFatal::~LogMessageFatal()
    @        0x10df7db11 NOPE_NOPE3_Test::TestBody()
    @        0x10e217b24 testing::internal::HandleExceptionsInMethodIfSupported<>()
    @        0x10e217a6d testing::Test::Run()
    @        0x10e218ea0 testing::TestInfo::Run()
    @        0x10e219827 testing::TestCase::Run()
    @        0x10e223197 testing::internal::UnitTestImpl::RunAllTests()
    @        0x10e222ab4 testing::internal::HandleExceptionsInMethodIfSupported<>()
    @        0x10e222a10 testing::UnitTest::Run()
    @        0x10deb7551 main
    @     0x7fff61674115 start
    @                0x2 (unknown)"#,
        &aborted[0].log.iter().join("\n")
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

fn run(test_executable: &Path, jobs: usize) -> usize {
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
                    STARTING => {
                        progress_shard.set_message(&t.testcase.to_string());
                    }
                    OK => {
                        progress_global.inc(1);
                    }
                    FAILED | ABORTED => {
                        progress_global.inc(1);
                        progress_shard.set_message(&format!("{}", style(&t.testcase).red()));
                        thread::sleep(Duration::from_millis(500));
                        thread_tx.send(t.clone()).unwrap();
                    }
                    RUNNING => { /*Ignoring running updates for now.*/ }
                }
            }

            progress_shard.finish_and_clear();
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

    failures.len()
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

    let mut ret_vec = Vec::new();
    for exe in test_executables {
        if multiple_tests {
            println!("{}", style(format!("Running {}", exe)).bold());
        }
        ret_vec.push(run(std::path::PathBuf::from(exe).as_path(), jobs));
    }
    std::process::exit(*ret_vec.iter().max().unwrap() as i32);
}

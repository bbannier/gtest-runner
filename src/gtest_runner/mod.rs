#![cfg_attr(feature = "clippy", feature(plugin))]
#![cfg_attr(feature = "clippy", plugin(clippy))]

extern crate console;
extern crate indicatif;
extern crate itertools;
extern crate num_cpus;
extern crate regex;

use console::{strip_ansi_codes, style};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use itertools::Itertools;
use regex::Regex;

use std::collections::HashSet;
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{ChildStdout, Command, Stdio};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

#[cfg(test)]
use std::iter::FromIterator;

#[derive(Clone, Debug, PartialEq)]
enum GTestStatus {
    STARTING,
    RUNNING,
    OK,
    FAILED,
    ABORTED,
}

impl GTestStatus {
    fn is_terminal(&self) -> bool {
        match self {
            GTestStatus::STARTING | GTestStatus::RUNNING => false,
            GTestStatus::ABORTED | GTestStatus::OK | GTestStatus::FAILED => true,
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
                    GTestStatus::OK
                } else if failed.is_match(&line) {
                    GTestStatus::FAILED
                } else if starting.is_match(&line) {
                    GTestStatus::STARTING
                } else {
                    GTestStatus::RUNNING
                }
            };

            match status {
                GTestStatus::STARTING => {
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
                status: GTestStatus::ABORTED,
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
                .filter(|result| result.status == GTestStatus::STARTING)
                .map(|result| result.testcase)
                .dedup(),
        )
    );

    assert_eq!(
        vec!["NOPE.NOPE1"],
        Vec::from_iter(
            GTestParser::new(output.split('\n').map(String::from))
                .filter(|result| result.status == GTestStatus::OK)
                .map(|result| result.testcase),
        )
    );

    assert_eq!(
        vec!["NOPE.NOPE2"],
        Vec::from_iter(
            GTestParser::new(output.split('\n').map(String::from))
                .filter(|result| result.status == GTestStatus::FAILED)
                .map(|result| result.testcase),
        )
    );

    let aborted = Vec::from_iter(
        GTestParser::new(output.split('\n').map(String::from))
            .filter(|result| result.status == GTestStatus::ABORTED),
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

fn run_shard(test_executable: &Path, job_index: usize, jobs: usize) -> Result<ChildStdout, &str> {
    Command::new(&test_executable)
        .env("GTEST_SHARD_INDEX", job_index.to_string())
        .env("GTEST_TOTAL_SHARDS", jobs.to_string())
        .env("GTEST_COLOR", "YES")
        .stderr(Stdio::null())
        .stdout(Stdio::piped())
        .spawn()
        .expect("Could not launch")
        .stdout
        .ok_or_else(|| "Could not capture output")
}

fn process_shard(
    output: ChildStdout,
    sender: mpsc::Sender<GTestResult>,
    progress_shard: ProgressBar,
    progress_global: Arc<ProgressBar>,
) {
    let reader = BufReader::new(output);

    // The output is processed on a separate thread to not block the main
    // thread while we wait for output.
    thread::spawn(move || {
        let lines = reader.lines().map(|line| match line {
            Ok(line) => line,
            Err(err) => panic!(err),
        });

        for t in GTestParser::new(lines) {
            progress_shard.inc(1);

            match t.status {
                GTestStatus::STARTING => {
                    progress_shard.set_message(&t.testcase.to_string());
                }
                GTestStatus::OK => {
                    progress_global.inc(1);
                    sender.send(t.clone()).unwrap();
                }
                GTestStatus::FAILED | GTestStatus::ABORTED => {
                    progress_global.inc(1);
                    progress_shard.set_message(&format!("{}", style(&t.testcase).red()));
                    thread::sleep(Duration::from_millis(500));
                    sender.send(t.clone()).unwrap();
                }
                GTestStatus::RUNNING => { /*Ignoring running updates for now.*/ }
            }
        }

        progress_shard.finish_and_clear();
    });
}

/// Sharded execution of a gtest executable
///
/// This function takes the path to a gtest executable and number
/// of shards. It the executes the tests in a sharded way and
/// returns the number of failures.
pub fn run(test_executable: &Path, jobs: usize) -> usize {
    // Determine the number of tests.
    let pb = ProgressBar::new(100);
    pb.set_style(ProgressStyle::default_spinner().template("{msg}"));
    pb.set_message("Determining number of tests ...");
    let tests = get_tests(test_executable).unwrap();

    pb.finish_and_clear();

    // Run tests.
    let m = MultiProgress::new();

    let progress_global = Arc::new(m.add(ProgressBar::new(tests.len() as u64)));
    progress_global.set_style(
        ProgressStyle::default_spinner()
            .template("{spinner:.green} {msg} {bar} [{pos}/{len}] {elapsed_precise}"),
    );
    progress_global.set_message("Running tests ...");

    // Set up a communication channel between the worker processing test
    // output threads and the main thread.
    let (sender, receiver) = mpsc::channel::<GTestResult>();

    // Execute the shards.
    for job in 0..jobs {
        let output = run_shard(&test_executable, job, jobs).unwrap();

        let progress_shard = m.add(ProgressBar::new(100));
        progress_shard.set_style(ProgressStyle::default_spinner().template("{spinner} {wide_msg}"));

        process_shard(
            output,
            sender.clone(),
            progress_shard,
            progress_global.clone(),
        );
    }

    // Run a spinlock checking whether all processes have finished.
    //
    // TODO(bbannier): Use e.g., a condition variable instead.
    thread::spawn(move || {
        while Arc::strong_count(&progress_global) > 1 {
            thread::sleep(Duration::from_millis(10));
        }
        progress_global.finish_with_message("All done");
    });

    m.join_and_clear().unwrap();

    // Report success or failures globally.
    let (successes, failures): (Vec<GTestResult>, Vec<GTestResult>) = receiver
        .try_iter()
        .partition(|r| r.status == GTestStatus::OK);

    if failures.is_empty() {
        println!(
            "{}",
            style(format!("{} tests passed", successes.len()))
                .bold()
                .green()
        );
    } else {
        println!(
            "{}",
            failures.iter().map(|f| f.log.iter().join("\n")).join("\n")
        );
        println!(
            "{}",
            style(format!(
                "{} out of {} tests failed\n",
                failures.len(),
                tests.len()
            )).bold()
                .red()
        );
    }

    failures.len()
}

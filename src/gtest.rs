#![cfg_attr(feature = "clippy", feature(plugin))]
#![cfg_attr(feature = "clippy", plugin(clippy))]

extern crate console;
extern crate indicatif;
extern crate itertools;
extern crate num_cpus;
extern crate regex;

use console::{strip_ansi_codes, style};
use indicatif::{MultiProgress, ProgressBar, ProgressDrawTarget, ProgressStyle};
use regex::Regex;

use std::collections::HashSet;
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{ChildStdout, Command, Stdio};
use std::sync::Arc;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

#[cfg(test)]
use std::iter::FromIterator;

#[cfg(test)]
use itertools::Itertools;

#[derive(Clone, Debug, PartialEq)]
enum Status {
    STARTING,
    RUNNING,
    OK,
    FAILED,
    ABORTED,
}

impl Status {
    fn is_terminal(&self) -> bool {
        match self {
            Status::STARTING | Status::RUNNING => false,
            Status::ABORTED | Status::OK | Status::FAILED => true,
        }
    }

    fn is_failed(&self) -> bool {
        match self {
            Status::STARTING | Status::RUNNING | Status::OK => false,
            Status::FAILED | Status::ABORTED => true,
        }
    }
}

#[derive(Debug, Clone)]
struct TestResult {
    pub testcase: String,
    pub log: Vec<String>,
    pub status: Status,
}

struct Parser<T: Iterator> {
    testcase: Option<String>,
    log: Vec<String>,
    reader: T,
}

impl<T> Parser<T>
where
    T: Iterator<Item = String>,
{
    fn new(reader: T) -> Parser<T> {
        Parser {
            testcase: None,
            log: vec![],
            reader,
        }
    }
}

impl<T> Iterator for Parser<T>
where
    T: Iterator<Item = String>,
{
    type Item = TestResult;

    fn next(&mut self) -> Option<TestResult> {
        let starting = Regex::new(r"^\[ RUN      \] .*").unwrap();
        let ok = Regex::new(r"^\[       OK \] .* \(\d* .*\)").unwrap();
        let failed = Regex::new(r"^\[  FAILED  \] .* \(\d* .*\)").unwrap();

        if let Some(line) = self.reader.next() {
            let status = {
                let line = strip_ansi_codes(&line);

                if ok.is_match(&line) {
                    Status::OK
                } else if failed.is_match(&line) {
                    Status::FAILED
                } else if starting.is_match(&line) {
                    Status::STARTING
                } else {
                    Status::RUNNING
                }
            };

            match status {
                Status::STARTING => {
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

            let result = TestResult {
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
            let result = TestResult {
                testcase: self.testcase.clone().unwrap(),
                log: self.log.clone(),
                status: Status::ABORTED,
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
            Parser::new(output.split('\n').map(String::from))
                .filter(|result| result.status == Status::STARTING)
                .map(|result| result.testcase)
                .dedup(),
        )
    );

    assert_eq!(
        vec!["NOPE.NOPE1"],
        Vec::from_iter(
            Parser::new(output.split('\n').map(String::from))
                .filter(|result| result.status == Status::OK)
                .map(|result| result.testcase),
        )
    );

    assert_eq!(
        vec!["NOPE.NOPE2"],
        Vec::from_iter(
            Parser::new(output.split('\n').map(String::from))
                .filter(|result| result.status == Status::FAILED)
                .map(|result| result.testcase),
        )
    );

    let aborted = Vec::from_iter(
        Parser::new(output.split('\n').map(String::from))
            .filter(|result| result.status == Status::ABORTED),
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
    sender: mpsc::Sender<TestResult>,
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

        for t in Parser::new(lines) {
            progress_shard.inc(1);

            match t.status {
                Status::STARTING => {
                    progress_shard.set_message(&t.testcase.to_string());
                }
                Status::OK => {
                    progress_global.inc(1);
                    sender.send(t.clone()).unwrap();
                }
                Status::FAILED | Status::ABORTED => {
                    progress_global.inc(1);
                    progress_shard.set_message(&format!("{}", style(&t.testcase).red()));
                    thread::sleep(Duration::from_millis(500));
                    sender.send(t.clone()).unwrap();
                }
                Status::RUNNING => { /*Ignoring running updates for now.*/ }
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
pub fn run(test_executable: &Path, jobs: usize, verbosity: usize, progress: bool) -> usize {
    let mut num_tests = 0;

    if progress {
        // Determine the number of tests.
        let pb = ProgressBar::new(100);
        pb.set_style(ProgressStyle::default_spinner().template("{msg}"));
        pb.set_message("Determining number of tests ...");
        num_tests = get_tests(test_executable).unwrap().len();

        pb.finish_and_clear();
    }

    // Run tests.
    let m = MultiProgress::new();
    if verbosity < 1 || verbosity > 2 {
        m.set_draw_target(ProgressDrawTarget::hidden());
    }

    let progress_global = Arc::new(m.add(ProgressBar::new(num_tests as u64)));
    if progress {
        progress_global.set_style(
            ProgressStyle::default_spinner()
                .template("{spinner:.green} {msg} {bar} [{pos}/{len}] {elapsed_precise}"),
        );
    } else {
        progress_global.set_style(
            ProgressStyle::default_spinner()
                .template("{spinner:.green} {msg} {bar} [{pos}/?] {elapsed_precise}"),
        );
    }
    progress_global.set_message("Running tests ...");

    // Set up a communication channel between the worker processing test
    // output threads and the main thread.
    let (sender, receiver) = mpsc::channel::<TestResult>();

    // Execute the shards.
    for job in 0..jobs {
        let output = run_shard(&test_executable, job, jobs).unwrap();

        let progress_shard = match verbosity != 2 {
            true => ProgressBar::hidden(),
            false => m.add(ProgressBar::new(100)),
        };
        progress_shard.set_style(ProgressStyle::default_spinner().template("{spinner} {wide_msg}"));

        process_shard(
            output,
            sender.clone(),
            progress_shard,
            progress_global.clone(),
        );
    }

    // Close the sender in this thread.
    drop(sender);

    //////////////////////////////////////////

    // Report successes or failures globally.
    let reporter = thread::spawn(move || {
        // We wait to be unparked from the outside at the right time.
        thread::park();

        let mut num_failures = 0;
        for result in receiver.iter() {
            if !result.status.is_terminal() {
                continue;
            }

            if result.status.is_failed() {
                num_failures += 1;
            }

            if (result.status.is_failed() && verbosity > 0) || verbosity > 2 {
                for line in result.log.iter() {
                    println!("{}", line);
                }
            }
        }

        num_failures
    });

    // Start the reporter immediately if we log all output.
    if verbosity > 2 {
        reporter.thread().unpark();
    }

    // Run a spinlock checking whether all processes have finished
    // and finishing the global progress.
    let _waiter = thread::spawn(move || {
        while Arc::strong_count(&progress_global) > 1 {
            thread::sleep(Duration::from_millis(10));
        }
        progress_global.finish();
    });

    // This implicitly joins the waiter thread.
    m.join_and_clear().unwrap();

    // If we log only failures wait until all shards have finished processing.
    if verbosity < 3 {
        reporter.thread().unpark();
    }

    let num_failures = reporter.join().unwrap();

    if num_failures == 0 {
        if verbosity > 0 {
            let message = match progress {
                true => format!("{} tests passed", num_tests),
                false => format!("All tests passed"),
            };

            println!("{}", style(message).bold().green());
        }
    } else {
        let message = match progress {
            true => format!("{} out of {} tests failed\n", num_failures, num_tests),
            false => format!("{} tests failed\n", num_failures),
        };
        println!("{}", style(message).bold().red());
    }

    num_failures
}

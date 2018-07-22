extern crate console;
extern crate itertools;
extern crate regex;

use console::strip_ansi_codes;

#[cfg(test)]
use std::iter::FromIterator;

#[cfg(test)]
use itertools::Itertools;

use gtest::{Status, TestResult};

pub struct Parser<T: Iterator> {
    testcase: Option<String>,
    log: Vec<String>,
    reader: T,
}

impl<T> Parser<T>
where
    T: Iterator<Item = String>,
{
    pub fn new(reader: T) -> Parser<T> {
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
        let starting = regex::Regex::new(r"^\[ RUN      \] .*").unwrap();
        let ok = regex::Regex::new(r"^\[       OK \] .* \(\d* .*\)").unwrap();
        let failed = regex::Regex::new(r"^\[  FAILED  \] .* \(\d* .*\)").unwrap();

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

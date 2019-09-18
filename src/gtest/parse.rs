use {
    crate::gtest::{Event, Status, Test},
    console::strip_ansi_codes,
};

#[cfg(test)]
use {
    itertools::{join, Itertools},
    std::iter::FromIterator,
};

pub struct Parser<T> {
    testcase: Option<String>,
    log: Vec<String>,
    reader: T,

    starting: regex::Regex,
    ok: regex::Regex,
    failed: regex::Regex,
}

impl<T> Parser<T> {
    fn parse(&mut self, line: String) -> Result<Option<Test>, String> {
        let line = strip_ansi_codes(&line).to_string();

        if self.testcase.is_some() {
            self.log.push(line.clone());
        }

        let mut result = None;

        if self.ok.is_match(&line) {
            result = Some(Test {
                testcase: self.testcase.clone().unwrap(),
                shard: None,
                event: Event::Terminal {
                    status: Status::OK,
                    log: self.log.clone(),
                },
            });

            self.testcase = None;
            self.log.clear();
        } else if self.failed.is_match(&line) {
            result = Some(Test {
                testcase: self.testcase.clone().unwrap(),
                shard: None,
                event: Event::Terminal {
                    status: Status::FAILED,
                    log: self.log.clone(),
                },
            });

            self.testcase = None;
            self.log.clear();
        } else if self.starting.is_match(&line) {
            self.testcase = Some(String::from(
                strip_ansi_codes(&line).to_string()[12..]
                    .split_whitespace()
                    .next()
                    .ok_or_else(|| {
                        format!("Expected at least a single space in line: {}", &line)
                    })?,
            ));

            result = Some(Test {
                testcase: self.testcase.clone().unwrap(),
                shard: None,
                event: Event::Starting,
            });

            self.log = vec![line];
        } else if let Some(testcase) = &self.testcase {
            result = Some(Test {
                testcase: testcase.clone(),
                shard: None,
                event: Event::Running,
            });
        };

        Ok(result)
    }

    fn finalize(&mut self) -> Option<Test> {
        // If we still have a non-terminal test case at this point we aborted.
        if let Some(testcase) = &self.testcase {
            let result = Test {
                testcase: testcase.clone(),
                shard: None,
                event: Event::Terminal {
                    status: Status::ABORTED,
                    log: self.log.clone(),
                },
            };

            self.testcase = None;

            return Some(result);
        }

        None
    }
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

            starting: regex::Regex::new(r"^\[ RUN      \] .*").unwrap(),
            ok: regex::Regex::new(r"^\[       OK \] .* \(\d* .*\)").unwrap(),
            failed: regex::Regex::new(r"^\[  FAILED  \] .* \(\d* .*\)").unwrap(),
        }
    }
}

impl<T> Iterator for Parser<T>
where
    T: Iterator<Item = String>,
{
    type Item = Test;

    fn next(&mut self) -> Option<Test> {
        match self.reader.next() {
            Some(line) => self.parse(line).ok()?.or_else(|| self.next()),
            None => self.finalize(),
        }
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
                .filter(|result| match result.event {
                    Event::Starting => true,
                    _ => false,
                })
                .map(|result| result.testcase)
                .dedup(),
        )
    );

    assert_eq!(
        vec!["NOPE.NOPE1"],
        Vec::from_iter(
            Parser::new(output.split('\n').map(String::from))
                .filter(|result| match &result.event {
                    Event::Terminal { status, log: _ } => *status == Status::OK,
                    _ => false,
                })
                .map(|result| result.testcase),
        )
    );

    assert_eq!(
        vec!["NOPE.NOPE2"],
        Vec::from_iter(
            Parser::new(output.split('\n').map(String::from))
                .filter(|result| match &result.event {
                    Event::Terminal { status, log: _ } => *status == Status::FAILED,
                    _ => false,
                })
                .map(|result| result.testcase),
        )
    );

    let aborted = Vec::from_iter(Parser::new(output.split('\n').map(String::from)).filter(
        |result| match &result.event {
            Event::Terminal { status, log: _ } => *status == Status::ABORTED,
            _ => false,
        },
    ));
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
        aborted
            .iter()
            .map(|result| match &result.event {
                Event::Terminal { status: _, log } => join(log, "\n"),
                _ => unreachable!(),
            })
            .next()
            .unwrap()
    );
}

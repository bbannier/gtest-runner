use crossbeam::Sender;
use rs_tracing::{trace_begin, trace_duration_internal, trace_end};
use std::collections::HashSet;
use std::convert::Into;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::thread;

use super::parse;
use super::Status;
use super::TestResult;

pub fn get_tests<P: Into<PathBuf>>(
    test_executable: P,
    include_disabled_tests: bool,
) -> Result<HashSet<String>, String> {
    let result = Command::new(test_executable.into())
        .args(&["--gtest_list_tests"])
        .output()
        .expect("Failed to execute process");

    if !result.status.success() {
        return Err("Failed to run program".to_owned());
    }

    let output = String::from_utf8_lossy(&result.stdout);

    let mut tests = HashSet::new();

    let mut current_test: Option<&str> = None;
    for line in output.lines() {
        if !line.starts_with(' ') {
            current_test = line.split_whitespace().next();
        } else {
            let case = &line
                .split_whitespace()
                .next()
                .ok_or_else(|| format!("Expected test case on line: {}", &line))?;

            let test = match current_test {
                Some(t) => [t, case].concat(),
                None => panic!("Couldn't determine test name"),
            };

            if !include_disabled_tests && test.contains("DISABLED_") {
                continue;
            }

            tests.insert(test);
        }
    }

    Ok(tests)
}

#[test]
fn test_get_tests() {
    let tests = get_tests("target/debug/dummy-gtest-executable", false).unwrap();
    assert_eq!(2, tests.len());
}

pub fn cmd<P: Into<PathBuf>>(test_executable: P, job_index: usize, jobs: usize) -> Command {
    let mut child = Command::new(&test_executable.into());

    child.env("GTEST_SHARD_INDEX", job_index.to_string());
    child.env("GTEST_TOTAL_SHARDS", jobs.to_string());
    child.env("GTEST_COLOR", "YES");
    child.stderr(Stdio::null());
    child.stdout(Stdio::piped());

    child
}

pub fn process_shard(
    shard: usize,
    child: Child,
    sender: Sender<TestResult>,
    done: Sender<()>,
) -> Result<(thread::JoinHandle<()>), &'static str> {
    // TODO(bbannier): Process stdout as well.
    let reader = BufReader::new(child.stdout.ok_or("Child process has not stdout")?);

    // The output is processed on a separate thread to not block the main
    // thread while we wait for output.
    Ok(thread::spawn(move || {
        let lines = reader.lines().map(|line| match line {
            Ok(line) => line,
            Err(err) => panic!(err),
        });

        for t in parse::Parser::new(lines) {
            let mut t = t;
            t.shard = Some(shard);

            sender.send(t.clone()).unwrap();

            // Update tracing.
            match t.status {
                Status::STARTING => {
                    trace_begin!(&t.testcase);
                }
                Status::OK | Status::FAILED | Status::ABORTED => {
                    trace_end!(&t.testcase);
                }
                Status::RUNNING => {}
            }
        }

        // Signal that we are done processing this shard.
        done.send(()).unwrap();
    }))
}

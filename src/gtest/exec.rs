use console::style;
use indicatif::ProgressBar;
use std::collections::HashSet;
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{ChildStdout, Command, Stdio};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use super::parse;
use super::Status;
use super::TestResult;

pub fn get_tests(test_executable: &Path) -> Result<HashSet<String>, &str> {
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

pub fn run_shard(
    test_executable: &Path,
    job_index: usize,
    jobs: usize,
) -> Result<ChildStdout, &str> {
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

pub fn process_shard(
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

        for t in parse::Parser::new(lines) {
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

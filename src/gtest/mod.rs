#![cfg_attr(feature = "clippy", feature(plugin))]
#![cfg_attr(feature = "clippy", plugin(clippy))]

use {
    anyhow::Result,
    console::style,
    crossbeam::channel,
    indicatif::{MultiProgress, ProgressBar, ProgressDrawTarget, ProgressStyle},
    rs_tracing::{trace_scoped, trace_scoped_internal},
    std::{cmp::min, env, fs::canonicalize, path::PathBuf, sync::Arc, thread},
};

#[cfg(test)]
use std::path::Path;
use std::time::Duration;

mod exec;
mod parse;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    Ok,
    Failed,
    Aborted,
}

impl Status {
    pub fn is_failed(&self) -> bool {
        match self {
            Status::Failed | Status::Aborted => true,
            Status::Ok => false,
        }
    }
}

#[derive(Debug, Clone)]
pub enum Event {
    Starting,
    Running,
    Terminal { status: Status, log: Vec<String> },
}

#[derive(Debug, Clone)]
pub struct Test {
    event: Event,
    testcase: String,
    shard: Option<usize>,
}

struct ShardStats {
    num_passed: usize,
    failed_tests: Vec<Test>,
}

impl ShardStats {
    fn num_failed(&self) -> usize {
        self.failed_tests.len()
    }
}

/// Sharded execution of a gtest executable
///
/// This function takes the path to a gtest executable and number
/// of shards. It then executes the tests in a sharded way and
/// returns the number of failures.
pub fn run<P: Into<PathBuf>>(
    test_executable: P,
    gtest_filter: Option<String>,
    jobs: usize,
    verbosity: u64,
    repeat: u64,
) -> Result<usize> {
    // We normalize the test executable path to decouple us from `Command::new` lookup semantics
    // and get the same results for when given `test-exe`, `./test-exe`, or `/path/to/test-exe`.
    let test_executable = canonicalize(test_executable.into())?;

    if let Some(filter) = gtest_filter {
        env::set_var("GTEST_FILTER", filter);
    }

    // If we show some sort of progress bar determine the total number of tests before running shards.
    let num_tests = {
        trace_scoped!("Determine number of tests");

        let run_disabled_tests = match env::var("GTEST_ALSO_RUN_DISABLED_TESTS") {
            Ok(val) => match val.parse::<i32>() {
                Ok(b) => b > 0,
                Err(_) => false,
            },
            Err(_) => false,
        };

        let pb = ProgressBar::new(100);

        if verbosity < 1 {
            pb.set_draw_target(ProgressDrawTarget::hidden());
        }

        pb.set_style(ProgressStyle::default_spinner().template("{msg}")?);
        pb.set_message("Determining number of tests ...");
        let num = exec::get_tests(&test_executable, run_disabled_tests)?.len();
        pb.finish_and_clear();

        num
    };

    // Do not execute more jobs than tests.
    let jobs = min(jobs, num_tests);

    // Run tests.
    let m = MultiProgress::new();
    if !(1..=2).contains(&verbosity) {
        m.set_draw_target(ProgressDrawTarget::hidden());
    }

    let progress_global = Arc::new(m.add(ProgressBar::new(num_tests as u64)));
    progress_global.set_style(
        ProgressStyle::default_spinner()
            .template("{spinner:.green} {msg} {bar} [{pos}/{len}] {elapsed_precise}")?,
    );
    progress_global.set_message("Running tests ...");

    // Make sure the elapsed time is updated even if no updates arrive from shards.
    progress_global.enable_steady_tick(Duration::from_millis(100));

    // Set up a communication channel between the worker processing test
    // output threads and the main thread.
    let (sender, receiver) = channel::unbounded();
    let mut done_receivers = vec![];

    let mut progress_shards = vec![];

    // Execute the shards.
    for job in 0..jobs {
        let (done_sender, done_receiver) = channel::unbounded();
        done_receivers.push(done_receiver);

        let cmd = exec::cmd(&test_executable, job, jobs).spawn()?;

        let progress_shard = if verbosity == 2 {
            m.add(ProgressBar::new(100))
        } else {
            ProgressBar::hidden()
        };
        progress_shard
            .set_style(ProgressStyle::default_spinner().template("{spinner} {wide_msg}")?);

        progress_shards.push(progress_shard);

        exec::process_shard(job, cmd, sender.clone(), done_sender)?;
    }

    // Close the sender in this thread.
    drop(sender);

    //////////////////////////////////////////

    // Report successes or failures globally.
    let reporter = thread::spawn(move || {
        let mut stats = ShardStats {
            num_passed: 0,
            failed_tests: vec![],
        };

        let mut sel = channel::Select::new();
        for done in &done_receivers {
            sel.recv(done);
        }

        for result in receiver.iter() {
            let shard = result.shard.unwrap();
            let progress_shard = &progress_shards[shard as usize];

            progress_shard.inc(1);

            if let Event::Terminal { log, .. } = &result.event {
                if verbosity > 2 {
                    for line in log {
                        println!("{}", line);
                    }
                }
            }

            match &result.event {
                Event::Starting => {
                    progress_shard.set_message(result.testcase);
                }
                Event::Running => {}
                Event::Terminal { status, .. } => {
                    progress_global.inc(1);

                    if status.is_failed() {
                        progress_shard.set_message(format!("{}", style(&result.testcase).red()));

                        stats.failed_tests.push(result.clone());
                    } else {
                        stats.num_passed += 1;
                    }
                }
            };

            // Check if any shards can be cleaned up.
            if let Ok(index) = sel.try_ready() {
                sel.remove(index);
                let progress_shard = &progress_shards[index];
                progress_shard.finish_and_clear();
            }
        }

        progress_global.finish_and_clear();

        stats
    });

    // This implicitly joins the waiter thread.
    m.clear()?;

    // If we log only failures wait until all shards have finished processing.
    if verbosity < 3 {
        reporter.thread().unpark();
    }

    let stats = reporter.join().unwrap();

    if stats.failed_tests.is_empty() {
        if verbosity > 0 {
            let message = format!("{} tests passed", stats.num_passed);
            println!("{}", style(message).bold().green());
        }
    } else {
        if verbosity <= 2 {
            for test in &stats.failed_tests {
                if let Event::Terminal { status, log } = &test.event {
                    if status.is_failed() {
                        for line in log {
                            println!("{}", line);
                        }
                    }
                }
            }
        }
        let message = format!(
            "{} out of {} tests failed",
            stats.num_failed(),
            stats.num_passed + stats.num_failed()
        );
        println!("{}", style(message).bold().red());
    }

    if repeat != 0 && !stats.failed_tests.is_empty() {
        let filter = stats
            .failed_tests
            .iter()
            .fold("".to_string(), |acc, t| acc + ":" + &t.testcase);

        return run(test_executable, Some(filter), jobs, verbosity, repeat - 1);
    }

    // Check that the number of reported tests is consistent with the number of expected tests.
    // This mostly serves to validate that we did not accidentally drop test results.
    let num_tests_reported = stats.num_failed() + stats.num_passed;
    if num_tests != num_tests_reported {
        eprintln!(
            "Expected {} tests but only saw results from {}",
            num_tests, num_tests_reported,
        );

        return Ok(1);
    }

    Ok(stats.num_failed())
}

#[cfg(test)]
pub fn test_executable() -> PathBuf {
    Path::new(env!("OUT_DIR")).join("dummy-gtest-executable")
}

#[test]
fn test_run1() {
    assert_eq!(0, run(test_executable(), None, 1, 0, 0).unwrap());
}

#[test]
fn test_run2() {
    assert_eq!(0, run(test_executable(), None, 2, 0, 0).unwrap());
}

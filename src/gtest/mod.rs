#![cfg_attr(feature = "clippy", feature(plugin))]
#![cfg_attr(feature = "clippy", plugin(clippy))]

use console::style;
use indicatif::{MultiProgress, ProgressBar, ProgressDrawTarget, ProgressStyle};
use std::cmp::min;
use std::env;
use std::fs::canonicalize;
use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

mod exec;
mod parse;

#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    STARTING,
    RUNNING,
    OK,
    FAILED,
    ABORTED,
}

impl Status {
    pub fn is_terminal(&self) -> bool {
        match self {
            Status::STARTING | Status::RUNNING => false,
            Status::ABORTED | Status::OK | Status::FAILED => true,
        }
    }

    pub fn is_failed(&self) -> bool {
        match self {
            Status::STARTING | Status::RUNNING | Status::OK => false,
            Status::FAILED | Status::ABORTED => true,
        }
    }
}

#[derive(Debug, Clone)]
pub struct TestResult {
    pub testcase: String,
    pub log: Vec<String>,
    pub status: Status,
}

/// Sharded execution of a gtest executable
///
/// This function takes the path to a gtest executable and number
/// of shards. It then executes the tests in a sharded way and
/// returns the number of failures.
pub fn run<P: Into<PathBuf>>(
    test_executable: P,
    jobs: usize,
    verbosity: usize,
) -> Result<usize, String> {
    // We normalize the test executable path to decouple us from `Command::new` lookup semantics
    // and get the same results for when given `test-exe`, `./test-exe`, or `/path/to/test-exe`.
    let test_executable = canonicalize(test_executable.into()).map_err(|e| e.to_string())?;

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
        pb.set_style(ProgressStyle::default_spinner().template("{msg}"));
        pb.set_message("Determining number of tests ...");
        let num = exec::get_tests(&test_executable, run_disabled_tests)?.len();
        pb.finish_and_clear();

        num
    };

    // Do not execute more jobs than tests.
    let jobs = min(jobs, num_tests);

    // Run tests.
    let m = MultiProgress::new();
    if verbosity < 1 || verbosity > 2 {
        m.set_draw_target(ProgressDrawTarget::hidden());
    }

    let progress_global = Arc::new(m.add(ProgressBar::new(num_tests as u64)));
    progress_global.set_style(
        ProgressStyle::default_spinner()
            .template("{spinner:.green} {msg} {bar} [{pos}/{len}] {elapsed_precise}"),
    );
    progress_global.set_message("Running tests ...");

    // Make sure the elapsed time is updated even if no updates arrive from shards.
    progress_global.enable_steady_tick(100 /*ms*/);

    // Set up a communication channel between the worker processing test
    // output threads and the main thread.
    let (sender, receiver) = mpsc::channel::<TestResult>();

    // Execute the shards.
    for job in 0..jobs {
        let cmd = exec::cmd(&test_executable, job, jobs)
            .spawn()
            .map_err(|e| e.to_string())?;

        let progress_shard = if verbosity != 2 {
            ProgressBar::hidden()
        } else {
            m.add(ProgressBar::new(100))
        };
        progress_shard.set_style(ProgressStyle::default_spinner().template("{spinner} {wide_msg}"));

        exec::process_shard(cmd, sender.clone(), progress_shard, progress_global.clone())?;
    }

    // Close the sender in this thread.
    drop(sender);

    //////////////////////////////////////////

    struct ShardStats {
        num_passed: usize,
        num_failed: usize,
    }

    // Report successes or failures globally.
    let reporter = thread::spawn(move || {
        // We wait to be unparked from the outside at the right time.
        thread::park();

        let mut stats = ShardStats {
            num_passed: 0,
            num_failed: 0,
        };

        for result in receiver.iter() {
            if !result.status.is_terminal() {
                continue;
            }

            if result.status.is_failed() {
                stats.num_failed += 1;
            } else {
                stats.num_passed += 1;
            }

            if result.status.is_failed() || verbosity > 2 {
                for line in &result.log {
                    println!("{}", line);
                }
            }
        }

        stats
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
    m.join_and_clear().map_err(|e| e.to_string())?;

    // If we log only failures wait until all shards have finished processing.
    if verbosity < 3 {
        reporter.thread().unpark();
    }

    let stats = reporter.join().unwrap();

    if stats.num_failed == 0 {
        if verbosity > 0 {
            let message = format!("{} tests passed", stats.num_passed);
            println!("{}", style(message).bold().green());
        }
    } else {
        let message = format!(
            "{} out of {} tests failed",
            stats.num_failed,
            stats.num_passed + stats.num_failed
        );
        println!("{}", style(message).bold().red());
    }

    // Check that the number of reported tests is consistent with the number of expected tests.
    // This mostly serves to validate that we did not accidentally drop test results.
    let num_tests_reported = stats.num_failed + stats.num_passed;
    if num_tests != num_tests_reported {
        eprintln!(
            "Expected {} tests but only saw results from {}",
            num_tests, num_tests_reported,
        );

        return Ok(1);
    }

    Ok(stats.num_failed)
}

#![cfg_attr(feature = "clippy", feature(plugin))]
#![cfg_attr(feature = "clippy", plugin(clippy))]

extern crate console;
extern crate indicatif;

use self::console::style;
use self::indicatif::{MultiProgress, ProgressBar, ProgressDrawTarget, ProgressStyle};
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
    progress: bool,
) -> Result<usize, String> {
    // We normalize the test executable path to decouple us from `Command::new` lookup semantics
    // and get the same results for when given `test-exe`, `./test-exe`, or `/path/to/test-exe`.
    let test_executable = canonicalize(test_executable.into()).map_err(|e| e.to_string())?;

    let num_tests = if progress {
        // Determine the number of tests.
        let pb = ProgressBar::new(100);
        pb.set_style(ProgressStyle::default_spinner().template("{msg}"));
        pb.set_message("Determining number of tests ...");
        let num = Some(exec::get_tests(&test_executable)?.len());
        pb.finish_and_clear();

        num
    } else {
        None
    };

    // Run tests.
    let m = MultiProgress::new();
    if verbosity < 1 || verbosity > 2 {
        m.set_draw_target(ProgressDrawTarget::hidden());
    }

    let progress_global = Arc::new(m.add(ProgressBar::new(num_tests.unwrap_or_else(|| 0) as u64)));
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

    // Make sure the elapsed time is updated even if no updates arrive from shards.
    progress_global.enable_steady_tick(100 /*ms*/);

    // Set up a communication channel between the worker processing test
    // output threads and the main thread.
    let (sender, receiver) = mpsc::channel::<TestResult>();

    // Execute the shards.
    for job in 0..jobs {
        let output = exec::cmd(&test_executable, job, jobs)
            .spawn()
            .map_err(|e| e.to_string())?;

        let progress_shard = if verbosity != 2 {
            ProgressBar::hidden()
        } else {
            m.add(ProgressBar::new(100))
        };
        progress_shard.set_style(ProgressStyle::default_spinner().template("{spinner} {wide_msg}"));

        exec::process_shard(
            output,
            sender.clone(),
            progress_shard,
            progress_global.clone(),
        )?;
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
                for line in &result.log {
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
    m.join_and_clear().map_err(|e| e.to_string())?;

    // If we log only failures wait until all shards have finished processing.
    if verbosity < 3 {
        reporter.thread().unpark();
    }

    let num_failures = reporter.join().unwrap();

    if num_failures == 0 {
        if verbosity > 0 {
            let message = if num_tests.is_some() {
                format!("{} tests passed", num_tests.unwrap())
            } else {
                "All tests passed".to_string()
            };

            println!("{}", style(message).bold().green());
        }
    } else {
        let message = if num_tests.is_some() {
            format!(
                "{} out of {} tests failed\n",
                num_failures,
                num_tests.unwrap()
            )
        } else {
            format!("{} tests failed\n", num_failures)
        };
        println!("{}", style(message).bold().red());
    }

    Ok(num_failures)
}

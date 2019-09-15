#![cfg_attr(feature = "clippy", feature(plugin))]
#![cfg_attr(feature = "clippy", plugin(clippy))]

use {
    console::style,
    crossbeam::channel,
    indicatif::{MultiProgress, ProgressBar, ProgressDrawTarget, ProgressStyle},
    rs_tracing::{trace_scoped, trace_scoped_internal},
    std::{cmp::min, env, fs::canonicalize, path::PathBuf, sync::Arc, thread},
};

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
    pub shard: Option<usize>,
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
) -> Result<usize, String> {
    // We normalize the test executable path to decouple us from `Command::new` lookup semantics
    // and get the same results for when given `test-exe`, `./test-exe`, or `/path/to/test-exe`.
    let test_executable = canonicalize(test_executable.into()).map_err(|e| e.to_string())?;

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
    let (sender, receiver) = channel::unbounded();
    let mut dones = vec![];

    let mut progress_shards = vec![];

    // Execute the shards.
    for job in 0..jobs {
        let (done1, done2) = channel::unbounded();
        dones.push(done2);

        let cmd = exec::cmd(&test_executable, job, jobs)
            .spawn()
            .map_err(|e| e.to_string())?;

        let progress_shard = if verbosity != 2 {
            ProgressBar::hidden()
        } else {
            m.add(ProgressBar::new(100))
        };
        progress_shard.set_style(ProgressStyle::default_spinner().template("{spinner} {wide_msg}"));

        progress_shards.push(progress_shard);

        exec::process_shard(job, cmd, sender.clone(), done1)?;
    }

    // Close the sender in this thread.
    drop(sender);

    //////////////////////////////////////////

    struct ShardStats {
        num_passed: usize,
        failed_tests: Vec<TestResult>,
    }

    impl ShardStats {
        fn num_failed(&self) -> usize {
            self.failed_tests.len()
        }
    }

    // Report successes or failures globally.
    let reporter = thread::spawn(move || {
        let mut stats = ShardStats {
            num_passed: 0,
            failed_tests: vec![],
        };

        let mut sel = channel::Select::new();
        for done in &dones {
            sel.recv(done);
        }

        for result in receiver.iter() {
            let shard = result.shard.unwrap();
            let progress_shard = &progress_shards[shard as usize];

            progress_shard.inc(1);

            if result.status.is_terminal() && verbosity > 2 {
                for line in &result.log {
                    println!("{}", line);
                }
            }

            match result.status {
                Status::STARTING => {
                    progress_shard.set_message(&result.testcase);
                }
                Status::OK => {
                    progress_global.inc(1);
                }
                Status::FAILED | Status::ABORTED => {
                    progress_shard.set_message(&format!("{}", style(&result.testcase).red()));
                    progress_global.inc(1);

                    stats.failed_tests.push(result.clone());
                }
                Status::RUNNING => {}
            }

            // Update statistics.
            if result.status.is_terminal() && !result.status.is_failed() {
                stats.num_passed += 1;
            }

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
    m.join_and_clear().map_err(|e| e.to_string())?;

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
                for line in &test.log {
                    println!("{}", line);
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

#[test]
fn test_run1() {
    assert_eq!(
        0,
        run("target/debug/dummy-gtest-executable", None, 1, 0, 0).unwrap()
    );
}

#[test]
fn test_run2() {
    assert_eq!(
        0,
        run("target/debug/dummy-gtest-executable", None, 2, 0, 0).unwrap()
    );
}

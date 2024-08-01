use rs_tracing::trace_begin;
use {
    crate::{opt::Opt, parse, Event, Test},
    anyhow::{anyhow, Result},
    console::style,
    core::str,
    crossbeam::channel::Sender,
    rs_tracing::{
        close_trace_file, close_trace_file_internal, open_trace_file, trace_duration_internal,
        trace_end, trace_scoped, trace_scoped_internal, trace_to_file_internal,
    },
    std::{
        collections::HashSet,
        convert::Into,
        env,
        io::{BufRead, BufReader},
        path::PathBuf,
        process::{Child, Command, Stdio},
        thread,
    },
};

pub fn get_tests<P: Into<PathBuf>>(
    test_executable: P,
    include_disabled_tests: bool,
) -> Result<HashSet<String>> {
    let result = Command::new(test_executable.into())
        .env("GTEST_LIST_TESTS", "1")
        .output()
        .expect("Failed to execute process");

    if !result.status.success() {
        return Err(anyhow!("Failed to run program"));
    }

    let output = String::from_utf8_lossy(&result.stdout);

    let mut tests = HashSet::new();

    let mut current_test: Option<&str> = None;
    for line in output.lines() {
        if line.starts_with(' ') {
            let case = &line
                .split_whitespace()
                .next()
                .ok_or_else(|| anyhow!("Expected test case on line: {}", &line))?;

            let test = match current_test {
                Some(t) => [t, case].concat(),
                None => panic!("Couldn't determine test name"),
            };

            if !include_disabled_tests && test.contains("DISABLED_") {
                continue;
            }

            tests.insert(test);
        } else {
            current_test = line.split_whitespace().next();
        }
    }

    Ok(tests)
}

pub fn cmd<P: Into<PathBuf>>(test_executable: P, job_index: usize, jobs: usize) -> Command {
    let mut child = Command::new(test_executable.into());

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
    sender: Sender<Test>,
    done: Sender<()>,
) -> Result<thread::JoinHandle<()>> {
    // TODO(bbannier): Process stdout as well.
    let reader = BufReader::new(
        child
            .stdout
            .ok_or_else(|| anyhow!("Child process has not stdout"))?,
    );

    // The output is processed on a separate thread to not block the main
    // thread while we wait for output.
    Ok(thread::spawn(move || {
        let lines = reader.lines().map(|line| match line {
            Ok(line) => line,
            Err(err) => panic!("{}", err),
        });

        for t in parse::Parser::new(lines) {
            let mut t = t;
            t.shard = Some(shard);

            // Update tracing.
            match &t.event {
                Event::Starting => {
                    trace_begin!(&t.testcase);
                }
                Event::Running => {}
                Event::Terminal { .. } => {
                    trace_end!(&t.testcase);
                }
            };

            sender.send(t).unwrap();
        }

        // Signal that we are done processing this shard.
        done.send(()).unwrap();
    }))
}

pub fn exec(opt: &Opt) -> Result<i32> {
    let ret = if let Some(test_executables) = &opt.mode.test_executables {
        if opt.trace {
            open_trace_file!(".").unwrap();
        }

        let available_parallelism = std::thread::available_parallelism()?.into();

        let mut ret_vec = Vec::new();
        for exe in test_executables {
            if test_executables.len() > 1 && opt.verbosity > 0 {
                println!("{}", style(format!("Running {}", exe)).bold());
            }
            trace_scoped!(exe);
            ret_vec.push(crate::run(
                exe,
                None,
                opt.jobs.unwrap_or(available_parallelism),
                opt.verbosity,
                opt.repeat,
            )?);
        }

        close_trace_file!();

        i32::try_from(ret_vec.iter().sum::<usize>()).map_err(|e| anyhow!(e.to_string()))
    } else {
        Ok(0)
    };

    if let Some(true) = opt.mode.sample_data {
        sample_data();
        return Ok(0);
    }

    ret
}

fn sample_data() {
    fn parse_arg(args: &[String], flag: &str, env: &str) -> Option<String> {
        args.iter()
            .find_map(|a| {
                if a.starts_with(&format!("--{}", flag)) {
                    a.split('=').next().or(Some("")).map(|x| x.to_string())
                } else {
                    None
                }
            })
            .or_else(|| env::var(env).ok())
    }

    let args: Vec<_> = env::args().collect();
    let gtest_shard_index = parse_arg(&args, "gtest_shard_index", "GTEST_SHARD_INDEX")
        .and_then(|x| x.parse::<usize>().ok())
        .unwrap_or(0);
    let gtest_total_shards = parse_arg(&args, "gtest_total_shards", "GTEST_TOTAL_SHARDS")
        .and_then(|x| x.parse::<usize>().ok())
        .unwrap_or(1);
    let gtest_list_tests = parse_arg(&args, "gtest_list_tests", "GTEST_LIST_TESTS");

    if gtest_list_tests.is_some() {
        println!(
            r#"NOPE.
  NOPE0
  NOPE1"#
        );
        return;
    }

    assert!(
        gtest_shard_index < gtest_total_shards,
        "Shard index ({}) is too large for number of shards ({})",
        gtest_shard_index,
        gtest_total_shards
    );

    match gtest_total_shards {
        1 => {
            println!(
                r#"[==========] Running 2 tests from 1 test case.
[----------] Global test environment set-up.
[----------] 2 tests from NOPE
[ RUN      ] NOPE.NOPE0
[       OK ] NOPE.NOPE0 (0 ms)
[ RUN      ] NOPE.NOPE1
[       OK ] NOPE.NOPE1 (0 ms)"#
            );
        }
        2 => {
            println!(
                r#"[==========] Running 1 tests from 1 test case.
[----------] Global test environment set-up.
[----------] 1 tests from NOPE
[ RUN      ] NOPE.NOPE{}
[       OK ] NOPE.NOPE{} (0 ms)"#,
                gtest_shard_index, gtest_shard_index
            );
        }
        n => {
            panic!(
                "Request {} shards, but only up to 2 shards are supported",
                n
            );
        }
    };
}

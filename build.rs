use std::{env, fs, io, path::Path};

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

fn main() -> io::Result<()> {
    let args: Vec<_> = env::args().collect();
    let gtest_shard_index = parse_arg(&args, "gtest_shard_index", "GTEST_SHARD_INDEX")
        .and_then(|x| x.parse::<usize>().ok())
        .unwrap_or(0);
    let gtest_total_shards = parse_arg(&args, "gtest_total_shards", "GTEST_TOTAL_SHARDS")
        .and_then(|x| x.parse::<usize>().ok())
        .unwrap_or(1);
    let gtest_list_tests = parse_arg(&args, "gtest_list_tests", "GTEST_LIST_TESTS");

    if let Ok(out_dir) = env::var("OUT_DIR") {
        if let Ok(name) = env::current_exe() {
            fs::copy(name, Path::new(&out_dir).join("dummy-gtest-executable")).unwrap();
        }
    }

    if gtest_list_tests.is_some() {
        println!(
            r#"NOPE.
  NOPE0
  NOPE1"#
        );
        return Ok(());
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

    Ok(())
}

use clap::Parser;
use std::{env, fs, io, path::Path};

#[derive(Parser, Debug, Default)]
struct Args {
    #[clap(
        long("gtest_shard_index"),
        default_value = "0",
        env = "GTEST_SHARD_INDEX"
    )]
    gtest_shard_index: usize,

    #[clap(
        long("gtest_total_shards"),
        default_value = "1",
        env = "GTEST_TOTAL_SHARDS"
    )]
    gtest_total_shards: usize,

    #[clap(long("gtest_list_tests"))]
    gtest_list_tests: bool,
}

fn main() -> io::Result<()> {
    let args = Args::parse();

    // This check only passes at build time since otherwise `OUT_DIR` is not set.
    if let Ok(out_dir) = env::var("OUT_DIR") {
        if let Ok(src) = env::current_exe() {
            let dest = Path::new(&out_dir).join("dummy-gtest-executable");

            if src != dest {
                fs::copy(src, dest).unwrap();
            }
        }
    }

    if args.gtest_list_tests {
        println!(
            r#"NOPE.
  NOPE0
  NOPE1"#
        );
        return Ok(());
    }

    assert!(
        args.gtest_shard_index < args.gtest_total_shards,
        "Shard index ({}) is too large for number of shards ({})",
        args.gtest_shard_index,
        args.gtest_total_shards
    );

    match args.gtest_total_shards {
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
                args.gtest_shard_index, args.gtest_shard_index
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

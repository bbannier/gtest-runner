use {
    std::{env, fs, io, path::Path},
    structopt::StructOpt,
};

#[derive(StructOpt, Debug)]
#[structopt(rename_all = "verbatim")]
struct GtestOpt {
    #[structopt(long)]
    gtest_filter: Option<String>,

    #[structopt(long, default_value = "0", env = "GTEST_SHARD_INDEX")]
    gtest_shard_index: usize,

    #[structopt(long, default_value = "1", env = "GTEST_TOTAL_SHARDS")]
    gtest_total_shards: usize,

    #[structopt(long)]
    gtest_list_tests: bool,

    #[structopt(long, env = "OUT_DIR")]
    out_dir: Option<String>,
}

fn main() -> io::Result<()> {
    let opt = GtestOpt::from_args();

    if opt.out_dir.is_some() {
        if let Ok(name) = env::current_exe() {
            fs::copy(
                &name,
                Path::new(&opt.out_dir.unwrap()).join("dummy-gtest-executable"),
            )
            .unwrap();
        }
    }

    if opt.gtest_list_tests {
        println!(
            r#"NOPE.
  NOPE0
  NOPE1"#
        );
        return Ok(());
    }

    assert!(
        opt.gtest_shard_index < opt.gtest_total_shards,
        "Shard index ({}) is too large for number of shards ({})",
        opt.gtest_shard_index,
        opt.gtest_total_shards
    );

    match opt.gtest_total_shards {
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
                opt.gtest_shard_index, opt.gtest_shard_index
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

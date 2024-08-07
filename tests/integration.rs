use std::env;

use gtest::{
    exec::exec,
    opt::{Opt, RunMode},
};
use rstest::{fixture, rstest};

#[fixture]
fn exe() -> &'static str {
    // Activate sample data mode for the executable.
    env::set_var("GTEST_RUNNER_SAMPLE_DATA", "true");

    // The test executable is the runner binary.
    env!("CARGO_BIN_EXE_gtest-runner")
}

#[rstest]
fn run1(exe: &str) {
    assert_eq!(0, gtest::run(exe, None, 1, 0, 0).unwrap());
}

#[rstest]
fn run2(exe: &str) {
    assert_eq!(0, gtest::run(exe, None, 2, 0, 0).unwrap());
}

#[rstest]
fn get_tests(exe: &str) {
    let num_tests = gtest::exec::get_tests(exe, false).map(|xs| xs.len());
    assert_eq!(2, num_tests.unwrap());
}

#[rstest]
fn trace(exe: &str) {
    let opt = Opt {
        trace: true,
        mode: RunMode {
            test_executables: Some(vec![exe.into()]),
            ..RunMode::default()
        },
        ..Default::default()
    };

    let cwd = std::env::current_dir().expect("Could not get current directory");

    let get_traces = |dir: &std::path::PathBuf| -> std::collections::HashSet<_> {
        std::fs::read_dir(&dir)
            .expect("Could not list directory")
            .map(|entry| entry.expect("Could not get directory entry").path())
            .filter(|path| {
                std::path::Path::new(&path)
                    .extension()
                    .and_then(std::ffi::OsStr::to_str)
                    .map(|ext| ext == "trace")
                    .is_some()
            })
            .collect()
    };

    let traces1 = get_traces(&cwd);
    exec(&opt).expect("Could not execute test executable");
    let traces2 = get_traces(&cwd);

    let traces = traces2.difference(&traces1).collect::<Vec<_>>();
    assert_eq!(
        traces.len(),
        1,
        "Expected exactly one trace file to be created"
    );

    let trace = traces[0];
    let size = trace
        .metadata()
        .expect("Could not get trace metadata")
        .len();
    assert!(size > 100, "Unexpected of small size of trace file");
    std::fs::remove_file(trace).expect("Could not remove test trace");
}

gtest-runner [![Build Status](https://travis-ci.org/bbannier/gtest-runner.svg?branch=master)](https://travis-ci.org/bbannier/gtest-runner)
============

A parallel test runner for [googletest](https://github.com/googletest).


Screenshots
-----------

For successful runs only minimal output is shown.

![Screenshot of run without failures](screenshot_success.gif)


If a test fails its log is shown.

![Screenshot of run with_failures](screenshot_failures.gif)

Usage
-----

    USAGE:
        gtest-runner [OPTIONS] <test_executable>...

    FLAGS:
        -h, --help       Prints help information
        -V, --version    Prints version information

    OPTIONS:
        -j, --jobs <jobs>               [env: GTEST_RUNNER_JOBS=]  [default: 8]
        -v, --verbosity <verbosity>     [env: GTEST_RUNNER_VERBOSITY=]  [default: 2]

    ARGS:
        <test_executable>...

Installation
------------

Installation requires a recent rust compiler.

    cargo install --git https://github.com/bbannier/gtest-runner


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
        gtest-runner [FLAGS] [OPTIONS] <test_executable>...

    FLAGS:
        -h, --help
                Prints help information

        -t, --trace
                Control tracing output.

                If this flag is present a chrome://tracing execution trace (http://dev.chromium.org/developers/how-
                tos/trace-event-profiling-tool) will be dumped to the current
                directory as `<pid>.trace` which can be used to analyze e.g., temporal relations between tests or their
                duration. The resulting file can e.g., directly be loaded into Google Chrome under chrome://tracing, or
                converted to HTML with `trace2html`.
        -V, --version
                Prints version information


    OPTIONS:
        -j, --jobs <jobs>
                Number of parallel jobs.

                This flag controls how many parallel jobs are used to execute test shards. We do not execute more jobs than
                there are tests (also see `progress`). Depending on the exact test workload, test execution typically
                becomes faster with more jobs until it reaches a plateau or even decreases when too many parallel executions
                compete for system resources (e.g., file system access; scheduling by the processor).

                This flag can be controlled with an environment variable and by default is set to the number of processors
                available to the runner process [env: GTEST_RUNNER_JOBS=]  [default: 12]
        -v, --verbosity <verbosity>
                Runner verbosity.

                This flag controls the verbosity with which the test runner reports execution progress and results.

                v=0: Do not provide any output during test execution. Report failed tests at the end.
                v=1: Report global test progress. Report failed tests at the end.
                v=2: Report currently executing tests. Report failed tests at the end.
                v>2: Pass through and report all test output.

                This flag can be controlled with an environment variable and has a default value [env:
                GTEST_RUNNER_VERBOSITY=]  [default: 2]

    ARGS:
        <test_executable>...
                One or more GTest executables.

                The test runner can execute tests from the same executable in parallel, but will currently not run different
                test executables in parallel. In order for tests to be executable in parallel they likely should not depend
                on system information (e.g., the ability to bind to fixed ports; the presence or absence of especially test-
                created files in fixed file system locations, etc.).

Installation
------------

Installation requires a recent rust compiler.

    cargo install --git https://github.com/bbannier/gtest-runner


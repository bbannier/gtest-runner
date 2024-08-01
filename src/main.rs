use {
    anyhow::Result,
    clap::Parser,
    gtest::{exec::exec, opt::Opt},
};

fn main() -> Result<()> {
    let opt = Opt::parse();

    std::process::exit(exec(&opt)?);
}

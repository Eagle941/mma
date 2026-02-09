use std::path::PathBuf;
use std::process;

use anyhow::anyhow;
use clap::Parser;
use exitcode::{OK, SOFTWARE};

#[derive(Clone, Parser, Debug)]
pub struct Args {}

fn main() {
    let args = Args::parse();

    match run(args) {
        Ok(()) => process::exit(OK),
        Err(e) => {
            eprintln!("Internal software error: {e}");
            process::exit(SOFTWARE);
        }
    }
}

fn run(args: Args) -> anyhow::Result<()> {
    Ok(())
}

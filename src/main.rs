use clap::{Parser, Subcommand};
use deflate_rs::{Compressor, Decompressor};
use std::io;

#[derive(Debug, Subcommand)]
enum Command {
    Compress,
    Decompress,
}

#[derive(Debug, Parser)]
struct Args {
    #[command(subcommand)]
    command: Command,
}

fn main() -> anyhow::Result<()> {
    let Args { command } = Args::try_parse()?;
    match command {
        Command::Compress => {
            let mut compressor = Compressor::new(io::stdin().lock(), io::stdout().lock());
            compressor.compress()?;
            Ok(())
        }
        Command::Decompress => {
            let mut decompressor = Decompressor::new(io::stdin().lock(), io::stdout().lock());
            decompressor.decompress()?;
            Ok(())
        }
    }
}

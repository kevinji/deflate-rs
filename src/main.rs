use clap::{Parser, Subcommand};
use deflate_rs::{DeflateDecoder, DeflateEncoder};
use std::io;

#[derive(Debug, Subcommand)]
enum Command {
    DeflateEncode,
    DeflateDecode,
}

#[derive(Debug, Parser)]
struct Args {
    #[command(subcommand)]
    command: Command,
}

fn main() -> anyhow::Result<()> {
    let Args { command } = Args::try_parse()?;
    match command {
        Command::DeflateEncode => {
            let mut encoder = DeflateEncoder::new(io::stdin().lock(), io::stdout().lock());
            encoder.encode()?;
            Ok(())
        }
        Command::DeflateDecode => {
            let mut decoder = DeflateDecoder::new(io::stdin().lock(), io::stdout().lock());
            decoder.decode()?;
            Ok(())
        }
    }
}

use clap::{Parser, Subcommand};
use deflate_rs::{BitReader, DeflateDecoder, DeflateEncoder};
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
            let mut encoder = DeflateEncoder::new();
            encoder.encode(&mut io::stdin().lock(), &mut io::stdout().lock())?;
            Ok(())
        }
        Command::DeflateDecode => {
            let mut decoder = DeflateDecoder::new();
            decoder.decode(
                &mut BitReader::new(io::stdin().lock()),
                &mut io::stdout().lock(),
            )?;
            Ok(())
        }
    }
}

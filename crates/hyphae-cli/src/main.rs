// SPDX-License-Identifier: Apache-2.0

//! Command-line entry point for the single Hyphae executable.

use std::{
    error::Error,
    io::{BufWriter, Write, stdout},
};

use clap::{Parser, Subcommand};
use hyphae_core::current_version;
use serde_json::json;

#[derive(Debug, Parser)]
#[command(
    name = "hyphae",
    version,
    about = "Autonomous, embeddable, and verifiable data engine"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Print independently versioned product surfaces.
    Version {
        /// Emit a machine-readable JSON object.
        #[arg(long)]
        json: bool,
    },
}

fn main() -> Result<(), Box<dyn Error>> {
    match Cli::parse().command {
        Command::Version { json } => print_version(json),
    }
}

fn print_version(json_output: bool) -> Result<(), Box<dyn Error>> {
    let version = current_version();
    let mut output = BufWriter::new(stdout().lock());

    if json_output {
        let value = json!({
            "product": version.product,
            "engine_version": version.engine,
            "api_version": version.api,
            "disk_format_version": version.disk_format,
        });
        serde_json::to_writer_pretty(&mut output, &value)?;
        writeln!(output)?;
    } else {
        writeln!(
            output,
            "{} {} (api {}, disk format {})",
            version.product, version.engine, version.api, version.disk_format
        )?;
    }

    Ok(())
}

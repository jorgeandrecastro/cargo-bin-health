mod cargo_info;
mod cli;
mod elf;
mod health;
mod memory_x;
mod report;

use anyhow::{Context, Result};
use clap::Parser;
use cli::Cli;

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error:#}");
        std::process::exit(2);
    }
}

fn run() -> Result<()> {
    // Cargo invokes subcommands as: cargo-bin-health bin-health [OPTIONS].
    let mut args: Vec<String> = std::env::args().collect();
    if args.get(1).is_some_and(|arg| arg == "bin-health") {
        args.remove(1);
    }
    let cli = Cli::parse_from(args);
    let context = cargo_info::inspect(&cli).context("Cargo project inspection failed")?;
    let elf = elf::inspect(&context.binary, cli.top).context("ELF inspection failed")?;
    let memory = memory_x::find_and_parse(&context.workspace_root, &context.manifest_dir);
    let diagnostics = health::analyze(&context, &elf);
    let report = report::Report::new(context, elf, memory, diagnostics);
    report::render(&report, &cli)?;
    if cli.ci && report::thresholds_exceeded(&report, &cli) {
        std::process::exit(1);
    }
    Ok(())
}

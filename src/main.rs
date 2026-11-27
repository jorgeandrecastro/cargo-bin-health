use anyhow::{Context, Result};
use clap::Parser;
use cargo_bin_health::{
    accounting,
    cargo_info,
    cli::Cli,
    elf,
    health,
    memory_profiles,
    memory_x,
    report,
};

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error:#}");
        std::process::exit(2);
    }
}

fn run() -> Result<()> {
    // Cargo invokes subcommands as: cargo-bin-health bin-health [OPTIONS].
    let mut args: Vec<String> = std::env::args().collect();
    if args.get(1).is_some_and(|argument| argument == "bin-health") {
        args.remove(1);
    }
    let cli = Cli::parse_from(args);
    let context = cargo_info::inspect(&cli).context("Cargo project inspection failed")?;

    let linker_map = memory_x::find_and_parse(
        &context.workspace_root,
        &context.manifest_dir,
        context.metadata.target_directory.as_std_path(),
        &context.binary,
    );
    let profile = memory_profiles::resolve_profile(
        cli.memory_profile,
        cli.target.as_deref(),
        linker_map.as_ref(),
    );
    let elf_report = elf::inspect(&context.binary, cli.top).context("ELF inspection failed")?;
    let memory_accounting = accounting::analyze(&elf_report, &profile, linker_map.as_ref());
    let diagnostics = health::analyze(&context, &elf_report);

    let report = report::Report::new(
        context,
        elf_report,
        profile,
        linker_map,
        memory_accounting,
        diagnostics,
    );
    report::render(&report, &cli)?;

    if cli.ci && report::thresholds_exceeded(&report, &cli) {
        std::process::exit(1);
    }
    Ok(())
}

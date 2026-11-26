use crate::{
    cargo_info::CargoContext, cli::Cli, elf::ElfReport, health::Diagnostic, memory_x::MemoryRegions,
};
use anyhow::Result;
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct Report {
    pub context: CargoContextReport,
    pub elf: ElfReport,
    pub memory: Option<MemoryRegions>,
    pub diagnostics: Vec<Diagnostic>,
}
#[derive(Debug, Serialize)]
pub struct CargoContextReport {
    pub package: String,
    pub target: String,
    pub binary: String,
}

impl Report {
    pub fn new(
        context: CargoContext,
        elf: ElfReport,
        memory: Option<MemoryRegions>,
        diagnostics: Vec<Diagnostic>,
    ) -> Self {
        Self {
            context: CargoContextReport {
                package: context.package.name.to_string(),
                target: context.target.name,
                binary: context.binary.display().to_string(),
            },
            elf,
            memory,
            diagnostics,
        }
    }
}

pub fn render(report: &Report, cli: &Cli) -> Result<()> {
    if cli.json {
        println!("{}", serde_json::to_string_pretty(report)?);
        return Ok(());
    }
    let color = !cli.no_color && std::env::var_os("NO_COLOR").is_none();
    println!("cargo-bin-health — {}", report.context.package);
    println!("Artifact : {}", report.elf.path);
    println!("Flash    : {} bytes", report.elf.flash_bytes);
    println!("RAM      : {} bytes", report.elf.ram_bytes);
    if let Some(memory) = &report.memory {
        if let Some(total) = memory.flash_bytes {
            print_capacity("FLASH", report.elf.flash_bytes, total);
        }
        if let Some(total) = memory.ram_bytes {
            print_capacity("RAM", report.elf.ram_bytes, total);
        }
    } else {
        println!("memory.x : not found (capacity percentages unavailable)");
    }
    println!("\nTop symbols:");
    for symbol in &report.elf.top_symbols {
        println!("  {:>9} B  {}", symbol.size, symbol.name);
    }
    println!("\nDiagnostics:");
    for diagnostic in &report.diagnostics {
        let prefix = match diagnostic.level.as_str() {
            "warning" => "WARN",
            "info" => "INFO",
            "ok" => " OK ",
            _ => "----",
        };
        if color && diagnostic.level == "warning" {
            println!("\x1b[33m[{prefix}]\x1b[0m {}", diagnostic.message);
        } else if color && diagnostic.level == "ok" {
            println!("\x1b[32m[{prefix}]\x1b[0m {}", diagnostic.message);
        } else {
            println!("[{prefix}] {}", diagnostic.message);
        }
    }
    Ok(())
}

fn print_capacity(label: &str, used: u64, total: u64) {
    let pct = if total == 0 {
        0.0
    } else {
        used as f64 * 100.0 / total as f64
    };
    let filled = (pct.clamp(0.0, 100.0) / 5.0).round() as usize;
    println!(
        "{label:<7}: {used}/{total} bytes ({pct:.1}%) [{}{}]",
        "#".repeat(filled),
        "-".repeat(20usize.saturating_sub(filled))
    );
}

pub fn thresholds_exceeded(report: &Report, cli: &Cli) -> bool {
    let flash = report
        .memory
        .as_ref()
        .and_then(|m| m.flash_bytes)
        .map(|total| report.elf.flash_bytes as f64 * 100.0 / total as f64);
    let ram = report
        .memory
        .as_ref()
        .and_then(|m| m.ram_bytes)
        .map(|total| report.elf.ram_bytes as f64 * 100.0 / total as f64);
    cli.max_flash_pct
        .zip(flash)
        .is_some_and(|(limit, used)| used > limit)
        || cli
            .max_ram_pct
            .zip(ram)
            .is_some_and(|(limit, used)| used > limit)
}

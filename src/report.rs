use crate::{
    cargo_info::CargoContext,
    cli::Cli,
    elf::ElfReport,
    health::Diagnostic,
    memory_x::MemoryRegions,
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
        if let Some(total) = memory.flash_bytes.filter(|value| *value > 0) {
            print_capacity("FLASH", report.elf.flash_bytes, total);
        }
        if let Some(total) = memory.ram_bytes.filter(|value| *value > 0) {
            print_capacity("RAM", report.elf.ram_bytes, total);
        }
        if !memory.regions.is_empty() {
            println!("\nLinker memory regions (declared windows):");
            for region in &memory.regions {
                let origin = region.origin.map(|v| format!("{v:#010x}")).unwrap_or_else(|| "unknown".into());
                let length = region.length
                    .map(|v| format!("{v} bytes ({v:#x})"))
                    .unwrap_or_else(|| "unknown / symbolic expression".into());
                let end = region.end.map(|v| format!("{v:#010x}")).unwrap_or_else(|| "unknown".into());
                println!("  {:<12} origin={origin:<12} length={length:<26} end={end}", region.name);
            }
            if let Some(source) = &memory.source {
                println!("  source: {source}");
            }
            println!("  Note: linker windows can alias the same physical memory; capacities are not summed.");
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
    if total == 0 {
        println!("{label:<7}: capacity unavailable (zero-sized linker region)");
        return;
    }
    let pct = used as f64 * 100.0 / total as f64;
    let filled = (pct.clamp(0.0, 100.0) / 5.0).round() as usize;
    println!(
        "{label:<7}: {used}/{total} bytes ({pct:.1}%) [{}{}]",
        "#".repeat(filled),
        "-".repeat(20usize.saturating_sub(filled))
    );
}

pub fn thresholds_exceeded(report: &Report, cli: &Cli) -> bool {
    let flash_pct = report.memory.as_ref()
        .and_then(|memory| memory.flash_bytes)
        .filter(|total| *total > 0)
        .map(|total| report.elf.flash_bytes as f64 * 100.0 / total as f64);

    let ram_pct = report.memory.as_ref()
        .and_then(|memory| memory.ram_bytes)
        .filter(|total| *total > 0)
        .map(|total| report.elf.ram_bytes as f64 * 100.0 / total as f64);

    cli.max_flash_pct.zip(flash_pct)
        .is_some_and(|(limit, used)| limit.is_finite() && used > limit)
        || cli.max_ram_pct.zip(ram_pct)
            .is_some_and(|(limit, used)| limit.is_finite() && used > limit)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{cli::Cli, elf::ElfReport};

    fn sample_report() -> Report {
        Report {
            context: CargoContextReport {
                package: "demo".into(),
                target: "demo".into(),
                binary: "demo".into(),
            },
            elf: ElfReport {
                path: "demo".into(),
                flash_bytes: 50,
                ram_bytes: 25,
                accounting_method: "test".into(),
                sections: vec![],
                segments: vec![],
                top_symbols: vec![],
            },
            memory: Some(MemoryRegions {
                flash_bytes: Some(100),
                ram_bytes: Some(100),
                source: None,
                regions: vec![],
            }),
            diagnostics: vec![],
        }
    }

    #[test]
    fn thresholds_only_fail_when_configured_limit_is_exceeded() {
        let report = sample_report();
        let cli = Cli {
            manifest_path: None, release: false, profile: None, target: None,
            package: None, bin: None, example: None, features: vec![],
            no_default_features: false, no_build: false, binary: None, top: 10,
            max_flash_pct: Some(40.0), max_ram_pct: Some(30.0), ci: true,
            json: false, no_color: true,
        };
        assert!(thresholds_exceeded(&report, &cli));
    }
}

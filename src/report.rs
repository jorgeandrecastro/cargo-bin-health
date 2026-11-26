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
    println!("ELF Flash allocation : {} bytes", report.elf.flash_bytes);
    println!("ELF RAM allocation   : {} bytes", report.elf.ram_bytes);

    if let Some(memory) = &report.memory {
        println!("\nLinker memory capacities:");
        if let Some(capacity) = memory.flash_bytes {
            print_capacity("FLASH mapped window", report.elf.flash_bytes, capacity);
        } else {
            println!("  FLASH: capacity unavailable");
        }

        if let Some(capacity) = memory.ram_bytes.filter(|capacity| *capacity > 0) {
            let aggregate_pct = memory.ram_used_bytes as f64 * 100.0 / capacity as f64;
            println!(
                "  RAM alias-aware linker capacity: {} bytes; mapped usage: {} bytes ({aggregate_pct:.1}% aggregate)",
                capacity, memory.ram_used_bytes
            );
            println!("  RAM capacity policy: {}", memory.ram_capacity_policy);
            if let Some(worst) = memory.ram_worst_group_pct {
                println!("  Worst RAM allocation-group utilization: {worst:.1}%");
            } else if memory.unmapped_ram_bytes > 0 {
                println!(
                    "  RAM threshold unavailable: {} bytes could not be mapped to declared RAM regions",
                    memory.unmapped_ram_bytes
                );
            }
        } else {
            println!("  RAM: capacity unavailable");
        }

        println!("\nLinker regions:");
        for region in &memory.regions {
            let origin = region
                .origin
                .map(|value| format!("{value:#010x}"))
                .unwrap_or_else(|| "unknown".into());
            let end = region
                .end
                .map(|value| format!("{value:#010x}"))
                .unwrap_or_else(|| "unknown".into());
            let length = region
                .length
                .map(|value| format!("{value} B"))
                .unwrap_or_else(|| "unknown".into());
            let usage = region
                .utilization_pct
                .map(|pct| format!("{}/{length} ({pct:.1}%)", region.used_bytes))
                .unwrap_or_else(|| format!("0/{length} (n/a)"));
            println!(
                "  {:<12} {:<7} origin={} end={} used={}",
                region.name, region.kind, origin, end, usage
            );
        }

        println!("\nRAM allocation groups (target-alias aware):");
        for group in &memory.ram_groups {
            let pct = group.utilization_pct
                .map(|value| format!("{value:.1}%"))
                .unwrap_or_else(|| "n/a".into());
            println!(
                "  {:<30} used={}/{} bytes ({pct}); regions={}",
                group.name,
                group.used_bytes,
                group.capacity_bytes,
                group.regions.join(",")
            );
        }
        println!("  Flash policy: {}", memory.flash_capacity_policy);
        println!("  RAM policy:   {}", memory.ram_capacity_policy);
        if let Some(source) = &memory.source {
            println!("  source: {source}");
        }
    } else {
        println!("\nmemory.x: not found (capacity percentages unavailable)");
    }

    println!("\nTop symbols:");
    if report.elf.top_symbols.is_empty() {
        println!("  No function symbols available (ELF may be stripped).");
    }
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
        println!("  {label}: capacity unavailable (zero-sized linker window)");
        return;
    }

    let pct = used as f64 * 100.0 / total as f64;
    let filled = (pct.clamp(0.0, 100.0) / 5.0).round() as usize;
    println!(
        "  {label}: {used}/{total} bytes ({pct:.1}%) [{}{}]",
        "#".repeat(filled),
        "-".repeat(20usize.saturating_sub(filled))
    );
}

/// CI RAM threshold semantics are intentionally conservative:
/// fail when ANY physical RAM allocation group exceeds the configured percentage.
/// Known linker aliases are normalized into one physical group. This catches
/// a nearly-full shared SRAM pool even when one individual VMA window looks sparse.
pub fn thresholds_exceeded(report: &Report, cli: &Cli) -> bool {
    let flash_pct = report
        .memory
        .as_ref()
        .and_then(|memory| memory.flash_bytes)
        .filter(|capacity| *capacity > 0)
        .map(|capacity| report.elf.flash_bytes as f64 * 100.0 / capacity as f64);

    let ram_pct = report
        .memory
        .as_ref()
        .and_then(|memory| memory.ram_worst_group_pct);

    let flash_failed = cli.max_flash_pct.is_some_and(|limit| {
        !limit.is_finite() || flash_pct.is_none_or(|used| used > limit)
    });
    let ram_failed = cli.max_ram_pct.is_some_and(|limit| {
        !limit.is_finite() || ram_pct.is_none_or(|used| used > limit)
    });

    flash_failed || ram_failed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{cli::Cli, elf::ElfReport, memory_x::parse_memory_x};

    fn sample_report() -> Report {
        let mut memory = parse_memory_x(
            "MEMORY {\nFLASH : ORIGIN=0x10000000, LENGTH=1M\nRAM_A : ORIGIN=0x20000000, LENGTH=100\nRAM_B : ORIGIN=0x20000100, LENGTH=100\n}",
        );
        // Simulate RAM region usage: RAM_A=40%, RAM_B=95%.
        memory.regions[1].used_bytes = 40;
        memory.regions[1].utilization_pct = Some(40.0);
        memory.regions[2].used_bytes = 95;
        memory.regions[2].utilization_pct = Some(95.0);
        memory.ram_worst_group_pct = Some(95.0);

        Report {
            context: CargoContextReport {
                package: "demo".into(),
                target: "demo".into(),
                binary: "demo".into(),
            },
            elf: ElfReport {
                path: "demo".into(),
                flash_bytes: 50,
                ram_bytes: 135,
                accounting_method: "test".into(),
                sections: vec![],
                segments: vec![],
                top_symbols: vec![],
            },
            memory: Some(memory),
            diagnostics: vec![],
        }
    }

    fn cli(max_ram_pct: Option<f64>) -> Cli {
        Cli {
            manifest_path: None,
            release: false,
            profile: None,
            target: None,
            package: None,
            bin: None,
            example: None,
            features: vec![],
            no_default_features: false,
            no_build: true,
            binary: None,
            top: 10,
            max_flash_pct: None,
            max_ram_pct,
            ci: true,
            json: false,
            no_color: true,
        }
    }

    #[test]
    fn ram_threshold_uses_worst_alias_aware_group_not_aggregate_capacity() {
        let report = sample_report();
        assert!(thresholds_exceeded(&report, &cli(Some(90.0))));
        assert!(!thresholds_exceeded(&report, &cli(Some(96.0))));
    }

    #[test]
    fn esp32c3_ram_threshold_uses_the_shared_internal_sram_group() {
        let script = include_str!("../tests/fixtures/esp32c3-memory.x");
        let mut memory = parse_memory_x(script);
        let sections = [
            (0x4038_0000, 0x9b34),
            (0x3fc8_9b38, 0x1ac8),
            (0x3fc8_b600, 0x01f0),
            (0x3fc8_b7f0, 0xc960),
            (0x3fc9_8150, 0x362b0),
            (0x3fcc_e400, 0x10000),
        ];
        for (address, size) in sections {
            let index = memory.find_region_index(address, size).unwrap();
            memory.record_section(index, address, size, 0);
        }
        memory.finalize_usage(0, 0);

        let mut report = sample_report();
        report.memory = Some(memory);

        assert!(thresholds_exceeded(&report, &cli(Some(90.0))));
        assert!(!thresholds_exceeded(&report, &cli(Some(99.9))));
    }

    #[test]
    fn unknown_ram_mapping_does_not_report_a_false_green_threshold() {
        let mut report = sample_report();
        report.memory.as_mut().unwrap().unmapped_ram_bytes = 32;
        report.memory.as_mut().unwrap().ram_worst_group_pct = None;
        assert!(thresholds_exceeded(&report, &cli(Some(90.0))));
    }
}

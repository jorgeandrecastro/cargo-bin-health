use crate::{
    accounting::{MemoryAccounting, PoolUsage},
    cargo_info::CargoContext,
    cli::Cli,
    elf::ElfReport,
    health::Diagnostic,
    memory_profiles::MemoryProfile,
    memory_x::MemoryRegions,
};
use anyhow::Result;
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct Report {
    pub context: CargoContextReport,
    pub elf: ElfReport,
    pub profile: MemoryProfile,
    pub memory_map: Option<MemoryRegions>,
    pub accounting: MemoryAccounting,
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
        profile: MemoryProfile,
        memory_map: Option<MemoryRegions>,
        accounting: MemoryAccounting,
        diagnostics: Vec<Diagnostic>,
    ) -> Self {
        Self {
            context: CargoContextReport {
                package: context.package.name.to_string(),
                target: context.target.name,
                binary: context.binary.display().to_string(),
            },
            elf,
            profile,
            memory_map,
            accounting,
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
    println!(
        "ELF      : machine={} class={} entry={:#x}",
        report.elf.machine,
        if report.elf.elf_class == goblin::elf::header::ELFCLASS64 { "ELF64" } else { "ELF32" },
        report.elf.entry
    );
    println!("Profile  : {} ({})", report.profile.display_name, report.profile.id);
    println!("Flash    : {} bytes linked", report.accounting.flash_used_bytes);
    println!("RAM      : {} bytes allocated", report.accounting.ram_used_bytes);

    print_capacity(
        "FLASH",
        report.accounting.flash_used_bytes,
        report.accounting.flash_capacity_bytes,
        report.accounting.flash_utilization_pct,
    );
    print_capacity(
        "RAM",
        report.accounting.ram_used_bytes,
        report.accounting.ram_capacity_bytes,
        report.accounting.ram_utilization_pct,
    );

    if !report.accounting.ram_pools.is_empty() {
        println!("\nRAM pools:");
        for pool in &report.accounting.ram_pools {
            print_pool(pool);
        }
    }

    if !report.accounting.regions.is_empty() {
        println!("\nLinker regions:");
        for region in &report.accounting.regions {
            let origin = region.origin.map(|value| format!("{value:#010x}")).unwrap_or_else(|| "unknown".into());
            let capacity = region.length.map(|value| value.to_string()).unwrap_or_else(|| "unknown".into());
            let pct = region.utilization_pct.map(|value| format!("{value:.1}%")).unwrap_or_else(|| "n/a".into());
            println!(
                "  {:<14} {:<8?} origin={} used={} B / {} B ({})",
                region.name, region.class, origin, region.used_bytes, capacity, pct
            );
        }
    }

    if let Some(source) = report.memory_map.as_ref().and_then(|map| map.source.as_ref()) {
        println!("\nLinker map: {source}");
    } else {
        println!("\nLinker map: not found; profile defaults are used where available");
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

    for note in &report.accounting.notes {
        println!("[NOTE] {note}");
    }
    Ok(())
}

fn print_pool(pool: &PoolUsage) {
    let capacity = pool.capacity_bytes.map(|value| value.to_string()).unwrap_or_else(|| "unknown".into());
    let pct = pool.utilization_pct.map(|value| format!("{value:.1}%")).unwrap_or_else(|| "n/a".into());
    println!(
        "  {:<28} used={:>9} / {:>9} B ({}) regions={}",
        pool.label,
        pool.used_bytes,
        capacity,
        pct,
        pool.regions.join(",")
    );
}

fn print_capacity(label: &str, used: u64, total: Option<u64>, percentage: Option<f64>) {
    let Some(total) = total.filter(|value| *value > 0) else {
        println!("{label:<7}: capacity unavailable (provide a linker MEMORY region or select a supported profile)");
        return;
    };
    let pct = percentage.unwrap_or_else(|| used as f64 * 100.0 / total as f64);
    let filled = (pct.clamp(0.0, 100.0) / 5.0).round() as usize;
    println!(
        "{label:<7}: {used}/{total} bytes ({pct:.1}%) [{}{}]",
        "#".repeat(filled),
        "-".repeat(20usize.saturating_sub(filled))
    );
}

/// CI threshold checks fail closed: a configured limit is considered failed
/// when the corresponding capacity/utilization cannot be computed reliably.
pub fn thresholds_exceeded(report: &Report, cli: &Cli) -> bool {
    let flash_failed = cli.max_flash_pct.is_some_and(|limit| {
        !limit.is_finite()
            || !(0.0..=100.0).contains(&limit)
            || report.accounting.flash_utilization_pct.is_none_or(|used| used > limit)
    });
    let ram_failed = cli.max_ram_pct.is_some_and(|limit| {
        !limit.is_finite()
            || !(0.0..=100.0).contains(&limit)
            || !report.accounting.ram_capacity_complete
            || report.accounting.worst_ram_pool_pct.is_none_or(|used| used > limit)
    });
    flash_failed || ram_failed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        accounting::MemoryAccounting,
        cli::{Cli, MemoryProfileArg},
        elf::ElfReport,
        memory_profiles::generic_profile,
    };

    fn cli() -> Cli {
        Cli {
            manifest_path: None,
            release: false,
            profile: None,
            target: None,
            memory_profile: MemoryProfileArg::Auto,
            package: None,
            bin: None,
            example: None,
            features: vec![],
            no_default_features: false,
            no_build: true,
            binary: None,
            top: 10,
            max_flash_pct: Some(80.0),
            max_ram_pct: Some(80.0),
            ci: true,
            json: false,
            no_color: true,
        }
    }

    fn sample_report() -> Report {
        Report {
            context: CargoContextReport {
                package: "demo".into(),
                target: "demo".into(),
                binary: "demo.elf".into(),
            },
            elf: ElfReport {
                path: "demo.elf".into(),
                machine: 0,
                elf_class: goblin::elf::header::ELFCLASS32,
                little_endian: true,
                entry: 0,
                sections: vec![],
                segments: vec![],
                top_symbols: vec![],
            },
            profile: generic_profile(),
            memory_map: None,
            accounting: MemoryAccounting {
                profile_id: "generic".into(),
                profile_name: "Generic ELF".into(),
                flash_used_bytes: 90,
                flash_capacity_bytes: Some(100),
                flash_utilization_pct: Some(90.0),
                ram_used_bytes: 95,
                ram_capacity_bytes: Some(100),
                ram_utilization_pct: Some(95.0),
                worst_ram_pool_pct: Some(95.0),
                ram_capacity_complete: true,
                unclassified_ram_bytes: 0,
                regions: vec![],
                ram_pools: vec![],
                notes: vec![],
            },
            diagnostics: vec![],
        }
    }

    #[test]
    fn thresholds_fail_when_a_configured_limit_is_exceeded() {
        assert!(thresholds_exceeded(&sample_report(), &cli()));
    }

    #[test]
    fn ram_threshold_fails_closed_when_capacity_is_unknown() {
        let mut report = sample_report();
        report.accounting.ram_capacity_complete = false;
        report.accounting.worst_ram_pool_pct = None;
        assert!(thresholds_exceeded(&report, &cli()));
    }
}

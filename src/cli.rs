use clap::{ArgAction, Parser, ValueEnum};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Default)]
#[value(rename_all = "kebab-case")]
pub enum MemoryProfileArg {
    /// Detect ESP32-C3 from its linker map; otherwise use a generic profile.
    #[default]
    Auto,
    /// Espressif ESP32-C3 memory windows and shared internal SRAM budget.
    Esp32c3,
    /// Raspberry Pi RP2040 (264 KiB main SRAM).
    Rp2040,
    /// Raspberry Pi RP2350 (520 KiB SRAM).
    Rp2350,
    /// Generic ARM Cortex-M; capacities come from the linker script.
    CortexM,
    /// Generic ELF; capacities come from the linker script when available.
    Generic,
}

#[derive(Debug, Parser)]
#[command(
    name = "cargo-bin-health",
    version,
    about = "Inspect Rust binary and firmware health"
)]
pub struct Cli {
    /// Path to the Cargo.toml to inspect.
    #[arg(long, value_name = "PATH")]
    pub manifest_path: Option<PathBuf>,
    /// Build in release mode.
    #[arg(long, conflicts_with = "profile")]
    pub release: bool,
    /// Cargo build profile (for example: dev, release, firmware).
    #[arg(long)]
    pub profile: Option<String>,
    /// Target triple.
    #[arg(long)]
    pub target: Option<String>,
    /// Memory-accounting profile. RP2040/RP2350 are explicit because their
    /// Rust target triples are shared with other Cortex-M devices.
    #[arg(long, value_enum, default_value = "auto")]
    pub memory_profile: MemoryProfileArg,
    /// Package to inspect.
    #[arg(long, short = 'p')]
    pub package: Option<String>,
    /// Binary target name.
    #[arg(long)]
    pub bin: Option<String>,
    /// Example target name.
    #[arg(long)]
    pub example: Option<String>,
    /// Space- or comma-separated Cargo features.
    #[arg(long, value_delimiter = ',')]
    pub features: Vec<String>,
    /// Disable default features.
    #[arg(long, action = ArgAction::SetTrue)]
    pub no_default_features: bool,
    /// Inspect an existing artifact without running cargo build.
    #[arg(long)]
    pub no_build: bool,
    /// Existing ELF path (useful with --no-build).
    #[arg(long)]
    pub binary: Option<PathBuf>,
    /// Number of largest function symbols to display.
    #[arg(long, default_value_t = 10)]
    pub top: usize,
    /// Fail CI if Flash usage exceeds this percentage.
    #[arg(long)]
    pub max_flash_pct: Option<f64>,
    /// Fail CI if any RAM allocation pool exceeds this percentage.
    #[arg(long)]
    pub max_ram_pct: Option<f64>,
    /// Return a non-zero status when a configured threshold is exceeded.
    #[arg(long)]
    pub ci: bool,
    /// Emit machine-readable JSON.
    #[arg(long)]
    pub json: bool,
    /// Disable ANSI color output.
    #[arg(long)]
    pub no_color: bool,
}

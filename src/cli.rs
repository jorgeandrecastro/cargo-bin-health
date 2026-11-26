use clap::{ArgAction, Parser};
use std::path::PathBuf;

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

    /// Number of largest symbols to display.
    #[arg(long, default_value_t = 10)]
    pub top: usize,

    /// Fail CI if flash usage exceeds this percentage.
    #[arg(long)]
    pub max_flash_pct: Option<f64>,

    /// Fail CI if any alias-aware RAM allocation group exceeds this percentage.
    /// Fails closed when a complete RAM memory map is unavailable.
    #[arg(long, help = "Fail CI if any alias-aware RAM allocation group exceeds this percentage (fails closed if RAM accounting is incomplete)")]
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

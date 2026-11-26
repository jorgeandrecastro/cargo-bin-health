use anyhow::{Context, Result, bail};
use cargo_metadata::{Metadata, MetadataCommand, Package, Target, TargetKind};
use std::{path::PathBuf, process::Command};

use crate::cli::Cli;

#[derive(Debug)]
pub struct CargoContext {
    pub metadata: Metadata,
    pub package: Package,
    pub target: Target,
    pub binary: PathBuf,
    pub workspace_root: PathBuf,
    pub manifest_dir: PathBuf,
    pub no_std: bool,
}

pub fn inspect(cli: &Cli) -> Result<CargoContext> {
    // Resolve the Cargo manifest.
    let manifest = match &cli.manifest_path {
        Some(path) => path.clone(),
        None => std::env::current_dir()?.join("Cargo.toml"),
    };

    if !manifest.is_file() {
        bail!("Cargo manifest not found: {}", manifest.display());
    }

    // Retrieve Cargo metadata from the selected project.
    let mut command = MetadataCommand::new();
    command.manifest_path(&manifest);

    let metadata = command.exec().context("unable to run cargo metadata")?;

    let package = select_package(&metadata, cli.package.as_deref())?;
    let target = select_target(&package, cli)?;

    // Build unless an existing ELF was explicitly supplied or --no-build is set.
    if !cli.no_build && cli.binary.is_none() {
        let mut build = Command::new("cargo");

        build
            .arg("build")
            .arg("--manifest-path")
            .arg(&package.manifest_path);

        if cli.release {
            build.arg("--release");
        }

        if let Some(profile) = &cli.profile {
            build.arg("--profile").arg(profile);
        }

        if let Some(triple) = &cli.target {
            build.arg("--target").arg(triple);
        }

        if let Some(bin) = &cli.bin {
            build.arg("--bin").arg(bin);
        }

        if let Some(example) = &cli.example {
            build.arg("--example").arg(example);
        }

        if !cli.features.is_empty() {
            build.arg("--features").arg(cli.features.join(","));
        }

        if cli.no_default_features {
            build.arg("--no-default-features");
        }

        let status = build.status().context("failed to start cargo build")?;

        if !status.success() {
            bail!("cargo build failed with status {status}");
        }
    }

    // Resolve the artifact path.
    let target_dir = metadata.target_directory.as_std_path();

    let profile = cli
        .profile
        .as_deref()
        .unwrap_or(if cli.release { "release" } else { "debug" });

    let binary = if let Some(path) = &cli.binary {
        path.canonicalize()
            .with_context(|| format!("unable to resolve ELF path: {}", path.display()))?
    } else {
        let mut path = target_dir.to_path_buf();

        if let Some(triple) = &cli.target {
            path.push(triple);
        }

        path.push(profile);
        path.push(target.name.replace('-', "_"));

        if cfg!(windows) {
            path.set_extension("exe");
        }

        path
    };

    if !binary.is_file() {
        bail!(
            "ELF artifact not found at {}. Specify --binary or check the selected Cargo target/profile.",
            binary.display()
        );
    }

    // Resolve the package directory.
    let manifest_dir = package
        .manifest_path
        .parent()
        .context("package manifest has no parent")?
        .as_std_path()
        .to_path_buf();

    // Basic no_std detection.
    let no_std = std::fs::read_to_string(manifest_dir.join("src/lib.rs"))
        .unwrap_or_default()
        .contains("#![no_std]")
        || std::fs::read_to_string(manifest_dir.join("src/main.rs"))
            .unwrap_or_default()
            .contains("#![no_std]");

    Ok(CargoContext {
        workspace_root: metadata.workspace_root.as_std_path().to_path_buf(),
        metadata,
        package,
        target,
        binary,
        manifest_dir,
        no_std,
    })
}

fn select_package(metadata: &Metadata, name: Option<&str>) -> Result<Package> {
    let selected = if let Some(name) = name {
        metadata.packages.iter().find(|p| p.name == name)
    } else {
        metadata
            .root_package()
            .or_else(|| metadata.packages.first())
    };

    selected.cloned().context("no matching Cargo package found")
}

fn select_target(package: &Package, cli: &Cli) -> Result<Target> {
    let target = package.targets.iter().find(|target| {
        if let Some(bin) = &cli.bin {
            target.name == *bin && target.kind.contains(&TargetKind::Bin)
        } else if let Some(example) = &cli.example {
            target.name == *example && target.kind.contains(&TargetKind::Example)
        } else {
            target.kind.contains(&TargetKind::Bin)
        }
    });

    target
        .cloned()
        .context("no matching binary/example target found")
}

pub fn dependency_summary(metadata: &Metadata, package: &Package) -> Vec<String> {
    let mut versions = std::collections::BTreeMap::<String, Vec<String>>::new();

    for dep in &metadata.packages {
        versions
            .entry(dep.name.to_string())
            .or_default()
            .push(dep.version.to_string());
    }

    let mut findings = Vec::new();

    for (name, mut vers) in versions {
        vers.sort();
        vers.dedup();

        if vers.len() > 1 {
            findings.push(format!(
                "duplicate dependency versions: {name} ({})",
                vers.join(", ")
            ));
        }
    }

    if package
        .dependencies
        .iter()
        .any(|d| d.name == "serde" && d.uses_default_features)
    {
        findings.push(
            "serde default features are enabled; review whether std support is required".into(),
        );
    }

    findings
}

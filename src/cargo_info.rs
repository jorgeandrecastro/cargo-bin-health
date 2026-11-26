use anyhow::{Context, Result, bail};
use cargo_metadata::{Metadata, MetadataCommand, Package, Target, TargetKind};
use std::{collections::{BTreeMap, HashSet, VecDeque}, path::PathBuf, process::Command};

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
    let manifest = match &cli.manifest_path {
        Some(path) => path.clone(),
        None => std::env::current_dir()?.join("Cargo.toml"),
    };
    if !manifest.is_file() {
        bail!("Cargo manifest not found: {}", manifest.display());
    }

    let mut command = MetadataCommand::new();
    command.manifest_path(&manifest);
    let metadata = command.exec().context("unable to run cargo metadata")?;

    let package = select_package(&metadata, cli.package.as_deref())?;
    let target = select_target(&package, cli)?;

    if !cli.no_build && cli.binary.is_none() {
        let mut build = Command::new("cargo");
        build.arg("build")
            .arg("--manifest-path")
            .arg(&package.manifest_path);

        if cli.release { build.arg("--release"); }
        if let Some(profile) = &cli.profile { build.arg("--profile").arg(profile); }
        if let Some(triple) = &cli.target { build.arg("--target").arg(triple); }
        if let Some(bin) = &cli.bin { build.arg("--bin").arg(bin); }
        if let Some(example) = &cli.example { build.arg("--example").arg(example); }
        if !cli.features.is_empty() { build.arg("--features").arg(cli.features.join(",")); }
        if cli.no_default_features { build.arg("--no-default-features"); }

        let status = build.status().context("failed to start cargo build")?;
        if !status.success() { bail!("cargo build failed with status {status}"); }
    }

    let profile = cli.profile.as_deref().unwrap_or(if cli.release { "release" } else { "debug" });
    let binary = if let Some(path) = &cli.binary {
        path.canonicalize().with_context(|| format!("unable to resolve ELF path: {}", path.display()))?
    } else {
        let mut path = metadata.target_directory.as_std_path().to_path_buf();
        if let Some(triple) = &cli.target { path.push(triple); }
        path.push(profile);
        path.push(target.name.replace('-', "_"));
        if cfg!(windows) { path.set_extension("exe"); }
        path
    };

    if !binary.is_file() {
        bail!("ELF artifact not found at {}. Specify --binary or check the selected Cargo target/profile.", binary.display());
    }

    let manifest_dir = package.manifest_path.parent()
        .context("package manifest has no parent")?
        .as_std_path().to_path_buf();

    let no_std = source_declares_no_std(&manifest_dir);

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

fn source_declares_no_std(package_dir: &std::path::Path) -> bool {
    ["src/lib.rs", "src/main.rs"].iter().any(|file| {
        std::fs::read_to_string(package_dir.join(file))
            .map(|source| source.lines().any(|line| line.trim() == "#![no_std]"))
            .unwrap_or(false)
    })
}

fn select_package(metadata: &Metadata, name: Option<&str>) -> Result<Package> {
    let selected = if let Some(name) = name {
        metadata.packages.iter().find(|p| p.name == name)
    } else {
        metadata.root_package().or_else(|| metadata.packages.first())
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
    target.cloned().context("no matching binary/example target found")
}

/// Return package IDs reachable from the selected package in Cargo's resolved graph.
/// This avoids reporting duplicate versions from unrelated workspace members.
fn reachable_package_ids(metadata: &Metadata, root: &Package) -> HashSet<String> {
    let Some(resolve) = &metadata.resolve else { return [root.id.to_string()].into_iter().collect(); };
    let mut visited = HashSet::new();
    let mut queue = VecDeque::from([root.id.to_string()]);
    let nodes: BTreeMap<String, Vec<String>> = resolve.nodes.iter().map(|node| {
        (node.id.to_string(), node.dependencies.iter().map(ToString::to_string).collect())
    }).collect();

    while let Some(id) = queue.pop_front() {
        if !visited.insert(id.clone()) { continue; }
        if let Some(dependencies) = nodes.get(&id) {
            queue.extend(dependencies.iter().filter(|dep| !visited.contains(*dep)).cloned());
        }
    }
    visited
}

pub fn dependency_summary(metadata: &Metadata, package: &Package) -> Vec<String> {
    let reachable = reachable_package_ids(metadata, package);
    let mut versions = BTreeMap::<String, Vec<String>>::new();

    for dependency in &metadata.packages {
        if !reachable.contains(&dependency.id.to_string()) { continue; }
        versions.entry(dependency.name.to_string()).or_default().push(dependency.version.to_string());
    }

    let mut findings = Vec::new();
    for (name, mut versions) in versions {
        versions.sort();
        versions.dedup();
        if versions.len() > 1 {
            findings.push(format!("duplicate dependency versions in resolved graph: {name} ({})", versions.join(", ")));
        }
    }

    if package.dependencies.iter().any(|dep| dep.name == "serde" && dep.uses_default_features) {
        findings.push("serde default features are enabled; review whether std support is required".into());
    }
    findings
}

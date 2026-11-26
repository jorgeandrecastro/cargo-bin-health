use crate::{
    cargo_info::{CargoContext, dependency_summary},
    elf::ElfReport,
};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct Diagnostic {
    pub level: String,
    pub message: String,
}

pub fn analyze(context: &CargoContext, elf: &ElfReport) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    for finding in dependency_summary(&context.metadata, &context.package) {
        out.push(Diagnostic {
            level: "warning".into(),
            message: finding,
        });
    }
    if elf.top_symbols.is_empty() {
        out.push(Diagnostic {
            level: "info".into(),
            message: "no function symbols found; the ELF may be stripped or contain no retained function symbol table".into(),
        });
    }
    if context.no_std {
        let has_std = context.metadata.packages.iter().any(|p| p.name == "std");
        if has_std {
            out.push(Diagnostic { level: "warning".into(), message: "no_std crate metadata includes std; inspect target-specific/transitive dependencies".into() });
        }
    }
    let manifest = std::fs::read_to_string(&context.package.manifest_path).unwrap_or_default();
    if !manifest.contains("lto") {
        out.push(Diagnostic {
            level: "info".into(),
            message: "release profile does not explicitly configure LTO".into(),
        });
    }
    if manifest.contains("opt-level = 0") {
        out.push(Diagnostic {
            level: "warning".into(),
            message: "opt-level = 0 detected; unsuitable for a size-optimized firmware release"
                .into(),
        });
    }
    if manifest.contains("panic = \"unwind\"") {
        out.push(Diagnostic {
            level: "warning".into(),
            message: "panic = unwind detected; consider panic = abort for firmware targets".into(),
        });
    }
    if out.is_empty() {
        out.push(Diagnostic {
            level: "ok".into(),
            message: "No configured health rules were triggered.".into(),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn diagnostic_is_serializable() {
        let d = Diagnostic {
            level: "warning".into(),
            message: "example".into(),
        };
        assert!(serde_json::to_string(&d).unwrap().contains("warning"));
    }
}

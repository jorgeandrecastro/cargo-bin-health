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

pub fn analyze(context: &CargoContext, _elf: &ElfReport) -> Vec<Diagnostic> {
    let mut out = Vec::new();

    for finding in dependency_summary(&context.metadata, &context.package) {
        out.push(Diagnostic {
            level: "warning".into(),
            message: finding,
        });
    }

    if context.no_std {
        out.push(Diagnostic {
            level: "info".into(),
            message: "the selected package declares #![no_std]; dependency metadata alone cannot prove that every linked dependency is no_std-compatible".into(),
        });
    }

    let manifest = std::fs::read_to_string(&context.package.manifest_path).unwrap_or_default();
    let release_profile = profile_section(&manifest, "profile.release");

    if release_profile.is_some_and(|section| !has_key(section, "lto")) {
        out.push(Diagnostic {
            level: "info".into(),
            message: "[profile.release] does not explicitly configure LTO".into(),
        });
    }

    if release_profile.is_some_and(|section| key_value(section, "opt-level").is_some_and(|v| v == "0")) {
        out.push(Diagnostic {
            level: "warning".into(),
            message: "[profile.release] opt-level = 0; this is usually unsuitable for size-optimized firmware".into(),
        });
    }

    if release_profile.is_some_and(|section| key_value(section, "panic").is_some_and(|v| v == "\"unwind\"" || v == "unwind")) {
        out.push(Diagnostic {
            level: "warning".into(),
            message: "[profile.release] panic = unwind; consider panic = abort if supported by the target/runtime".into(),
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

fn profile_section<'a>(manifest: &'a str, wanted: &str) -> Option<&'a str> {
    let mut offset = 0usize;
    let mut start = None;

    for line in manifest.split_inclusive('\n') {
        let header = line.split('#').next().unwrap_or("").trim();
        if header.starts_with('[') && header.ends_with(']') {
            if let Some(section_start) = start {
                return Some(&manifest[section_start..offset]);
            }
            if &header[1..header.len() - 1] == wanted {
                start = Some(offset + line.len());
            }
        }
        offset += line.len();
    }

    start.map(|section_start| &manifest[section_start..])
}

fn key_value(section: &str, key: &str) -> Option<String> {
    section.lines().find_map(|line| {
        let line = line.split('#').next().unwrap_or("").trim();
        let (found, value) = line.split_once('=')?;
        if found.trim() == key { Some(value.trim().trim_end_matches(',').trim().to_string()) } else { None }
    })
}

fn has_key(section: &str, key: &str) -> bool {
    key_value(section, key).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_release_profile_without_matching_comments_or_other_sections() {
        let manifest = r#"
# lto = true
[dependencies]
lto = "not-a-profile-setting"

[profile.release]
opt-level = 0
panic = "abort"
"#;
        let section = profile_section(manifest, "profile.release").unwrap();
        assert_eq!(key_value(section, "opt-level").as_deref(), Some("0"));
        assert_eq!(key_value(section, "panic").as_deref(), Some("\"abort\""));
        assert!(!has_key(section, "lto"));
    }
}

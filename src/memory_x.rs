use serde::Serialize;
use std::{fs, path::Path};

#[derive(Debug, Clone, Default, Serialize)]
pub struct MemoryRegions {
    pub flash_bytes: Option<u64>,
    pub ram_bytes: Option<u64>,
    pub source: Option<String>,
}

pub fn parse_memory_x(input: &str) -> MemoryRegions {
    let mut result = MemoryRegions::default();
    for line in input.lines() {
        let upper = line.to_ascii_uppercase();
        let Some((name, rest)) = line.split_once(':') else {
            continue;
        };
        let Some((_, body)) = rest.split_once("ORIGIN") else {
            continue;
        };
        let Some((_, length_part)) = body.split_once("LENGTH") else {
            continue;
        };
        let length = length_part.split(';').next().unwrap_or("").trim();
        let value = parse_size(length);
        if upper.contains("FLASH") {
            result.flash_bytes = value;
        }
        if upper.contains("RAM") {
            result.ram_bytes = value;
        }
        let _ = name;
    }
    result
}

fn parse_size(value: &str) -> Option<u64> {
    let token = value
        .trim()
        .trim_start_matches('=')
        .split_whitespace()
        .next()?
        .trim_end_matches(';');
    let split = token.find(|c: char| !c.is_ascii_digit())?;
    let number: u64 = token[..split].parse().ok()?;
    let unit = token[split..].trim().to_ascii_uppercase();
    let multiplier = match unit.as_str() {
        "K" | "KB" | "KIB" => 1024,
        "M" | "MB" | "MIB" => 1024 * 1024,
        "" => 1,
        _ => return None,
    };
    number.checked_mul(multiplier)
}

pub fn find_and_parse(workspace: &Path, package: &Path) -> Option<MemoryRegions> {
    let mut candidates = vec![
        package.join("memory.x"),
        workspace.join("memory.x"),
        package.join("build/memory.x"),
    ];

    candidates.dedup();

    for path in candidates {
        if path.is_file()
            && let Ok(content) = fs::read_to_string(&path)
        {
            let mut parsed = parse_memory_x(&content);
            parsed.source = Some(path.display().to_string());
            return Some(parsed);
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_linker_regions() {
        let parsed = parse_memory_x(
            "MEMORY { FLASH : ORIGIN = 0x08000000, LENGTH = 512K\n RAM : ORIGIN = 0x20000000, LENGTH = 128K }",
        );
        assert_eq!(parsed.flash_bytes, Some(512 * 1024));
        assert_eq!(parsed.ram_bytes, Some(128 * 1024));
    }
}

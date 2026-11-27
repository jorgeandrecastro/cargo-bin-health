use serde::Serialize;
use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Serialize, Default, PartialEq, Eq)]
pub struct MemoryRegion {
    pub name: String,
    pub origin: Option<u64>,
    pub length: Option<u64>,
    pub end: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct MemoryRegions {
    /// Largest declared Flash-like linker window. This is not necessarily
    /// the size of the physical Flash chip (for example, RP2040 XIP is a window).
    pub flash_bytes: Option<u64>,
    /// Largest declared RAM-like linker window. This is not a target capacity.
    /// Do not use this field for CI thresholds; use the target-aware accounting report.
    pub ram_bytes: Option<u64>,
    pub source: Option<String>,
    pub regions: Vec<MemoryRegion>,
}

fn strip_comments(input: &str) -> String {
    let chars: Vec<char> = input.chars().collect();
    let mut output = String::with_capacity(input.len());
    let mut index = 0usize;
    let mut in_block = false;
    let mut in_line = false;

    while index < chars.len() {
        let current = chars[index];
        let next = chars.get(index + 1).copied();

        if in_block {
            if current == '*' && next == Some('/') {
                in_block = false;
                index += 2;
            } else {
                if current == '\n' {
                    output.push('\n');
                }
                index += 1;
            }
        } else if in_line {
            if current == '\n' {
                in_line = false;
                output.push('\n');
            }
            index += 1;
        } else if current == '/' && next == Some('*') {
            in_block = true;
            index += 2;
        } else if current == '/' && next == Some('/') {
            in_line = true;
            index += 2;
        } else {
            output.push(current);
            index += 1;
        }
    }

    output
}

fn split_top_level(input: &str, delimiter: char) -> Vec<&str> {
    let mut result = Vec::new();
    let mut depth = 0i32;
    let mut start = 0usize;

    for (index, ch) in input.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => depth -= 1,
            _ => {}
        }
        if ch == delimiter && depth == 0 {
            result.push(&input[start..index]);
            start = index + ch.len_utf8();
        }
    }

    result.push(&input[start..]);
    result
}

fn trim_outer_parens(mut expression: &str) -> &str {
    loop {
        expression = expression.trim();
        if !expression.starts_with('(') || !expression.ends_with(')') {
            return expression;
        }

        let mut depth = 0i32;
        let mut encloses_all = true;
        for (index, ch) in expression.char_indices() {
            match ch {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 && index + ch.len_utf8() != expression.len() {
                        encloses_all = false;
                        break;
                    }
                    if depth < 0 {
                        return expression;
                    }
                }
                _ => {}
            }
        }

        if encloses_all && depth == 0 {
            expression = &expression[1..expression.len() - 1];
        } else {
            return expression;
        }
    }
}

/// Evaluates the common integer expressions used in GNU ld MEMORY declarations.
/// Supports decimal/hex values, K/M suffixes, addition, subtraction and parentheses.
fn eval_num(input: &str) -> Option<u64> {
    let expression = trim_outer_parens(input.trim());
    if expression.is_empty() {
        return None;
    }

    let mut depth = 0i32;
    for (index, ch) in expression.char_indices().rev() {
        match ch {
            ')' => depth += 1,
            '(' => depth -= 1,
            '+' | '-' if depth == 0 && index > 0 => {
                let left = eval_num(&expression[..index])?;
                let right = eval_num(&expression[index + 1..])?;
                return if ch == '+' {
                    left.checked_add(right)
                } else {
                    left.checked_sub(right)
                };
            }
            _ => {}
        }
    }

    let compact: String = expression
        .chars()
        .filter(|ch| !ch.is_whitespace() && *ch != '_')
        .collect();
    let (number, multiplier) = match compact.chars().last()? {
        'K' | 'k' => (&compact[..compact.len() - 1], 1024u64),
        'M' | 'm' => (&compact[..compact.len() - 1], 1024u64 * 1024),
        _ => (compact.as_str(), 1u64),
    };

    let value = if let Some(hex) = number.strip_prefix("0x").or_else(|| number.strip_prefix("0X")) {
        u64::from_str_radix(hex, 16).ok()?
    } else {
        number.parse::<u64>().ok()?
    };
    value.checked_mul(multiplier)
}

fn resolve_region_functions(input: &str, regions: &[MemoryRegion]) -> Option<String> {
    let mut expression = input.to_string();

    loop {
        let upper = expression.to_ascii_uppercase();
        let origin_pos = upper.find("ORIGIN(");
        let length_pos = upper.find("LENGTH(");
        let (position, function) = match (origin_pos, length_pos) {
            (Some(left), Some(right)) if left <= right => (left, "ORIGIN"),
            (Some(_), Some(right)) => (right, "LENGTH"),
            (Some(position), None) => (position, "ORIGIN"),
            (None, Some(position)) => (position, "LENGTH"),
            (None, None) => break,
        };

        let open = position + function.len();
        if expression.as_bytes().get(open) != Some(&b'(') {
            return None;
        }
        let mut depth = 0i32;
        let mut close = None;
        for (offset, ch) in expression[open..].char_indices() {
            match ch {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        close = Some(open + offset);
                        break;
                    }
                }
                _ => {}
            }
        }
        let close = close?;
        let region_name = expression[open + 1..close].trim();
        let region = regions
            .iter()
            .find(|region| region.name.eq_ignore_ascii_case(region_name))?;
        let value = if function == "ORIGIN" {
            region.origin?
        } else {
            region.length?
        };
        expression.replace_range(position..=close, &value.to_string());
    }

    Some(expression)
}

fn parse_assignment(body: &str, key: &str, regions: &[MemoryRegion]) -> Option<u64> {
    for item in split_top_level(body, ',') {
        let Some((name, value)) = item.split_once('=') else {
            continue;
        };
        if !name.trim().eq_ignore_ascii_case(key) {
            continue;
        }
        let resolved = resolve_region_functions(value.trim().trim_end_matches(';'), regions)?;
        return eval_num(&resolved);
    }
    None
}

fn find_matching_brace(input: &str, opening: usize) -> Option<usize> {
    let mut depth = 0i32;
    for (offset, ch) in input[opening..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(opening + offset);
                }
            }
            _ => {}
        }
    }
    None
}

fn split_declarations(input: &str) -> Vec<&str> {
    let mut result = Vec::new();
    let mut depth = 0i32;
    let mut start = 0usize;

    for (index, ch) in input.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => depth -= 1,
            _ => {}
        }
        if depth == 0 && (ch == '\n' || ch == ';') {
            let declaration = input[start..index].trim();
            if !declaration.is_empty() {
                result.push(declaration);
            }
            start = index + ch.len_utf8();
        }
    }
    let remaining = input[start..].trim();
    if !remaining.is_empty() {
        result.push(remaining);
    }
    result
}

fn is_flash_name(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    upper.contains("FLASH") || matches!(upper.as_str(), "IROM" | "DROM" | "ROM")
}

fn is_ram_name(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    upper.contains("RAM")
        || matches!(upper.as_str(), "SRAM" | "IRAM" | "DRAM" | "RTC_FAST" | "RTC_SLOW" | "DTCM" | "ITCM" | "CCM")
}

pub fn parse_memory_x(input: &str) -> MemoryRegions {
    let cleaned = strip_comments(input);
    let upper = cleaned.to_ascii_uppercase();
    let Some(memory_pos) = upper.find("MEMORY") else {
        return MemoryRegions::default();
    };
    let after_memory = &cleaned[memory_pos + "MEMORY".len()..];
    let Some(open_relative) = after_memory.find('{') else {
        return MemoryRegions::default();
    };
    let opening = memory_pos + "MEMORY".len() + open_relative;
    let Some(closing) = find_matching_brace(&cleaned, opening) else {
        return MemoryRegions::default();
    };
    let body = &cleaned[opening + 1..closing];

    let mut result = MemoryRegions::default();
    for declaration in split_declarations(body) {
        let Some((raw_name, attributes)) = declaration.split_once(':') else {
            continue;
        };
        let name = raw_name.split_whitespace().next().unwrap_or("").trim();
        if name.is_empty() {
            continue;
        }
        let origin = parse_assignment(attributes, "ORIGIN", &result.regions);
        let length = parse_assignment(attributes, "LENGTH", &result.regions)
            .or_else(|| parse_assignment(attributes, "len", &result.regions));
        let end = origin.zip(length).and_then(|(origin, length)| origin.checked_add(length));

        if let Some(length) = length {
            if is_flash_name(name) {
                result.flash_bytes = Some(result.flash_bytes.unwrap_or(0).max(length));
            }
            if is_ram_name(name) {
                result.ram_bytes = Some(result.ram_bytes.unwrap_or(0).max(length));
            }
        }

        result.regions.push(MemoryRegion {
            name: name.to_string(),
            origin,
            length,
            end,
        });
    }
    result
}

fn collect_memory_scripts(directory: &Path, depth: usize, output: &mut Vec<PathBuf>) {
    if depth == 0 {
        return;
    }
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    let mut entries: Vec<_> = entries.flatten().collect();
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        if path.is_file() && path.file_name().is_some_and(|name| name == "memory.x") {
            output.push(path);
        } else if path.is_dir() {
            collect_memory_scripts(&path, depth - 1, output);
        }
    }
}

fn path_has_component(path: &Path, component: &std::ffi::OsStr) -> bool {
    path.components().any(|part| part.as_os_str() == component)
}

/// Finds a linker memory script for the inspected artifact. Generated scripts
/// are filtered to the binary's profile and, when possible, target triple.
pub fn find_and_parse(
    workspace: &Path,
    package: &Path,
    target_dir: &Path,
    binary: &Path,
) -> Option<MemoryRegions> {
    let binary_parent = binary.parent()?;
    let profile = binary_parent.file_name()?;
    let triple = binary_parent.parent().and_then(Path::file_name);

    let mut generated = Vec::new();
    collect_memory_scripts(target_dir, 8, &mut generated);
    let workspace_target = workspace.join("target");
    if target_dir != workspace_target {
        collect_memory_scripts(&workspace_target, 8, &mut generated);
    }
    let package_target = package.join("target");
    if package != workspace && target_dir != package_target {
        collect_memory_scripts(&package_target, 8, &mut generated);
    }

    generated.retain(|path| {
        path_has_component(path, std::ffi::OsStr::new("build"))
            && path_has_component(path, std::ffi::OsStr::new("out"))
            && path_has_component(path, profile)
    });
    generated.sort_by_key(|path| {
        !(triple.is_some_and(|value| path_has_component(path, value)))
    });

    let mut candidates = generated;
    candidates.extend([
        package.join("memory.x"),
        workspace.join("memory.x"),
        package.join("build/memory.x"),
    ]);

    let mut seen = HashSet::new();
    for path in candidates {
        let canonical = fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
        if !seen.insert(canonical) || !path.is_file() {
            continue;
        }
        let Ok(content) = fs::read_to_string(&path) else {
            continue;
        };
        let mut parsed = parse_memory_x(&content);
        if !parsed.regions.is_empty() {
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
    fn parses_flash_and_ram() {
        let parsed = parse_memory_x(
            "MEMORY {\nFLASH : ORIGIN = 0x08000000, LENGTH = 1M\nRAM : ORIGIN = 0x20000000, LENGTH = 256K\n}",
        );
        assert_eq!(parsed.flash_bytes, Some(1024 * 1024));
        assert_eq!(parsed.ram_bytes, Some(256 * 1024));
    }

    #[test]
    fn parses_esp32c3_symbolic_expressions() {
        let parsed = parse_memory_x(
            "MEMORY {\nICACHE : ORIGIN = 0x4037C000, LENGTH = 0x4000\nIRAM : ORIGIN = 0x4037C000 + 0x4000, LENGTH = 313K - 0x4000\nDRAM : ORIGIN = 0x3FC80000, LENGTH = 313K\ndram2_seg ( RW ) : ORIGIN = ORIGIN(DRAM) + LENGTH(DRAM), len = 0x3fcde710 - (ORIGIN(DRAM) + LENGTH(DRAM))\n}",
        );
        assert_eq!(parsed.regions.len(), 4);
        assert_eq!(parsed.regions[1].origin, Some(0x4038_0000));
        assert_eq!(parsed.regions[1].length, Some(304_128));
        assert_eq!(parsed.regions[3].origin, Some(0x3fcc_e400));
        assert_eq!(parsed.regions[3].length, Some(66_320));
    }

    #[test]
    fn ignores_comments_and_supports_nested_expressions() {
        let parsed = parse_memory_x(
            "/* MEMORY { BAD : ORIGIN=0, LENGTH=1K } */\nMEMORY { RAM : ORIGIN=0x20000000, LENGTH=256K\nEXTRA (RW) : ORIGIN=(ORIGIN(RAM) + LENGTH(RAM)), len=(512K - (128K + 64K))\n}",
        );
        assert_eq!(parsed.regions.len(), 2);
        assert_eq!(parsed.regions[1].origin, Some(0x2004_0000));
        assert_eq!(parsed.regions[1].length, Some(320 * 1024));
    }

    #[test]
    fn selects_memory_script_for_binary_profile() {
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("target");
        let triple = "riscv32imc-unknown-none-elf";
        let debug = target.join(triple).join("debug/build/hal-debug/out");
        let release = target.join(triple).join("release/build/hal-release/out");
        fs::create_dir_all(&debug).unwrap();
        fs::create_dir_all(&release).unwrap();
        fs::write(debug.join("memory.x"), "MEMORY { RAM : ORIGIN=0x2000, LENGTH=4K }").unwrap();
        fs::write(release.join("memory.x"), "MEMORY { RAM : ORIGIN=0x2000, LENGTH=8K }").unwrap();

        let binary = target.join(triple).join("release/firmware");
        let found = find_and_parse(temp.path(), temp.path(), &target, &binary).unwrap();
        assert!(found.source.unwrap().contains("release/build/hal-release/out/memory.x"));
        assert_eq!(found.ram_bytes, Some(8 * 1024));
    }

    #[test]
    fn selects_debug_memory_script_for_debug_binary() {
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("target");
        let triple = "riscv32imc-unknown-none-elf";
        let debug = target.join(triple).join("debug/build/hal-debug/out");
        let release = target.join(triple).join("release/build/hal-release/out");
        fs::create_dir_all(&debug).unwrap();
        fs::create_dir_all(&release).unwrap();
        fs::write(debug.join("memory.x"), "MEMORY { RAM : ORIGIN=0x2000, LENGTH=4K }").unwrap();
        fs::write(release.join("memory.x"), "MEMORY { RAM : ORIGIN=0x2000, LENGTH=8K }").unwrap();

        let binary = target.join(triple).join("debug/firmware");
        let found = find_and_parse(temp.path(), temp.path(), &target, &binary).unwrap();
        assert!(found.source.unwrap().contains("debug/build/hal-debug/out/memory.x"));
        assert_eq!(found.ram_bytes, Some(4 * 1024));
    }
}

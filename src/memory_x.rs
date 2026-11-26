
use serde::Serialize;
use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Serialize, Default)]
pub struct MemoryRegion {
    pub name: String,
    pub origin: Option<u64>,
    pub length: Option<u64>,
    pub end: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct MemoryRegions {
    /// Largest declared Flash-mapped linker window.
    ///
    /// This is NOT necessarily the physical Flash capacity.
    /// IROM and DROM can map the same physical Flash.
    pub flash_bytes: Option<u64>,

    /// Largest declared RAM-like linker window.
    ///
    /// This is NOT necessarily the total physical RAM capacity.
    pub ram_bytes: Option<u64>,

    pub source: Option<String>,
    pub regions: Vec<MemoryRegion>,
}

/// Removes C-style and C++-style comments while preserving newlines.
fn strip_comments(input: &str) -> String {
    let chars: Vec<char> = input.chars().collect();
    let mut output = String::with_capacity(input.len());

    let mut i = 0;
    let mut block_comment = false;
    let mut line_comment = false;

    while i < chars.len() {
        let current = chars[i];
        let next = chars.get(i + 1).copied();

        if block_comment {
            if current == '*' && next == Some('/') {
                block_comment = false;
                i += 2;
                continue;
            }

            if current == '\n' {
                output.push('\n');
            }

            i += 1;
            continue;
        }

        if line_comment {
            if current == '\n' {
                line_comment = false;
                output.push('\n');
            }

            i += 1;
            continue;
        }

        if current == '/' && next == Some('*') {
            block_comment = true;
            i += 2;
            continue;
        }

        if current == '/' && next == Some('/') {
            line_comment = true;
            i += 2;
            continue;
        }

        output.push(current);
        i += 1;
    }

    output
}

/// Splits a string at a delimiter only when outside parentheses.
fn split_top_level(input: &str, delimiter: char) -> Vec<&str> {
    let mut result = Vec::new();
    let mut depth = 0i32;
    let mut start = 0;

    for (i, ch) in input.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => depth -= 1,
            _ => {}
        }

        if ch == delimiter && depth == 0 {
            result.push(&input[start..i]);
            start = i + ch.len_utf8();
        }
    }

    result.push(&input[start..]);
    result
}

/// Removes outer parentheses only when they enclose the entire expression.
fn trim_outer_parens(mut expression: &str) -> &str {
    loop {
        expression = expression.trim();

        if !expression.starts_with('(') || !expression.ends_with(')') {
            return expression;
        }

        let mut depth = 0i32;
        let mut encloses_all = true;

        for (i, ch) in expression.char_indices() {
            match ch {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;

                    if depth == 0 && i + ch.len_utf8() != expression.len() {
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

/// Evaluates simple GNU linker-script numeric expressions.
///
/// Supported:
/// - Decimal numbers
/// - Hexadecimal numbers
/// - K / M suffixes
/// - Addition
/// - Subtraction
/// - Nested parentheses
fn eval_num(input: &str) -> Option<u64> {
    let expression = trim_outer_parens(input.trim());

    if expression.is_empty() {
        return None;
    }

    let mut depth = 0i32;

    // Evaluate operators from right to left.
    // This preserves left associativity.
    for (i, ch) in expression.char_indices().rev() {
        match ch {
            ')' => depth += 1,
            '(' => depth -= 1,
            '+' | '-' if depth == 0 && i > 0 => {
                let left = eval_num(&expression[..i])?;
                let right = eval_num(&expression[i + 1..])?;

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

    if compact.is_empty() {
        return None;
    }

    let (number, multiplier) = match compact.chars().last()? {
        'K' | 'k' => (&compact[..compact.len() - 1], 1024u64),
        'M' | 'm' => (&compact[..compact.len() - 1], 1024u64 * 1024),
        _ => (compact.as_str(), 1u64),
    };

    let value = if let Some(hex) = number
        .strip_prefix("0x")
        .or_else(|| number.strip_prefix("0X"))
    {
        u64::from_str_radix(hex, 16).ok()?
    } else {
        number.parse::<u64>().ok()?
    };

    value.checked_mul(multiplier)
}

/// Resolves ORIGIN(NAME) and LENGTH(NAME) expressions.
///
/// Previously parsed regions are used for resolution.
fn resolve_region_functions(
    input: &str,
    regions: &[MemoryRegion],
) -> Option<String> {
    let mut expression = input.to_string();

    loop {
        let upper = expression.to_ascii_uppercase();

        let origin_pos = upper.find("ORIGIN(");
        let length_pos = upper.find("LENGTH(");

        let (position, function) = match (origin_pos, length_pos) {
            (Some(a), Some(b)) if a <= b => (a, "ORIGIN"),
            (Some(_), Some(b)) => (b, "LENGTH"),
            (Some(a), None) => (a, "ORIGIN"),
            (None, Some(b)) => (b, "LENGTH"),
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

/// Parses one linker-script assignment.
fn parse_assignment(
    body: &str,
    key: &str,
    regions: &[MemoryRegion],
) -> Option<u64> {
    for item in split_top_level(body, ',') {
        let Some((name, value)) = item.split_once('=') else {
            continue;
        };

        if !name.trim().eq_ignore_ascii_case(key) {
            continue;
        }

        let value = value.trim().trim_end_matches(';').trim();
        let resolved = resolve_region_functions(value, regions)?;

        return eval_num(&resolved);
    }

    None
}

/// Finds the matching closing brace.
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

/// Splits MEMORY declarations at newlines or semicolons,
/// without splitting expressions inside parentheses.
fn split_declarations(input: &str) -> Vec<&str> {
    let mut result = Vec::new();
    let mut depth = 0i32;
    let mut start = 0;

    for (i, ch) in input.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => depth -= 1,
            _ => {}
        }

        if depth == 0 && (ch == '\n' || ch == ';') {
            let item = input[start..i].trim();

            if !item.is_empty() {
                result.push(item);
            }

            start = i + ch.len_utf8();
        }
    }

    let remaining = input[start..].trim();

    if !remaining.is_empty() {
        result.push(remaining);
    }

    result
}

/// Determines whether a linker region represents a Flash mapping.
///
/// ICACHE is deliberately excluded: it is not a Flash capacity window.
fn is_flash_region(name: &str) -> bool {
    matches!(
        name.to_ascii_uppercase().as_str(),
        "FLASH" | "IROM" | "DROM"
    )
}

/// Determines whether a linker region represents RAM.
fn is_ram_region(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();

    upper.contains("RAM")
        || matches!(
            upper.as_str(),
            "IRAM"
                | "DRAM"
                | "RTC_FAST"
                | "RTC_SLOW"
                | "DRAM2_SEG"
        )
}

/// Parses a GNU linker memory.x script.
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

    let open = memory_pos + "MEMORY".len() + open_relative;

    let Some(close) = find_matching_brace(&cleaned, open) else {
        return MemoryRegions::default();
    };

    let body = &cleaned[open + 1..close];
    let mut result = MemoryRegions::default();

    for declaration in split_declarations(body) {
        let Some((name, attributes)) = declaration.split_once(':') else {
            continue;
        };

        // Handles declarations such as:
        // dram2_seg ( RW ) : ORIGIN = ..., len = ...
        let name = name
            .split_whitespace()
            .next()
            .unwrap_or("")
            .trim();

        if name.is_empty() {
            continue;
        }

        let origin = parse_assignment(attributes, "ORIGIN", &result.regions);

        let length = parse_assignment(attributes, "LENGTH", &result.regions)
            .or_else(|| parse_assignment(attributes, "len", &result.regions));

        let end = origin
            .zip(length)
            .and_then(|(origin, length)| origin.checked_add(length));

        if let Some(length) = length {
            if is_flash_region(name) {
                result.flash_bytes = Some(
                    result.flash_bytes
                        .unwrap_or(0)
                        .max(length),
                );
            }

            if is_ram_region(name) {
                result.ram_bytes = Some(
                    result.ram_bytes
                        .unwrap_or(0)
                        .max(length),
                );
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

/// Recursively collects generated memory.x files.
fn collect_memory_scripts(
    directory: &Path,
    depth: usize,
    output: &mut Vec<PathBuf>,
) {
    if depth == 0 {
        return;
    }

    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };

    // Deterministic traversal.
    let mut entries: Vec<_> = entries.flatten().collect();
    entries.sort_by_key(|entry| entry.file_name());

    for entry in entries {
        let path = entry.path();

        if path.is_file()
            && path.file_name().is_some_and(|name| name == "memory.x")
        {
            output.push(path);
            continue;
        }

        if path.is_dir() {
            collect_memory_scripts(&path, depth - 1, output);
        }
    }
}

/// Returns true for Cargo-generated build-script output directories.
///
/// Typical path:
/// target/<profile>/build/<crate>-<hash>/out/memory.x
fn is_cargo_generated_script(path: &Path) -> bool {
    let components: Vec<_> = path
        .components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect();

    components
        .windows(3)
        .any(|window| window[0] == "build" && window[2] == "out")
}

/// Checks whether a generated script belongs to the requested profile.
fn belongs_to_profile(path: &Path, profile: &std::ffi::OsStr) -> bool {
    path.components()
        .any(|component| component.as_os_str() == profile)
}

/// Searches for a usable memory.x and parses it.
///
/// Generated scripts are restricted to the profile of the inspected ELF.
/// Static project-level scripts remain valid fallbacks.
pub fn find_and_parse(
    workspace: &Path,
    package: &Path,
    target_dir: &Path,
    binary: &Path,
) -> Option<MemoryRegions> {
    let profile = binary.parent()?.file_name()?;

    let mut candidates = vec![
        package.join("memory.x"),
        workspace.join("memory.x"),
        package.join("build/memory.x"),
        target_dir.join("memory.x"),
        target_dir.join("build/memory.x"),
    ];

    collect_memory_scripts(target_dir, 6, &mut candidates);

    let workspace_target = workspace.join("target");

    if target_dir != workspace_target {
        collect_memory_scripts(
            &workspace_target,
            6,
            &mut candidates,
        );
    }

    if package != workspace {
        collect_memory_scripts(
            &package.join("target"),
            6,
            &mut candidates,
        );
    }

    let mut seen = HashSet::new();

    candidates.retain(|path| {
        if !seen.insert(path.clone()) || !path.is_file() {
            return false;
        }

        // Generated scripts must match the ELF profile.
        // Static scripts are not tied to a Cargo profile.
        !is_cargo_generated_script(path) || belongs_to_profile(path, profile)
    });

    // Prefer profile-specific generated scripts.
    // Static project-level scripts are fallbacks.
    candidates.sort_by_key(|path| !is_cargo_generated_script(path));

    for path in candidates {
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
        let memory = parse_memory_x(
            "MEMORY {\n\
             FLASH : ORIGIN = 0x08000000, LENGTH = 1M\n\
             RAM : ORIGIN = 0x20000000, LENGTH = 256K\n\
             }",
        );

        assert_eq!(memory.flash_bytes, Some(1024 * 1024));
        assert_eq!(memory.ram_bytes, Some(256 * 1024));
    }

    #[test]
    fn parses_esp32c3_expressions_and_len() {
        let memory = parse_memory_x(
            "MEMORY\n{\n\
             ICACHE : ORIGIN = 0x4037C000, LENGTH = 0x4000\n\
             IRAM : ORIGIN = 0x4037C000 + 0x4000, LENGTH = 313K - 0x4000\n\
             DRAM : ORIGIN = 0x3FC80000, LENGTH = 313K\n\
             dram2_seg ( RW ) : ORIGIN = ORIGIN(DRAM) + LENGTH(DRAM), len = 0x3fcde710 - (ORIGIN(DRAM) + LENGTH(DRAM))\n\
             }",
        );

        assert_eq!(memory.regions.len(), 4);
        assert_eq!(memory.regions[1].origin, Some(0x40380000));
        assert_eq!(
            memory.regions[1].length,
            Some(313 * 1024 - 0x4000)
        );
        assert_eq!(memory.regions[3].origin, Some(0x3fcce400));
        assert_eq!(memory.regions[3].length, Some(0x10310));

        // ICACHE must not be reported as Flash.
        assert_eq!(memory.flash_bytes, None);

        // DRAM and dram2_seg are both RAM-like.
        assert_eq!(
            memory.ram_bytes,
            Some(313 * 1024)
        );
    }

    #[test]
    fn ignores_comments() {
        let memory = parse_memory_x(
            "/* MEMORY { BAD : ORIGIN=0, LENGTH=1K } */\n\
             MEMORY { FLASH : ORIGIN=0x1000, LENGTH=64K }",
        );

        assert_eq!(memory.regions.len(), 1);
        assert_eq!(memory.flash_bytes, Some(64 * 1024));
    }

    #[test]
    fn parses_assignments_with_attributes() {
        let memory = parse_memory_x(
            "MEMORY {\n\
             IRAM (rwx) : ORIGIN = 0x40380000, FLAGS = 5, LENGTH = 128K\n\
             }",
        );

        assert_eq!(memory.regions.len(), 1);
        assert_eq!(memory.regions[0].length, Some(128 * 1024));
    }

    #[test]
    fn resolves_nested_parentheses() {
        let memory = parse_memory_x(
            "MEMORY {\n\
             RAM : ORIGIN = 0x20000000, LENGTH = 256K\n\
             EXTRA (RW) : ORIGIN = (ORIGIN(RAM) + LENGTH(RAM)), len = (512K - (128K + 64K))\n\
             }",
        );

        assert_eq!(memory.regions[1].origin, Some(0x20040000));
        assert_eq!(memory.regions[1].length, Some(320 * 1024));
    }

    #[test]
    fn does_not_sum_irom_and_drom() {
        let memory = parse_memory_x(
            "MEMORY {\n\
             IROM : ORIGIN = 0x42000020, LENGTH = 4M\n\
             DROM : ORIGIN = 0x3C000020, LENGTH = 4M\n\
             }",
        );

        assert_eq!(memory.flash_bytes, Some(4 * 1024 * 1024));
    }

    #[test]
    fn supports_multiple_declarations_on_one_line() {
        let memory = parse_memory_x(
            "MEMORY { FLASH : ORIGIN = 0x1000, LENGTH = 1M; RAM : ORIGIN = 0x2000, LENGTH = 64K; }",
        );

        assert_eq!(memory.regions.len(), 2);
        assert_eq!(memory.flash_bytes, Some(1024 * 1024));
        assert_eq!(memory.ram_bytes, Some(64 * 1024));
    }

    #[test]
    fn rejects_invalid_arithmetic() {
        assert_eq!(eval_num("10 - 20"), None);
        assert_eq!(eval_num("0x1000 + 2K"), Some(0x1800));
    }

    #[test]
    fn classifies_dram2_segment_as_ram() {
        let memory = parse_memory_x(
            "MEMORY {\n\
             DRAM : ORIGIN = 0x3FC80000, LENGTH = 313K\n\
             dram2_seg ( RW ) : ORIGIN = 0x3fcce400, len = 0x10310\n\
             }",
        );

        assert_eq!(memory.regions.len(), 2);
        assert_eq!(memory.ram_bytes, Some(313 * 1024));
    }

    #[test]
    fn does_not_classify_icache_as_flash() {
        let memory = parse_memory_x(
            "MEMORY {\n\
             ICACHE : ORIGIN = 0x4037C000, LENGTH = 16K\n\
             }",
        );

        assert_eq!(memory.flash_bytes, None);
    }
}

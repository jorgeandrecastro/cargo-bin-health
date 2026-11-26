use serde::Serialize;
use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Serialize, Default)]
pub struct MemoryRegion {
    pub name: String,
    /// `ram`, `flash`, `cache`, or `other`.
    pub kind: String,
    pub origin: Option<u64>,
    pub length: Option<u64>,
    pub end: Option<u64>,
    /// Physical allocation group when linker windows are known aliases.
    pub alias_group: Option<String>,
    /// Offset of this region in its alias group's canonical address space.
    pub alias_offset: u64,
    /// Unique VMA bytes occupied by allocated ELF sections in this region.
    pub used_bytes: u64,
    pub utilization_pct: Option<f64>,
    pub load_bytes: u64,
    #[serde(skip)]
    used_intervals: Vec<(u64, u64)>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct MemoryGroup {
    pub name: String,
    pub kind: String,
    pub regions: Vec<String>,
    pub capacity_bytes: u64,
    pub used_bytes: u64,
    pub utilization_pct: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct MemoryRegions {
    /// Largest Flash-mapped linker window. IROM/DROM are treated as aliases
    /// of the same external Flash capacity and are not added together.
    pub flash_bytes: Option<u64>,
    /// Total linker-derived RAM capacity after applying recognized target aliases.
    /// For ESP32-C3, IRAM/DRAM/dram2_seg are one SRAM pool; RTC_FAST is separate.
    /// This is not a raw silicon-capacity measurement.
    pub ram_bytes: Option<u64>,
    /// Unique RAM bytes used across the recognized physical allocation groups.
    pub ram_used_bytes: u64,
    pub flash_used_bytes: u64,
    /// RAM bytes that could not be assigned to any declared RAM region.
    pub unmapped_ram_bytes: u64,
    /// Maximum utilization of any physical RAM group. None when accounting is incomplete.
    pub ram_worst_group_pct: Option<f64>,
    pub flash_capacity_policy: String,
    pub ram_capacity_policy: String,
    pub source: Option<String>,
    pub regions: Vec<MemoryRegion>,
    pub ram_groups: Vec<MemoryGroup>,
}

impl MemoryRegion {
    pub fn contains(&self, address: u64, size: u64) -> bool {
        let (Some(start), Some(end)) = (self.origin, self.end) else {
            return false;
        };
        let Some(section_end) = address.checked_add(size) else {
            return false;
        };
        address >= start && section_end <= end
    }

    fn record_interval(&mut self, address: u64, size: u64, file_size: u64) {
        if size == 0 {
            return;
        }
        if let Some(end) = address.checked_add(size) {
            self.used_intervals.push((address, end));
        }
        self.load_bytes = self.load_bytes.saturating_add(file_size);
        self.used_bytes = interval_union_len(&self.used_intervals);
        self.utilization_pct = self.length.filter(|length| *length > 0).map(|length| {
            self.used_bytes as f64 * 100.0 / length as f64
        });
    }
}

impl MemoryRegions {
    /// Finds the most specific region containing the complete section VMA range.
    pub fn find_region_index(&self, address: u64, size: u64) -> Option<usize> {
        self.regions
            .iter()
            .enumerate()
            .filter(|(_, region)| region.contains(address, size))
            .min_by_key(|(_, region)| region.length.unwrap_or(u64::MAX))
            .map(|(index, _)| index)
    }

    /// Records a section in its selected linker region.
    pub fn record_section(
        &mut self,
        region_index: usize,
        address: u64,
        size: u64,
        file_size: u64,
    ) {
        if let Some(region) = self.regions.get_mut(region_index) {
            region.record_interval(address, size, file_size);
        }
    }

    /// Rebuilds per-region and physical-group utilization after ELF mapping.
    pub fn finalize_usage(&mut self, flash_used: u64, unmapped_ram: u64) {
        self.flash_used_bytes = flash_used;
        self.unmapped_ram_bytes = unmapped_ram;

        for region in &mut self.regions {
            region.used_bytes = interval_union_len(&region.used_intervals);
            region.utilization_pct = region.length.filter(|length| *length > 0).map(|length| {
                region.used_bytes as f64 * 100.0 / length as f64
            });
        }

        self.ram_groups = build_ram_groups(&self.regions, true);
        self.ram_used_bytes = self.ram_groups.iter().fold(0u64, |sum, group| {
            sum.saturating_add(group.used_bytes)
        });
        self.ram_worst_group_pct = if unmapped_ram == 0 {
            self.ram_groups
                .iter()
                .filter_map(|group| group.utilization_pct)
                .reduce(f64::max)
        } else {
            None
        };
    }
}

fn interval_union_len(intervals: &[(u64, u64)]) -> u64 {
    if intervals.is_empty() {
        return 0;
    }

    let mut sorted = intervals.to_vec();
    sorted.sort_unstable_by_key(|(start, end)| (*start, *end));

    let mut total = 0u64;
    let mut current = sorted[0];

    for &(start, end) in sorted.iter().skip(1) {
        if start <= current.1 {
            current.1 = current.1.max(end);
        } else {
            total = total.saturating_add(current.1.saturating_sub(current.0));
            current = (start, end);
        }
    }

    total.saturating_add(current.1.saturating_sub(current.0))
}

/// Removes C-style and C++-style comments while preserving newlines and UTF-8.
fn strip_comments(input: &str) -> String {
    let chars: Vec<char> = input.chars().collect();
    let mut output = String::with_capacity(input.len());
    let mut index = 0usize;
    let mut block_comment = false;
    let mut line_comment = false;

    while index < chars.len() {
        let current = chars[index];
        let next = chars.get(index + 1).copied();

        if block_comment {
            if current == '*' && next == Some('/') {
                block_comment = false;
                index += 2;
            } else {
                if current == '\n' {
                    output.push('\n');
                }
                index += 1;
            }
        } else if line_comment {
            if current == '\n' {
                line_comment = false;
                output.push('\n');
            }
            index += 1;
        } else if current == '/' && next == Some('*') {
            block_comment = true;
            index += 2;
        } else if current == '/' && next == Some('/') {
            line_comment = true;
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

/// Evaluates the integer arithmetic used by common GNU linker MEMORY declarations.
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
        let value = if function == "ORIGIN" { region.origin? } else { region.length? };
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
            let item = input[start..index].trim();
            if !item.is_empty() {
                result.push(item);
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

fn region_kind(name: &str) -> &'static str {
    let upper = name.to_ascii_uppercase();

    if upper == "ICACHE" || upper.contains("CACHE") {
        "cache"
    } else if matches!(upper.as_str(), "IROM" | "DROM") || upper.contains("FLASH") {
        "flash"
    } else if upper.contains("RAM")
        || matches!(upper.as_str(), "IRAM" | "DRAM" | "RTC_FAST" | "RTC_SLOW")
    {
        "ram"
    } else {
        "other"
    }
}

fn is_esp32c3_memory_layout(regions: &[MemoryRegion]) -> bool {
    let has = |name: &str| regions.iter().any(|region| region.name.eq_ignore_ascii_case(name));
    has("ICACHE") && has("IRAM") && has("DRAM") && has("IROM") && has("DROM")
}

/// ESP32-C3 maps the same internal SRAM through instruction (IRAM) and data
/// (DRAM) address windows. DRAM plus dram2_seg defines the data-side capacity;
/// IRAM is an alias view into that same pool, not additional physical SRAM.
fn apply_known_alias_profiles(regions: &mut [MemoryRegion]) {
    if !is_esp32c3_memory_layout(regions) {
        return;
    }

    let dram_origin = regions
        .iter()
        .find(|region| region.name.eq_ignore_ascii_case("DRAM"))
        .and_then(|region| region.origin);
    let Some(dram_origin) = dram_origin else {
        return;
    };

    for region in regions {
        let upper = region.name.to_ascii_uppercase();
        if upper == "IRAM" || upper == "DRAM" {
            region.alias_group = Some("esp32c3_internal_sram".into());
            region.alias_offset = 0;
        } else if upper == "DRAM2_SEG" {
            if let Some(origin) = region.origin {
                if let Some(offset) = origin.checked_sub(dram_origin) {
                    region.alias_group = Some("esp32c3_internal_sram".into());
                    region.alias_offset = offset;
                }
            }
        }
    }
}

fn build_ram_groups(regions: &[MemoryRegion], include_usage: bool) -> Vec<MemoryGroup> {
    use std::collections::BTreeMap;

    struct Accumulator {
        regions: Vec<String>,
        capacity_intervals: Vec<(u64, u64)>,
        used_intervals: Vec<(u64, u64)>,
    }

    let mut alias_groups = BTreeMap::<String, Accumulator>::new();
    let mut ordinary_regions: Vec<&MemoryRegion> = Vec::new();

    for region in regions.iter().filter(|region| region.kind == "ram") {
        if let Some(alias_group) = &region.alias_group {
            let entry = alias_groups
                .entry(alias_group.clone())
                .or_insert_with(|| Accumulator {
                    regions: Vec::new(),
                    capacity_intervals: Vec::new(),
                    used_intervals: Vec::new(),
                });
            entry.regions.push(region.name.clone());

            if let Some(length) = region.length {
                if let Some(end) = region.alias_offset.checked_add(length) {
                    entry.capacity_intervals.push((region.alias_offset, end));
                }
            }

            if include_usage {
                if let Some(origin) = region.origin {
                    for &(start, end) in &region.used_intervals {
                        let Some(relative_start) = start.checked_sub(origin) else {
                            continue;
                        };
                        let Some(relative_end) = end.checked_sub(origin) else {
                            continue;
                        };
                        let Some(canonical_start) = region.alias_offset.checked_add(relative_start) else {
                            continue;
                        };
                        let Some(canonical_end) = region.alias_offset.checked_add(relative_end) else {
                            continue;
                        };
                        entry.used_intervals.push((canonical_start, canonical_end));
                    }
                }
            }
        } else {
            ordinary_regions.push(region);
        }
    }

    let mut groups = Vec::new();

    for (name, accumulator) in alias_groups {
        let capacity_bytes = interval_union_len(&accumulator.capacity_intervals);
        let used_bytes = interval_union_len(&accumulator.used_intervals);
        let utilization_pct = (capacity_bytes > 0)
            .then(|| used_bytes as f64 * 100.0 / capacity_bytes as f64);
        groups.push(MemoryGroup {
            name,
            kind: "ram".into(),
            regions: accumulator.regions,
            capacity_bytes,
            used_bytes,
            utilization_pct,
        });
    }

    // For ordinary regions, merge overlapping VMA windows so the same address
    // range is not counted twice. Disjoint windows remain independent groups.
    ordinary_regions.sort_by_key(|region| region.origin.unwrap_or(u64::MAX));
    let mut clusters: Vec<Vec<&MemoryRegion>> = Vec::new();
    let mut cluster_end = None;

    for region in ordinary_regions {
        let (Some(start), Some(end)) = (region.origin, region.end) else {
            clusters.push(vec![region]);
            cluster_end = None;
            continue;
        };

        if let Some(current_end) = cluster_end {
            if start < current_end {
                let cluster = clusters.last_mut().expect("cluster end implies a cluster");
                cluster.push(region);
                cluster_end = Some(current_end.max(end));
                continue;
            }
        }

        clusters.push(vec![region]);
        cluster_end = Some(end);
    }

    for (index, cluster) in clusters.into_iter().enumerate() {
        let mut capacity_intervals = Vec::new();
        let mut used_intervals = Vec::new();
        let mut names = Vec::new();

        for region in &cluster {
            names.push(region.name.clone());
            if let (Some(start), Some(end)) = (region.origin, region.end) {
                capacity_intervals.push((start, end));
            }
            if include_usage {
                used_intervals.extend(region.used_intervals.iter().copied());
            }
        }

        let capacity_bytes = interval_union_len(&capacity_intervals);
        let used_bytes = interval_union_len(&used_intervals);
        let utilization_pct = (capacity_bytes > 0)
            .then(|| used_bytes as f64 * 100.0 / capacity_bytes as f64);
        let name = if names.len() == 1 {
            names[0].clone()
        } else {
            format!("overlapping_ram_windows_{index}")
        };

        groups.push(MemoryGroup {
            name,
            kind: "ram".into(),
            regions: names,
            capacity_bytes,
            used_bytes,
            utilization_pct,
        });
    }

    groups
}

fn compute_capacities(regions: &[MemoryRegion]) -> (Option<u64>, Option<u64>) {
    // IROM and DROM are aliases into the same external Flash device.
    let flash_capacity = regions
        .iter()
        .filter(|region| region.kind == "flash")
        .filter_map(|region| region.length)
        .max();

    let ram_groups = build_ram_groups(regions, false);
    let ram_capacity = if ram_groups.is_empty() {
        None
    } else {
        Some(ram_groups.iter().fold(0u64, |sum, group| {
            sum.saturating_add(group.capacity_bytes)
        }))
    };

    (flash_capacity, ram_capacity)
}

/// Parses a GNU ld-style MEMORY block. It deliberately does not attempt to
/// interpret the complete linker-script language.
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

    let mut regions = Vec::new();
    for declaration in split_declarations(body) {
        let Some((raw_name, attributes)) = declaration.split_once(':') else {
            continue;
        };
        let name = raw_name.split_whitespace().next().unwrap_or("").trim();
        if name.is_empty() {
            continue;
        }

        let origin = parse_assignment(attributes, "ORIGIN", &regions);
        let length = parse_assignment(attributes, "LENGTH", &regions)
            .or_else(|| parse_assignment(attributes, "len", &regions));
        let end = origin.zip(length).and_then(|(origin, length)| origin.checked_add(length));

        regions.push(MemoryRegion {
            name: name.to_string(),
            kind: region_kind(name).to_string(),
            origin,
            length,
            end,
            alias_group: None,
            alias_offset: 0,
            used_bytes: 0,
            utilization_pct: None,
            load_bytes: 0,
            used_intervals: Vec::new(),
        });
    }

    apply_known_alias_profiles(&mut regions);
    let (flash_bytes, ram_bytes) = compute_capacities(&regions);

    MemoryRegions {
        flash_bytes,
        ram_bytes,
        ram_used_bytes: 0,
        flash_used_bytes: 0,
        unmapped_ram_bytes: 0,
        ram_worst_group_pct: None,
        flash_capacity_policy: "largest_flash_window_irom_drom_alias_safe".into(),
        ram_capacity_policy: "sum_of_linker_ram_pools_after_known_alias_deduplication; esp32c3_iram_dram_dram2_share_one_internal_sram_pool; icache_excluded; not_raw_silicon_capacity".into(),
        source: None,
        regions,
        ram_groups: Vec::new(),
    }
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

fn is_generated_script(path: &Path) -> bool {
    let components: Vec<String> = path
        .components()
        .map(|component| component.as_os_str().to_string_lossy().into_owned())
        .collect();
    components.windows(3).any(|window| {
        window[0] == "build" && window[2] == "out"
    })
}

fn profile_matches(path: &Path, profile: &std::ffi::OsStr) -> bool {
    path.components().any(|component| component.as_os_str() == profile)
}

fn target_triple_matches(path: &Path, triple: Option<&std::ffi::OsStr>) -> bool {
    let Some(triple) = triple else {
        return false;
    };
    path.components().any(|component| component.as_os_str() == triple)
}

/// Discovers the memory.x associated with the inspected ELF.
/// Generated scripts must match the ELF profile; target-triple matches are preferred.
/// Static package/workspace scripts remain valid fallbacks.
pub fn find_and_parse(
    workspace: &Path,
    package: &Path,
    target_dir: &Path,
    binary: &Path,
) -> Option<MemoryRegions> {
    let binary_parent = binary.parent()?;
    let profile = binary_parent.file_name()?;
    let triple = binary_parent.parent().and_then(Path::file_name);

    let static_candidates = [
        package.join("memory.x"),
        workspace.join("memory.x"),
        package.join("build/memory.x"),
    ];

    let mut generated_candidates = Vec::new();
    collect_memory_scripts(target_dir, 8, &mut generated_candidates);

    let workspace_target = workspace.join("target");
    if target_dir != workspace_target {
        collect_memory_scripts(&workspace_target, 8, &mut generated_candidates);
    }

    let package_target = package.join("target");
    if package != workspace && target_dir != package_target {
        collect_memory_scripts(&package_target, 8, &mut generated_candidates);
    }

    generated_candidates.retain(|path| {
        is_generated_script(path) && profile_matches(path, profile)
    });
    generated_candidates.sort_by_key(|path| {
        // Exact target-triple/profile paths win over host-profile or other target paths.
        !target_triple_matches(path, triple)
    });

    let mut candidates = generated_candidates;
    candidates.extend(static_candidates);

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

    const ESP32C3_MEMORY_X: &str = include_str!("../tests/fixtures/esp32c3-memory.x");

    #[test]
    fn parses_esp32c3_memory_regions_and_capacities() {
        let memory = parse_memory_x(ESP32C3_MEMORY_X);

        assert_eq!(memory.regions.len(), 7);
        assert_eq!(memory.flash_bytes, Some(0x400000 - 0x20));
        assert_eq!(memory.ram_bytes, Some(395_024));
        assert_eq!(memory.regions[0].kind, "cache");
        assert_eq!(memory.regions[1].origin, Some(0x4038_0000));
        assert_eq!(memory.regions[1].length, Some(304_128));
        assert_eq!(memory.regions[2].length, Some(320_512));
        assert_eq!(memory.regions[3].kind, "ram");
        assert_eq!(memory.regions[3].origin, Some(0x3fcc_e400));
        assert_eq!(memory.regions[3].length, Some(0x10310));
        assert_eq!(memory.regions[3].end, Some(0x3fcd_e710));
        assert_eq!(memory.regions[1].alias_group.as_deref(), Some("esp32c3_internal_sram"));
        assert_eq!(memory.regions[2].alias_group.as_deref(), Some("esp32c3_internal_sram"));
        assert_eq!(memory.regions[3].alias_group.as_deref(), Some("esp32c3_internal_sram"));
    }

    #[test]
    fn irom_and_drom_are_not_summed() {
        let memory = parse_memory_x(
            "MEMORY {\nIROM : ORIGIN = 0x42000020, LENGTH = 4M\nDROM : ORIGIN = 0x3C000020, LENGTH = 4M\n}",
        );
        assert_eq!(memory.flash_bytes, Some(4 * 1024 * 1024));
    }

    #[test]
    fn ram_capacity_deduplicates_overlapping_linker_windows() {
        let memory = parse_memory_x(
            "MEMORY {\nRAM_A : ORIGIN = 0x1000, LENGTH = 4K\nRAM_B : ORIGIN = 0x1800, LENGTH = 4K\n}",
        );
        assert_eq!(memory.ram_bytes, Some(0x1800));
    }

    #[test]
    fn dram2_sections_map_to_dram2_not_the_main_dram_region() {
        let mut memory = parse_memory_x(ESP32C3_MEMORY_X);
        let dram2_index = memory.find_region_index(0x3fcc_e400, 0x10000).unwrap();
        assert_eq!(memory.regions[dram2_index].name, "dram2_seg");

        memory.record_section(dram2_index, 0x3fcc_e400, 0x10000, 0);
        memory.finalize_usage(0, 0);

        assert_eq!(memory.regions[dram2_index].used_bytes, 65_536);
        assert_eq!(memory.ram_groups[0].name, "esp32c3_internal_sram");
        assert_eq!(memory.ram_groups[0].capacity_bytes, 386_832);
        assert_eq!(memory.ram_groups[0].used_bytes, 65_536);
        assert!(memory.regions[dram2_index].utilization_pct.unwrap() > 98.0);
        assert!(memory.regions[dram2_index].utilization_pct.unwrap() < 99.0);
        assert_eq!(memory.ram_bytes, Some(395_024));
    }

    #[test]
    fn stack_section_maps_to_main_dram_region() {
        let memory = parse_memory_x(ESP32C3_MEMORY_X);
        let stack_index = memory.find_region_index(0x3fc9_8150, 0x362b0).unwrap();
        assert_eq!(memory.regions[stack_index].name, "DRAM");
    }

    #[test]
    fn parses_nested_expressions_and_symbolic_region_references() {
        let memory = parse_memory_x(
            "MEMORY {\nRAM : ORIGIN = 0x20000000, LENGTH = 256K\nEXTRA (RW) : ORIGIN = (ORIGIN(RAM) + LENGTH(RAM)), len = (512K - (128K + 64K))\n}",
        );
        assert_eq!(memory.regions[1].origin, Some(0x2004_0000));
        assert_eq!(memory.regions[1].length, Some(320 * 1024));
    }

    #[test]
    fn ignores_comments_and_excludes_icache_from_flash() {
        let memory = parse_memory_x(
            "/* MEMORY { BAD : ORIGIN=0, LENGTH=1K } */\nMEMORY { ICACHE : ORIGIN = 0x4037C000, LENGTH = 16K\nIROM : ORIGIN = 0x42000020, LENGTH = 4M\n}",
        );
        assert_eq!(memory.regions.len(), 2);
        assert_eq!(memory.regions[0].kind, "cache");
        assert_eq!(memory.flash_bytes, Some(4 * 1024 * 1024));
    }

    #[test]
    fn release_binary_selects_release_memory_script_not_debug() {
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("target");
        let triple = "riscv32imc-unknown-none-elf";
        let debug_out = target.join(triple).join("debug/build/esp-hal-debug/out");
        let release_out = target.join(triple).join("release/build/esp-hal-release/out");
        fs::create_dir_all(&debug_out).unwrap();
        fs::create_dir_all(&release_out).unwrap();
        fs::write(debug_out.join("memory.x"), "MEMORY { RAM : ORIGIN=0x1000, LENGTH=1K }").unwrap();
        fs::write(release_out.join("memory.x"), ESP32C3_MEMORY_X).unwrap();

        let binary = target.join(triple).join("release/firmware");
        let memory = find_and_parse(temp.path(), temp.path(), &target, &binary).unwrap();
        assert!(memory.source.unwrap().contains("release/build/esp-hal-release/out/memory.x"));
    }

    #[test]
    fn debug_binary_selects_debug_memory_script_not_release() {
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("target");
        let triple = "riscv32imc-unknown-none-elf";
        let debug_out = target.join(triple).join("debug/build/esp-hal-debug/out");
        let release_out = target.join(triple).join("release/build/esp-hal-release/out");
        fs::create_dir_all(&debug_out).unwrap();
        fs::create_dir_all(&release_out).unwrap();
        fs::write(debug_out.join("memory.x"), ESP32C3_MEMORY_X).unwrap();
        fs::write(release_out.join("memory.x"), "MEMORY { RAM : ORIGIN=0x1000, LENGTH=1K }").unwrap();

        let binary = target.join(triple).join("debug/firmware");
        let memory = find_and_parse(temp.path(), temp.path(), &target, &binary).unwrap();
        assert!(memory.source.unwrap().contains("debug/build/esp-hal-debug/out/memory.x"));
    }
}

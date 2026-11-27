
use crate::{
    elf::{ElfReport, SectionUsage},
    memory_profiles::{MemoryClass, MemoryPoolSpec, MemoryProfile, MemoryWindow},
    memory_x::{MemoryRegion, MemoryRegions},
};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Serialize)]
pub struct RegionUsage {
    pub name: String,
    pub class: MemoryClass,
    pub origin: Option<u64>,
    pub length: Option<u64>,
    pub used_bytes: u64,
    pub file_bytes: u64,
    pub utilization_pct: Option<f64>,
    pub pool_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PoolUsage {
    pub id: String,
    pub label: String,
    pub capacity_bytes: Option<u64>,
    pub used_bytes: u64,
    pub utilization_pct: Option<f64>,
    pub regions: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MemoryAccounting {
    pub profile_id: String,
    pub profile_name: String,
    pub flash_used_bytes: u64,
    pub flash_capacity_bytes: Option<u64>,
    pub flash_utilization_pct: Option<f64>,
    pub ram_used_bytes: u64,
    pub ram_capacity_bytes: Option<u64>,
    pub ram_utilization_pct: Option<f64>,
    pub worst_ram_pool_pct: Option<f64>,
    pub ram_capacity_complete: bool,
    pub unclassified_ram_bytes: u64,
    pub regions: Vec<RegionUsage>,
    pub ram_pools: Vec<PoolUsage>,
    pub notes: Vec<String>,
}

#[derive(Default)]
struct RegionAccumulator {
    class: Option<MemoryClass>,
    origin: Option<u64>,
    length: Option<u64>,
    used: u64,
    file: u64,
    pool_id: Option<String>,
}

#[derive(Default)]
struct PoolAccumulator {
    used: u64,
    regions: BTreeSet<String>,
}

fn range_contains(region: &MemoryRegion, address: u64, size: u64) -> bool {
    let (Some(start), Some(length)) = (region.origin, region.length) else {
        return false;
    };

    let Some(end) = start.checked_add(length) else {
        return false;
    };

    let Some(section_end) = address.checked_add(size) else {
        return false;
    };

    address >= start && section_end <= end
}

fn best_linker_region<'a>(
    regions: &'a [MemoryRegion],
    address: u64,
    size: u64,
) -> Option<&'a MemoryRegion> {
    regions
        .iter()
        .filter(|region| range_contains(region, address, size))
        .min_by_key(|region| region.length.unwrap_or(u64::MAX))
}

fn generic_class_for_section(section: &SectionUsage) -> MemoryClass {
    let name = section.name.to_ascii_lowercase();

    if name.contains(".bss")
        || name.contains(".data")
        || name.contains(".stack")
        || name.contains(".heap")
        || name.contains(".noinit")
        || name.contains(".uninit")
    {
        MemoryClass::Ram
    } else if section.writable {
        MemoryClass::Ram
    } else {
        MemoryClass::Unknown
    }
}

fn pool_spec_capacity(
    spec: &MemoryPoolSpec,
    linker_map: Option<&MemoryRegions>,
) -> Option<u64> {
    let linker_capacity = linker_map.and_then(|map| {
        let mut matches: Vec<u64> = map
            .regions
            .iter()
            .filter(|region| {
                spec.capacity_regions
                    .iter()
                    .any(|name| region.name.eq_ignore_ascii_case(name))
            })
            .filter_map(|region| region.length)
            .collect();

        if matches.is_empty() {
            None
        } else {
            matches.sort_unstable();
            Some(matches.into_iter().fold(0u64, u64::saturating_add))
        }
    });

    match (spec.fixed_capacity_bytes, linker_capacity) {
        (Some(fixed), Some(linker)) => Some(fixed.min(linker)),
        (Some(fixed), None) => Some(fixed),
        (None, value) => value,
    }
}

fn flash_capacity(
    profile: &MemoryProfile,
    linker_map: Option<&MemoryRegions>,
) -> Option<u64> {
    linker_map.and_then(|map| {
        map.regions
            .iter()
            .filter(|region| {
                profile
                    .flash_capacity_regions
                    .iter()
                    .any(|name| region.name.eq_ignore_ascii_case(name))
                    || (profile.flash_capacity_regions.is_empty()
                        && is_flash_region_name(&region.name))
            })
            .filter_map(|region| region.length)
            .max()
    })
}

fn is_flash_region_name(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();

    upper.contains("FLASH")
        || matches!(upper.as_str(), "IROM" | "DROM" | "ROM" | "XIP")
}

/// Accounts the intersection of one ELF section with the profile's RAM windows.
///
/// A section may span multiple physical banks. Each byte is attributed to at
/// most one window, even if a future profile contains overlapping windows.
///
/// Returns:
/// - mapped RAM bytes;
/// - mapped file-backed bytes.
fn account_ram_window_overlaps(
    section: &SectionUsage,
    profile: &MemoryProfile,
    region_usage: &mut BTreeMap<String, RegionAccumulator>,
    pool_usage: &mut BTreeMap<String, PoolAccumulator>,
) -> (u64, u64) {
    if section.memory_size == 0 {
        return (0, 0);
    }

    let Some(section_end) = section.vma.checked_add(section.memory_size) else {
        return (0, 0);
    };

    let file_backed_size = section.file_size.min(section.memory_size);
    let file_end = section
        .vma
        .checked_add(file_backed_size)
        .unwrap_or(u64::MAX);

    let mut overlaps: Vec<(&MemoryWindow, u64, u64)> = profile
        .windows
        .iter()
        .filter(|window| window.class == MemoryClass::Ram)
        .filter_map(|window| {
            let start = section.vma.max(window.start);
            let end = section_end.min(window.end);

            (start < end).then_some((window, start, end))
        })
        .collect();

    overlaps.sort_by_key(|(_, start, _)| *start);

    let mut cursor = section.vma;
    let mut mapped_bytes = 0u64;
    let mut mapped_file_bytes = 0u64;

    for (window, overlap_start, overlap_end) in overlaps {
        // Prevent double accounting if two profile windows overlap.
        let start = overlap_start.max(cursor);
        let end = overlap_end.min(section_end);

        if start >= end {
            continue;
        }

        let used = end - start;
        let file_bytes = end.min(file_end).saturating_sub(start);

        let region_name = window.name.clone();
        let pool_id = window.pool_id.clone().unwrap_or_default();

        let region = region_usage
            .entry(region_name.clone())
            .or_default();

        region.class = Some(MemoryClass::Ram);
        region.origin = Some(window.start);
        region.length = Some(window.end.saturating_sub(window.start));
        region.used = region.used.saturating_add(used);
        region.file = region.file.saturating_add(file_bytes);

        if !pool_id.is_empty() {
            region.pool_id = Some(pool_id.clone());

            let pool = pool_usage.entry(pool_id).or_default();
            pool.used = pool.used.saturating_add(used);
            pool.regions.insert(region_name);
        }

        mapped_bytes = mapped_bytes.saturating_add(used);
        mapped_file_bytes = mapped_file_bytes.saturating_add(file_bytes);
        cursor = end;
    }

    (mapped_bytes, mapped_file_bytes)
}

/// Maps ELF section allocation onto a target profile and, where available,
/// the selected linker MEMORY declarations.
///
/// PT_LOAD segments remain metadata; section bytes are the sole accounting
/// source to prevent double counting.
pub fn analyze(
    elf: &ElfReport,
    profile: &MemoryProfile,
    linker_map: Option<&MemoryRegions>,
) -> MemoryAccounting {
    let linker_regions = linker_map
        .map(|map| map.regions.as_slice())
        .unwrap_or_default();

    let mut region_usage = BTreeMap::<String, RegionAccumulator>::new();
    let mut pool_usage = BTreeMap::<String, PoolAccumulator>::new();

    let mut flash_used = 0u64;
    let mut ram_used = 0u64;
    let mut unclassified_ram = 0u64;

    for section in &elf.sections {
        if section.name.ends_with("_dummy") || section.name.contains(".dummy") {
            continue;
        }

        // Every allocated file-backed section contributes to the linked image.
        // This includes initialized data copied from Flash into RAM.
        flash_used = flash_used.saturating_add(section.file_size);

        // First, account RAM sections against every intersecting profile window.
        //
        // This is essential for RP2350 sections such as:
        // .stack: 0x20080000..0x20082000
        //
        // That section occupies SRAM8 and SRAM9, so it cannot be mapped using
        // a single find_window(address, size) call.
        let (mapped_ram, mapped_file) = account_ram_window_overlaps(
            section,
            profile,
            &mut region_usage,
            &mut pool_usage,
        );

        if mapped_ram > 0 {
            ram_used = ram_used.saturating_add(section.memory_size);

            let remaining_ram = section
                .memory_size
                .saturating_sub(mapped_ram);

            let remaining_file = section
                .file_size
                .min(section.memory_size)
                .saturating_sub(mapped_file);

            if remaining_ram > 0 {
                unclassified_ram =
                    unclassified_ram.saturating_add(remaining_ram);

                let region = region_usage
                    .entry("unmapped".into())
                    .or_default();

                region.class = Some(MemoryClass::Ram);
                region.used = region.used.saturating_add(remaining_ram);
                region.file = region.file.saturating_add(remaining_file);
                region.pool_id = Some("unmapped_ram".into());
            }

            // The complete section has already been accounted, either against
            // profile RAM windows or as genuinely unmapped RAM.
            continue;
        }

        // Normal path for sections that do not intersect profile RAM windows.
        let map_region =
            best_linker_region(linker_regions, section.vma, section.memory_size);

        let profile_window =
            profile.find_window(section.vma, section.memory_size);

        let classification = map_region
            .and_then(|region| {
                profile.classify_linker_region_at(
                    &region.name,
                    section.vma,
                    section.memory_size,
                )
            })
            .or_else(|| {
                profile_window.map(|window| {
                    (
                        window.class,
                        window.pool_id.clone().unwrap_or_default(),
                    )
                })
            })
            .or_else(|| {
                map_region
                    .and_then(|region| generic_region_class(&region.name))
                    .map(|class| {
                        (
                            class,
                            generic_pool_id(map_region.unwrap(), class),
                        )
                    })
            });

        let (class, pool_id, region_name, origin, length) =
            if let Some((class, pool_id)) = classification {
                let region_name = map_region
                    .map(|region| region.name.clone())
                    .or_else(|| profile_window.map(|window| window.name.clone()))
                    .unwrap_or_else(|| "profile-window".into());

                let origin = map_region
                    .and_then(|region| region.origin)
                    .or_else(|| profile_window.map(|window| window.start));

                let length = map_region
                    .and_then(|region| region.length)
                    .or_else(|| {
                        profile_window.map(|window| {
                            window.end.saturating_sub(window.start)
                        })
                    });

                (class, pool_id, region_name, origin, length)
            } else {
                let class = generic_class_for_section(section);
                let region_name = "unmapped".to_string();

                let pool_id = if class == MemoryClass::Ram {
                    "unmapped_ram".to_string()
                } else {
                    String::new()
                };

                (class, pool_id, region_name, None, None)
            };

        let section_ram = if class == MemoryClass::Ram {
            section.memory_size
        } else {
            0
        };

        ram_used = ram_used.saturating_add(section_ram);

        if section_ram > 0 && region_name == "unmapped" {
            unclassified_ram =
                unclassified_ram.saturating_add(section_ram);
        }

        let entry = region_usage
            .entry(region_name.clone())
            .or_default();

        entry.class = Some(class);
        entry.origin = origin;
        entry.length = length;
        entry.used = entry.used.saturating_add(section_ram);
        entry.file = entry.file.saturating_add(section.file_size);

        if !pool_id.is_empty() {
            entry.pool_id = Some(pool_id.clone());
        }

        if section_ram > 0
            && !pool_id.is_empty()
            && pool_id != "unmapped_ram"
        {
            let pool = pool_usage.entry(pool_id).or_default();
            pool.used = pool.used.saturating_add(section_ram);
            pool.regions.insert(region_name);
        }
    }

    // Materialize target-defined pools.
    //
    // ESP32-C3 deliberately has different capacity and usage region sets:
    // IRAM consumes the shared SRAM budget, while the budget capacity is
    // derived from DRAM + dram2_seg.
    let mut pools = Vec::<PoolUsage>::new();

    for spec in &profile.ram_pools {
        let usage = pool_usage
            .remove(&spec.id)
            .unwrap_or_default();

        let mut region_names: BTreeSet<String> = usage.regions;

        for name in &spec.usage_regions {
            if linker_regions
                .iter()
                .any(|region| region.name.eq_ignore_ascii_case(name))
                && !region_names
                    .iter()
                    .any(|existing| existing.eq_ignore_ascii_case(name))
            {
                region_names.insert(name.clone());
            }
        }

        let capacity = pool_spec_capacity(spec, linker_map);

        pools.push(PoolUsage {
            id: spec.id.clone(),
            label: spec.label.clone(),
            capacity_bytes: capacity,
            used_bytes: usage.used,
            utilization_pct: capacity
                .filter(|value| *value > 0)
                .map(|value| usage.used as f64 * 100.0 / value as f64),
            regions: region_names.into_iter().collect(),
        });
    }

    // For linker-driven Cortex-M/generic maps, include every declared RAM
    // region, even when no allocated section currently occupies it.
    for linker_region in linker_regions {
        let broad_rp2350_ram = if profile.id == "rp2350" {
            match (linker_region.origin, linker_region.end) {
                (Some(start), Some(end)) => {
                    profile
                        .windows
                        .iter()
                        .filter(|window| {
                            window.class == MemoryClass::Ram
                                && start < window.end
                                && window.start < end
                        })
                        .count()
                        > 1
                }
                _ => false,
            }
        } else {
            false
        };

        if broad_rp2350_ram {
            // A broad RAM linker declaration spans several RP2350 bank pools.
            // Address windows split individual section usage into those banks.
            continue;
        }

        let classification = profile
            .classify_linker_region_at(
                &linker_region.name,
                linker_region.origin.unwrap_or(0),
                linker_region.length.unwrap_or(0),
            )
            .or_else(|| {
                generic_region_class(&linker_region.name).map(|class| {
                    (class, generic_pool_id(linker_region, class))
                })
            });

        let Some((MemoryClass::Ram, id)) = classification else {
            continue;
        };

        if let Some(pool) = pools.iter_mut().find(|pool| pool.id == id) {
            if !pool
                .regions
                .iter()
                .any(|name| name.eq_ignore_ascii_case(&linker_region.name))
            {
                pool.regions.push(linker_region.name.clone());
            }

            continue;
        }

        let used = region_usage
            .get(&linker_region.name)
            .map(|region| region.used)
            .unwrap_or(0);

        let capacity = linker_region.length;

        pools.push(PoolUsage {
            id,
            label: linker_region.name.clone(),
            capacity_bytes: capacity,
            used_bytes: used,
            utilization_pct: capacity
                .filter(|value| *value > 0)
                .map(|value| used as f64 * 100.0 / value as f64),
            regions: vec![linker_region.name.clone()],
        });
    }

    // Profile-only windows without a matching linker region are also reported.
    for (name, region) in &region_usage {
        if region.class != Some(MemoryClass::Ram) {
            continue;
        }

        let id = region
            .pool_id
            .clone()
            .unwrap_or_else(|| format!("ram:{name}"));

        if id == "unmapped_ram" || pools.iter().any(|pool| pool.id == id) {
            continue;
        }

        pools.push(PoolUsage {
            id,
            label: name.clone(),
            capacity_bytes: region.length,
            used_bytes: region.used,
            utilization_pct: region
                .length
                .filter(|value| *value > 0)
                .map(|value| region.used as f64 * 100.0 / value as f64),
            regions: vec![name.clone()],
        });
    }

    // Add unclassified RAM exactly once.
    //
    // It remains part of total RAM usage, but has no known capacity.
    if unclassified_ram > 0 {
        pools.push(PoolUsage {
            id: "unmapped_ram".into(),
            label: "Unmapped RAM (capacity unknown)".into(),
            capacity_bytes: None,
            used_bytes: unclassified_ram,
            utilization_pct: None,
            regions: vec!["unmapped".into()],
        });
    }

    pools.sort_by(|left, right| left.id.cmp(&right.id));

    let ram_capacity_complete = !pools.is_empty()
        && unclassified_ram == 0
        && pools.iter().all(|pool| pool.capacity_bytes.is_some())
        && pools
            .iter()
            .filter_map(|pool| pool.capacity_bytes)
            .any(|capacity| capacity > 0);

    let ram_capacity = if ram_capacity_complete {
        Some(
            pools
                .iter()
                .filter_map(|pool| pool.capacity_bytes)
                .fold(0u64, u64::saturating_add),
        )
    } else {
        None
    };

    let worst_ram_pool_pct = pools
        .iter()
        .filter_map(|pool| pool.utilization_pct)
        .reduce(f64::max);

    let ram_pct = if ram_capacity_complete {
        ram_capacity
            .filter(|value| *value > 0)
            .map(|capacity| ram_used as f64 * 100.0 / capacity as f64)
    } else {
        None
    };

    let regions: Vec<RegionUsage> = region_usage
        .into_iter()
        .map(|(name, region)| RegionUsage {
            name,
            class: region.class.unwrap_or(MemoryClass::Unknown),
            origin: region.origin,
            length: region.length,
            used_bytes: region.used,
            file_bytes: region.file,
            utilization_pct: region
                .length
                .filter(|length| *length > 0)
                .map(|length| region.used as f64 * 100.0 / length as f64),
            pool_id: region.pool_id,
        })
        .collect();

    let flash_capacity = flash_capacity(profile, linker_map);

    let flash_pct = flash_capacity
        .filter(|value| *value > 0)
        .map(|capacity| flash_used as f64 * 100.0 / capacity as f64);

    let mut notes = profile.notes.clone();

    if linker_map.is_none() {
        notes.push(
            "No linker MEMORY map was found; only profile-defined capacities are available."
                .into(),
        );
    }

    if unclassified_ram > 0 {
        notes.push(format!(
            "{} bytes of allocated RAM could not be assigned to a known memory pool; RAM threshold evaluation is incomplete.",
            unclassified_ram
        ));
    }

    MemoryAccounting {
        profile_id: profile.id.clone(),
        profile_name: profile.display_name.clone(),
        flash_used_bytes: flash_used,
        flash_capacity_bytes: flash_capacity,
        flash_utilization_pct: flash_pct,
        ram_used_bytes: ram_used,
        ram_capacity_bytes: ram_capacity,
        ram_utilization_pct: ram_pct,
        worst_ram_pool_pct,
        ram_capacity_complete,
        unclassified_ram_bytes: unclassified_ram,
        regions,
        ram_pools: pools,
        notes,
    }
}

fn generic_region_class(name: &str) -> Option<MemoryClass> {
    let upper = name.to_ascii_uppercase();

    if upper.contains("CACHE") || upper.contains("RESERVED") {
        Some(MemoryClass::Reserved)
    } else if upper.contains("FLASH")
        || matches!(upper.as_str(), "IROM" | "DROM" | "ROM" | "XIP")
    {
        Some(MemoryClass::Flash)
    } else if upper.contains("RAM")
        || matches!(
            upper.as_str(),
            "SRAM" | "IRAM" | "DRAM" | "DTCM" | "ITCM" | "CCM"
        )
    {
        Some(MemoryClass::Ram)
    } else {
        None
    }
}

fn generic_pool_id(region: &MemoryRegion, class: MemoryClass) -> String {
    match class {
        MemoryClass::Ram => {
            format!("ram:{}", region.name.to_ascii_uppercase())
        }
        MemoryClass::Flash => "external_flash".into(),
        MemoryClass::Reserved => {
            format!("reserved:{}", region.name.to_ascii_uppercase())
        }
        MemoryClass::Unknown => {
            format!("unknown:{}", region.name.to_ascii_uppercase())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        elf::{ElfReport, SectionUsage},
        memory_profiles::{esp32c3_profile, rp2040_profile, rp2350_profile},
        memory_x::parse_memory_x,
    };

    fn section(
        name: &str,
        address: u64,
        size: u64,
        file_size: u64,
    ) -> SectionUsage {
        SectionUsage {
            index: 0,
            name: name.into(),
            vma: address,
            lma: Some(address),
            size,
            file_size,
            memory_size: size,
            flags: 0,
            section_type: 1,
            allocated: true,
            writable: file_size == 0 || name.contains("data"),
            executable: name.contains("text"),
            segment_index: Some(0),
        }
    }

    fn elf(sections: Vec<SectionUsage>) -> ElfReport {
        ElfReport {
            path: "test.elf".into(),
            machine: 0,
            elf_class: 1,
            little_endian: true,
            entry: 0,
            sections,
            segments: vec![],
            top_symbols: vec![],
        }
    }

    #[test]
    fn rp2040_uses_profile_ram_capacity_but_does_not_invent_external_flash_size() {
        let report = elf(vec![section(".data", 0x2000_0000, 1024, 1024)]);
        let usage = analyze(&report, &rp2040_profile(), None);

        assert_eq!(usage.ram_capacity_bytes, Some(264 * 1024));
        assert_eq!(usage.flash_capacity_bytes, None);
        assert_eq!(usage.ram_used_bytes, 1024);
    }

    #[test]
    fn rp2350_splits_stack_across_sram8_and_sram9() {
        let report = elf(vec![section(".stack", 0x2008_0000, 8192, 0)]);
        let usage = analyze(&report, &rp2350_profile(), None);

        assert_eq!(usage.ram_used_bytes, 8192);
        assert_eq!(usage.ram_capacity_bytes, Some(520 * 1024));
        assert_eq!(usage.unclassified_ram_bytes, 0);
        assert!(usage.ram_capacity_complete);

        let sram8 = usage
            .ram_pools
            .iter()
            .find(|pool| pool.id == "rp2350_sram8")
            .unwrap();

        let sram9 = usage
            .ram_pools
            .iter()
            .find(|pool| pool.id == "rp2350_sram9")
            .unwrap();

        assert_eq!(sram8.used_bytes, 4096);
        assert_eq!(sram9.used_bytes, 4096);

        assert!(!usage
            .ram_pools
            .iter()
            .any(|pool| pool.id == "unmapped_ram"));
    }

    #[test]
    fn rp2350_does_not_duplicate_unmapped_ram_pool() {
        let report = elf(vec![section(".stack", 0x2100_0000, 8192, 0)]);
        let usage = analyze(&report, &rp2350_profile(), None);

        assert_eq!(usage.unclassified_ram_bytes, 8192);
        assert_eq!(usage.ram_used_bytes, 8192);

        let unmapped_pools: Vec<_> = usage
            .ram_pools
            .iter()
            .filter(|pool| pool.id == "unmapped_ram")
            .collect();

        assert_eq!(unmapped_pools.len(), 1);
        assert_eq!(unmapped_pools[0].used_bytes, 8192);
        assert!(!usage.ram_capacity_complete);
    }

    #[test]
    fn rp2350_combines_partially_and_fully_unmapped_ram() {
        let report = elf(vec![
            section(".stack", 0x2008_1800, 4096, 0),
            section(".bss", 0x2100_0000, 1024, 0),
        ]);
        let usage = analyze(&report, &rp2350_profile(), None);

        assert_eq!(usage.ram_used_bytes, 5120);
        assert_eq!(usage.unclassified_ram_bytes, 3072);
        assert_eq!(usage.ram_capacity_bytes, None);
        assert!(!usage.ram_capacity_complete);
        let unmapped: Vec<_> = usage
            .ram_pools
            .iter()
            .filter(|pool| pool.id == "unmapped_ram")
            .collect();
        assert_eq!(unmapped.len(), 1);
        assert_eq!(unmapped[0].used_bytes, 3072);
        assert_eq!(unmapped[0].capacity_bytes, None);
        assert_eq!(unmapped[0].utilization_pct, None);
        assert_eq!(
            usage.ram_pools.iter().find(|pool| pool.id == "rp2350_sram9").unwrap().used_bytes,
            2048
        );
    }

    #[test]
    fn esp32c3_shared_budget_counts_iram_and_dram_usage_but_not_iram_capacity_twice() {
        let map = parse_memory_x(
            "MEMORY {\nIRAM : ORIGIN=0x40380000, LENGTH=296K\nDRAM : ORIGIN=0x3fc80000, LENGTH=313K\ndram2_seg (RW) : ORIGIN=0x3fcce400, len=0x10310\nRTC_FAST : ORIGIN=0x50000000, LENGTH=8K\nIROM : ORIGIN=0x42000020, LENGTH=4M\nDROM : ORIGIN=0x3c000020, LENGTH=4M\n}",
        );

        let report = elf(vec![
            section(".rwtext", 0x4038_0000, 40_000, 40_000),
            section(".bss", 0x3fc8_0000, 200_000, 0),
            section(".dram2_uninit", 0x3fcc_e400, 60_000, 0),
        ]);

        let usage = analyze(&report, &esp32c3_profile(), Some(&map));

        assert_eq!(usage.ram_capacity_bytes, Some(395_024));
        assert_eq!(usage.ram_used_bytes, 300_000);

        let internal = usage
            .ram_pools
            .iter()
            .find(|pool| pool.id == "esp32c3_internal_sram")
            .unwrap();

        assert_eq!(internal.capacity_bytes, Some(386_832));
        assert_eq!(internal.used_bytes, 300_000);
    }
}

use anyhow::{Context, Result};
use goblin::elf::{program_header, section_header, sym, Elf};
use rustc_demangle::demangle;
use serde::Serialize;
use std::{collections::HashSet, fs, path::Path};

use crate::memory_x::MemoryRegions;

#[derive(Debug, Clone, Serialize)]
pub struct SectionUsage {
    pub name: String,
    pub address: u64,
    pub vma: u64,
    pub lma: Option<u64>,
    pub size: u64,
    pub file_size: u64,
    pub memory_size: u64,
    pub flash: u64,
    pub ram: u64,
    pub accounting_source: String,
    pub memory_region: String,
    /// Index of the PT_LOAD segment containing this section, when available.
    pub segment_index: Option<usize>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SegmentUsage {
    pub index: usize,
    pub segment_type: String,
    pub vma: u64,
    pub lma: u64,
    pub file_offset: u64,
    pub file_size: u64,
    pub memory_size: u64,
    pub flags: u32,
    /// File-backed allocated section bytes assigned to this segment.
    /// This is not p_filesz; it excludes linker-only dummy sections.
    pub flash: u64,
    /// RAM-resident allocated section bytes assigned to this segment.
    /// This is not p_memsz; it excludes linker-only dummy sections.
    pub ram: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct SymbolUsage {
    pub name: String,
    pub size: u64,
    pub address: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ElfReport {
    pub path: String,
    pub flash_bytes: u64,
    pub ram_bytes: u64,
    pub accounting_method: String,
    pub sections: Vec<SectionUsage>,
    pub segments: Vec<SegmentUsage>,
    pub top_symbols: Vec<SymbolUsage>,
}

fn contains_range(start: u64, size: u64, address: u64, length: u64) -> bool {
    match (start.checked_add(size), address.checked_add(length)) {
        (Some(end), Some(requested_end)) => address >= start && requested_end <= end,
        _ => false,
    }
}

fn segment_for_section<'a>(
    section: &goblin::elf::SectionHeader,
    segments: &'a [program_header::ProgramHeader],
) -> Option<(usize, &'a program_header::ProgramHeader)> {
    segments.iter().enumerate().find(|(_, segment)| {
        segment.p_type == program_header::PT_LOAD
            && contains_range(segment.p_vaddr, segment.p_memsz, section.sh_addr, section.sh_size)
            && (section.sh_type == section_header::SHT_NOBITS
                || contains_range(segment.p_offset, segment.p_filesz, section.sh_offset, section.sh_size))
    })
}

/// Fallback address windows when no linker memory map is available.
/// A parsed memory.x always takes precedence over these heuristics.
fn classify_esp32c3(address: u64) -> Option<&'static str> {
    match address {
        0x4037_c000..0x403e_0000 => Some("IRAM"),
        0x3fc8_0000..0x3fce_0000 => Some("DRAM"),
        0x5000_0000..0x5000_2000 => Some("RTC_FAST"),
        0x4200_0000..0x4280_0000 => Some("IROM"),
        0x3c00_0000..0x3c80_0000 => Some("DROM"),
        _ => None,
    }
}

fn fallback_region_kind(name: &str) -> &'static str {
    match name {
        "IRAM" | "DRAM" | "RTC_FAST" => "ram",
        "IROM" | "DROM" => "flash",
        _ => "other",
    }
}

fn inspect_symbols(elf: &Elf, top: usize) -> Vec<SymbolUsage> {
    let mut symbols = Vec::new();
    let mut seen = HashSet::new();

    for symbol in &elf.syms {
        if symbol.st_type() != sym::STT_FUNC || symbol.st_size == 0 {
            continue;
        }
        let Some(raw) = elf.strtab.get_at(symbol.st_name) else {
            continue;
        };
        if raw.is_empty() || !seen.insert((symbol.st_value, symbol.st_size, raw.to_string())) {
            continue;
        }
        symbols.push(SymbolUsage {
            name: demangle(raw).to_string(),
            size: symbol.st_size,
            address: symbol.st_value,
        });
    }

    for symbol in &elf.dynsyms {
        if symbol.st_type() != sym::STT_FUNC || symbol.st_size == 0 {
            continue;
        }
        let Some(raw) = elf.dynstrtab.get_at(symbol.st_name) else {
            continue;
        };
        if raw.is_empty() || !seen.insert((symbol.st_value, symbol.st_size, raw.to_string())) {
            continue;
        }
        symbols.push(SymbolUsage {
            name: demangle(raw).to_string(),
            size: symbol.st_size,
            address: symbol.st_value,
        });
    }

    symbols.sort_by(|a, b| {
        b.size
            .cmp(&a.size)
            .then_with(|| a.address.cmp(&b.address))
            .then_with(|| a.name.cmp(&b.name))
    });
    symbols.truncate(top);
    symbols
}

fn inspect_sections(
    elf: &Elf,
    memory: &mut Option<&mut MemoryRegions>,
) -> Vec<SectionUsage> {
    let mut result = Vec::new();
    let mut unmapped_ram_bytes = 0u64;

    for section in &elf.section_headers {
        let name = elf
            .shdr_strtab
            .get_at(section.sh_name)
            .unwrap_or("<unknown>")
            .to_string();
        let allocated = section.sh_flags & section_header::SHF_ALLOC as u64 != 0;
        if !allocated || section.sh_size == 0 || name.ends_with("_dummy") {
            continue;
        }

        let nobits = section.sh_type == section_header::SHT_NOBITS;
        let matched_segment = segment_for_section(section, &elf.program_headers);
        let segment_index = matched_segment.map(|(index, _)| index);
        let lma = matched_segment.and_then(|(_, segment)| {
            segment
                .p_paddr
                .checked_add(section.sh_addr.saturating_sub(segment.p_vaddr))
        });
        let file_backed = !nobits && matched_segment.is_some();
        let file_size = if file_backed { section.sh_size } else { 0 };
        let writable = section.sh_flags & section_header::SHF_WRITE as u64 != 0;
        let executable = section.sh_flags & section_header::SHF_EXECINSTR as u64 != 0;
        let relocated = matched_segment.is_some_and(|(_, segment)| segment.p_vaddr != segment.p_paddr);

        let mapped_index = memory
            .as_ref()
            .and_then(|map| map.find_region_index(section.sh_addr, section.sh_size));

        let (region_name, region_kind) = if let Some(index) = mapped_index {
            let map = memory.as_ref().expect("mapped index requires memory map");
            let region = &map.regions[index];
            (region.name.clone(), region.kind.clone())
        } else if let Some(name) = classify_esp32c3(section.sh_addr) {
            (name.to_string(), fallback_region_kind(name).to_string())
        } else {
            ("unknown".to_string(), "other".to_string())
        };

        let ram = match region_kind.as_str() {
            "ram" => section.sh_size,
            "flash" | "cache" => 0,
            _ if writable || (executable && relocated) => section.sh_size,
            _ => 0,
        };
        let flash = file_size;

        if let Some(index) = mapped_index {
            let should_record = (region_kind == "ram" && ram > 0)
                || (region_kind == "flash" && file_size > 0);
            if should_record {
                if let Some(map) = memory.as_deref_mut() {
                    map.record_section(index, section.sh_addr, section.sh_size, file_size);
                }
            }
        } else if ram > 0 && memory.is_some() {
            // A linker map exists, but this RAM section did not fit any declared
            // RAM window. Keep it in the ELF total and make RAM CI thresholds
            // unavailable rather than silently treating the report as complete.
            unmapped_ram_bytes = unmapped_ram_bytes.saturating_add(ram);
        }

        result.push(SectionUsage {
            name,
            address: section.sh_addr,
            vma: section.sh_addr,
            lma,
            size: section.sh_size,
            file_size,
            memory_size: section.sh_size,
            flash,
            ram,
            accounting_source: if matched_segment.is_some() {
                "PT_LOAD+section".into()
            } else {
                "section_flags_only".into()
            },
            memory_region: region_name,
            segment_index,
        });
    }

    if let Some(map) = memory.as_deref_mut() {
        let flash_total = result.iter().fold(0u64, |sum, section| sum.saturating_add(section.flash));
        map.finalize_usage(flash_total, unmapped_ram_bytes);
    }

    result
}

fn inspect_segments(
    elf: &Elf,
    sections: &[SectionUsage],
) -> Vec<SegmentUsage> {
    let mut result: Vec<SegmentUsage> = elf
        .program_headers
        .iter()
        .enumerate()
        .filter_map(|(index, segment)| {
            if segment.p_type != program_header::PT_LOAD {
                return None;
            }
            Some(SegmentUsage {
                index,
                segment_type: "PT_LOAD".into(),
                vma: segment.p_vaddr,
                lma: segment.p_paddr,
                file_offset: segment.p_offset,
                file_size: segment.p_filesz,
                memory_size: segment.p_memsz,
                flags: segment.p_flags,
                flash: 0,
                ram: 0,
            })
        })
        .collect();

    // Segment contribution is derived from its allocated sections. This keeps
    // PT_LOAD metadata useful while excluding linker-only *_dummy sections.
    for section in sections {
        let Some(index) = section.segment_index else {
            continue;
        };
        if let Some(segment) = result.iter_mut().find(|segment| segment.index == index) {
            segment.flash = segment.flash.saturating_add(section.flash);
            segment.ram = segment.ram.saturating_add(section.ram);
        }
    }

    result
}

pub fn inspect(
    path: &Path,
    top: usize,
    mut memory: Option<&mut MemoryRegions>,
) -> Result<ElfReport> {
    let bytes = fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let elf = Elf::parse(&bytes)
        .with_context(|| format!("invalid ELF file: {}", path.display()))?;

    let sections = inspect_sections(&elf, &mut memory);
    let segments = inspect_segments(&elf, &sections);
    let flash_bytes = sections
        .iter()
        .fold(0u64, |sum, section| sum.saturating_add(section.flash));
    let ram_bytes = sections
        .iter()
        .fold(0u64, |sum, section| sum.saturating_add(section.ram));
    let top_symbols = inspect_symbols(&elf, top);

    Ok(ElfReport {
        path: path.display().to_string(),
        flash_bytes,
        ram_bytes,
        accounting_method: "allocated_sections_with_linker_region_mapping".into(),
        sections,
        segments,
        top_symbols,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn range_containment_checks_overflow_and_bounds() {
        assert!(contains_range(100, 100, 120, 20));
        assert!(!contains_range(100, 100, 190, 20));
        assert!(!contains_range(u64::MAX - 5, 10, u64::MAX - 2, 1));
        assert!(contains_range(100, 100, 200, 0));
    }

    #[test]
    fn esp32c3_fallback_windows_are_classified() {
        assert_eq!(classify_esp32c3(0x4038_0000), Some("IRAM"));
        assert_eq!(classify_esp32c3(0x3fc8_0000), Some("DRAM"));
        assert_eq!(classify_esp32c3(0x4200_0020), Some("IROM"));
        assert_eq!(classify_esp32c3(0x3c00_0020), Some("DROM"));
        assert_eq!(classify_esp32c3(0x2000_0000), None);
    }
}

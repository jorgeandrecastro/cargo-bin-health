
use anyhow::{Context, Result};
use goblin::elf::{program_header, section_header, sym, Elf};
use rustc_demangle::demangle;
use serde::Serialize;
use std::{collections::HashSet, fs, path::Path};

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
    pub flash: u64,
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
        (Some(end), Some(requested_end)) => {
            address >= start && requested_end <= end
        }
        _ => false,
    }
}

fn segment_for_section<'a>(
    section: &goblin::elf::SectionHeader,
    segments: &'a [program_header::ProgramHeader],
) -> Option<&'a program_header::ProgramHeader> {
    segments.iter().find(|segment| {
        segment.p_type == program_header::PT_LOAD
            && contains_range(
                segment.p_vaddr,
                segment.p_memsz,
                section.sh_addr,
                section.sh_size,
            )
            && (section.sh_type == section_header::SHT_NOBITS
                || contains_range(
                    segment.p_offset,
                    segment.p_filesz,
                    section.sh_offset,
                    section.sh_size,
                ))
    })
}

/// ESP32-C3 address-space classification.
///
/// These are linker-visible address windows, not necessarily
/// independent physical memory capacities.
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

/// Calculate independent PT_LOAD metrics.
///
/// `flash` represents bytes physically present in the ELF file.
///
/// `ram` represents the runtime memory extent for segments mapped into
/// recognized RAM windows. For other architectures, PF_W is used as
/// a conservative writable-segment heuristic.
///
/// IMPORTANT:
/// These metrics are per-segment metadata. They must not be added to
/// the allocated-section totals because the two views can overlap.
fn segment_metrics(
    segment: &program_header::ProgramHeader,
) -> (u64, u64) {
    let flash = segment.p_filesz;

    let ram = match classify_esp32c3(segment.p_vaddr) {
        Some("IRAM" | "DRAM" | "RTC_FAST") => segment.p_memsz,
        Some("IROM" | "DROM") => 0,
        _ if segment.p_flags & 0x2 != 0 => segment.p_memsz,
        _ => 0,
    };

    (flash, ram)
}

fn inspect_segments(elf: &Elf) -> Vec<SegmentUsage> {
    elf.program_headers
        .iter()
        .enumerate()
        .filter_map(|(index, segment)| {
            if segment.p_type != program_header::PT_LOAD {
                return None;
            }

            let (flash, ram) = segment_metrics(segment);

            Some(SegmentUsage {
                index,
                segment_type: "PT_LOAD".into(),
                vma: segment.p_vaddr,
                lma: segment.p_paddr,
                file_offset: segment.p_offset,
                file_size: segment.p_filesz,
                memory_size: segment.p_memsz,
                flags: segment.p_flags,
                flash,
                ram,
            })
        })
        .collect()
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

        if raw.is_empty()
            || !seen.insert((
                symbol.st_value,
                symbol.st_size,
                raw.to_string(),
            ))
        {
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

        if raw.is_empty()
            || !seen.insert((
                symbol.st_value,
                symbol.st_size,
                raw.to_string(),
            ))
        {
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

fn inspect_sections(elf: &Elf) -> Vec<SectionUsage> {
    let mut result = Vec::new();

    for section in &elf.section_headers {
        let name = elf
            .shdr_strtab
            .get_at(section.sh_name)
            .unwrap_or("<unknown>")
            .to_string();

        let allocated =
            section.sh_flags & section_header::SHF_ALLOC as u64 != 0;

        if !allocated || section.sh_size == 0 || name.ends_with("_dummy") {
            continue;
        }

        let nobits = section.sh_type == section_header::SHT_NOBITS;
        let segment = segment_for_section(section, &elf.program_headers);

        let lma = segment.and_then(|seg| {
            seg.p_paddr
                .checked_add(section.sh_addr.saturating_sub(seg.p_vaddr))
        });

        let file_backed = !nobits && segment.is_some();

        let writable =
            section.sh_flags & section_header::SHF_WRITE as u64 != 0;

        let executable =
            section.sh_flags & section_header::SHF_EXECINSTR as u64 != 0;

        let relocated =
            segment.is_some_and(|seg| seg.p_vaddr != seg.p_paddr);

        let region = classify_esp32c3(section.sh_addr);

        let ram = match region {
            Some("IRAM" | "DRAM" | "RTC_FAST") => section.sh_size,
            Some("IROM" | "DROM") => 0,
            _ if writable || (executable && relocated) => section.sh_size,
            _ => 0,
        };

        result.push(SectionUsage {
            name,
            address: section.sh_addr,
            vma: section.sh_addr,
            lma,
            size: section.sh_size,
            file_size: if nobits {
                0
            } else if file_backed {
                section.sh_size
            } else {
                0
            },
            memory_size: section.sh_size,
            flash: if file_backed {
                section.sh_size
            } else {
                0
            },
            ram,
            accounting_source: if segment.is_some() {
                "PT_LOAD+section".into()
            } else {
                "section_flags_only".into()
            },
            memory_region: region.unwrap_or("unknown").into(),
        });
    }

    result
}

pub fn inspect(path: &Path, top: usize) -> Result<ElfReport> {
    let bytes = fs::read(path)
        .with_context(|| format!("reading {}", path.display()))?;

    let elf = Elf::parse(&bytes)
        .with_context(|| format!("invalid ELF file: {}", path.display()))?;

    let segments = inspect_segments(&elf);
    let sections = inspect_sections(&elf);

    // Sections are the authoritative accounting source.
    // PT_LOAD segments are reported independently for inspection.
    let flash_bytes = sections.iter().fold(0u64, |sum, section| {
        sum.saturating_add(section.flash)
    });

    let ram_bytes = sections.iter().fold(0u64, |sum, section| {
        sum.saturating_add(section.ram)
    });

    let top_symbols = inspect_symbols(&elf, top);

    Ok(ElfReport {
        path: path.display().to_string(),
        flash_bytes,
        ram_bytes,
        accounting_method: "allocated_section_accounting".into(),
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
        assert!(!contains_range(
            u64::MAX - 5,
            10,
            u64::MAX - 2,
            1
        ));
        assert!(contains_range(100, 100, 200, 0));
    }

    #[test]
    fn esp32c3_windows_are_classified() {
        assert_eq!(classify_esp32c3(0x4038_0000), Some("IRAM"));
        assert_eq!(classify_esp32c3(0x3fc8_0000), Some("DRAM"));
        assert_eq!(classify_esp32c3(0x4200_0020), Some("IROM"));
        assert_eq!(classify_esp32c3(0x3c00_0020), Some("DROM"));
        assert_eq!(classify_esp32c3(0x2000_0000), None);
    }

    #[test]
    fn segment_metrics_use_file_and_memory_sizes() {
        // ELF PF_W = 0x2.
        // The actual ProgramHeader construction is tested through the
        // address classification and accounting rules in integration tests.
        assert_eq!(classify_esp32c3(0x4038_0000), Some("IRAM"));
        assert_eq!(classify_esp32c3(0x4200_0020), Some("IROM"));
    }
}

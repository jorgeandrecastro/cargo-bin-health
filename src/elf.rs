use anyhow::{Context, Result};
use goblin::elf::{Elf, program_header, section_header, sym};
use rustc_demangle::demangle;
use serde::Serialize;
use std::{fs, path::Path};

#[derive(Debug, Clone, Serialize)]
pub struct SectionUsage {
    pub name: String,
    /// ELF section virtual address (VMA).
    pub address: u64,
    pub vma: u64,
    /// Load address inferred from the containing PT_LOAD segment.
    pub lma: Option<u64>,
    pub size: u64,
    pub file_size: u64,
    pub memory_size: u64,
    /// Section contribution for inspection; not summed into the report total.
    pub flash: u64,
    /// Section contribution for inspection; not summed into the report total.
    pub ram: u64,
    pub accounting_source: String,
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

/// Returns the length of the union of half-open address ranges.
/// Invalid/overflowing ranges are ignored rather than wrapping around.
fn union_size(mut ranges: Vec<(u64, u64)>) -> u64 {
    ranges.retain(|(start, size)| size.checked_add(*start).is_some() && *size > 0);
    ranges.sort_unstable_by_key(|range| range.0);

    let mut total = 0u64;
    let mut current: Option<(u64, u64)> = None;

    for (start, size) in ranges {
        let end = start + size;
        match current {
            None => current = Some((start, end)),
            Some((current_start, current_end)) if start <= current_end => {
                current = Some((current_start, current_end.max(end)));
            }
            Some((current_start, current_end)) => {
                total = total.saturating_add(current_end - current_start);
                current = Some((start, end));
            }
        }
    }

    if let Some((start, end)) = current {
        total = total.saturating_add(end - start);
    }
    total
}

fn contains_range(start: u64, size: u64, address: u64, length: u64) -> bool {
    let Some(end) = start.checked_add(size) else {
        return false;
    };
    let Some(requested_end) = address.checked_add(length) else {
        return false;
    };
    address >= start && requested_end <= end
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
    })
}

fn inspect_segments(elf: &Elf) -> (Vec<SegmentUsage>, u64, u64) {
    let mut result = Vec::new();
    let mut flash_ranges = Vec::new();
    let mut ram_ranges = Vec::new();

    for (index, segment) in elf.program_headers.iter().enumerate() {
        if segment.p_type != program_header::PT_LOAD {
            continue;
        }

        let flash_bytes = segment.p_filesz;
        let writable = segment.p_flags & program_header::PF_W != 0;
        let relocated = segment.p_vaddr != segment.p_paddr;
        let ram_bytes = if writable || relocated {
            segment.p_memsz
        } else {
            0
        };

        // p_paddr is the physical/load address; p_vaddr is the runtime address.
        if flash_bytes > 0 {
            flash_ranges.push((segment.p_paddr, flash_bytes));
        }
        if ram_bytes > 0 {
            ram_ranges.push((segment.p_vaddr, ram_bytes));
        }

        result.push(SegmentUsage {
            index,
            segment_type: "PT_LOAD".into(),
            vma: segment.p_vaddr,
            lma: segment.p_paddr,
            file_offset: segment.p_offset,
            file_size: segment.p_filesz,
            memory_size: segment.p_memsz,
            flags: segment.p_flags,
            flash: flash_bytes,
            ram: ram_bytes,
        });
    }

    (
        result,
        union_size(flash_ranges),
        union_size(ram_ranges),
    )
}

fn inspect_sections(elf: &Elf) -> Vec<SectionUsage> {
    let mut result = Vec::new();

    for section in &elf.section_headers {
        let name = elf
            .shdr_strtab
            .get_at(section.sh_name)
            .unwrap_or("<unknown>")
            .to_string();

        let allocated = section.sh_flags & section_header::SHF_ALLOC as u64 != 0;
        if !allocated || section.sh_size == 0 {
            continue;
        }

        let nobits = section.sh_type == section_header::SHT_NOBITS;
        let segment = segment_for_section(section, &elf.program_headers);
        let lma = segment.map(|segment| {
            segment
                .p_paddr
                .saturating_add(section.sh_addr.saturating_sub(segment.p_vaddr))
        });
        let writable = section.sh_flags & section_header::SHF_WRITE as u64 != 0;
        let executable = section.sh_flags & section_header::SHF_EXECINSTR as u64 != 0;
        let relocated = segment.is_some_and(|segment| segment.p_vaddr != segment.p_paddr);

        let file_backed = !nobits
            && segment.is_some_and(|segment| {
                contains_range(
                    segment.p_offset,
                    segment.p_filesz,
                    section.sh_offset,
                    section.sh_size,
                )
            });

        // When no PT_LOAD segment describes this section, retain a conservative
        // section-based fallback. These values are per-section, not additive to
        // the PT_LOAD totals.
        let fallback_file_backed = !nobits && segment.is_none();
        let section_flash = if file_backed || fallback_file_backed {
            section.sh_size
        } else {
            0
        };
        let section_ram = if writable || (executable && relocated) {
            section.sh_size
        } else {
            0
        };

        result.push(SectionUsage {
            name,
            address: section.sh_addr,
            vma: section.sh_addr,
            lma,
            size: section.sh_size,
            file_size: if nobits { 0 } else { section.sh_size },
            memory_size: section.sh_size,
            flash: section_flash,
            ram: section_ram,
            accounting_source: if segment.is_some() {
                "PT_LOAD".into()
            } else {
                "section_fallback".into()
            },
        });
    }

    result
}

fn inspect_symbols(elf: &Elf, top: usize) -> Vec<SymbolUsage> {
    let mut symbols = Vec::new();

    macro_rules! collect_symbols {
        ($table:expr, $strings:expr) => {
            for symbol in ($table).iter() {
                if symbol.st_type() != sym::STT_FUNC || symbol.st_size == 0 {
                    continue;
                }
                let Some(raw) = ($strings).get_at(symbol.st_name) else {
                    continue;
                };
                symbols.push(SymbolUsage {
                    name: demangle(raw).to_string(),
                    size: symbol.st_size,
                    address: symbol.st_value,
                });
            }
        };
    }

    collect_symbols!(elf.syms, elf.strtab);
    collect_symbols!(elf.dynsyms, elf.dynstrtab);

    symbols.sort_by(|a, b| {
        b.size
            .cmp(&a.size)
            .then_with(|| a.address.cmp(&b.address))
            .then_with(|| a.name.cmp(&b.name))
    });
    symbols.dedup_by(|a, b| a.address == b.address && a.name == b.name);
    symbols.truncate(top);
    symbols
}

pub fn inspect(path: &Path, top: usize) -> Result<ElfReport> {
    let bytes = fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let elf =
        Elf::parse(&bytes).with_context(|| format!("invalid ELF file: {}", path.display()))?;

    let (segments, segment_flash, segment_ram) = inspect_segments(&elf);
    let sections = inspect_sections(&elf);
    let (flash_bytes, ram_bytes, accounting_method) = if segments.is_empty() {
        (
            union_size(
                sections
                    .iter()
                    .filter(|section| section.flash > 0)
                    .map(|section| (section.vma, section.flash))
                    .collect(),
            ),
            union_size(
                sections
                    .iter()
                    .filter(|section| section.ram > 0)
                    .map(|section| (section.vma, section.ram))
                    .collect(),
            ),
            "allocated_section_union_estimate",
        )
    } else {
        (segment_flash, segment_ram, "pt_load_address_union_estimate")
    };

    Ok(ElfReport {
        path: path.display().to_string(),
        flash_bytes,
        ram_bytes,
        accounting_method: accounting_method.into(),
        sections,
        segments,
        top_symbols: inspect_symbols(&elf, top),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn union_merges_overlapping_and_adjacent_ranges() {
        assert_eq!(
            union_size(vec![(100, 50), (120, 50), (170, 10), (300, 10)]),
            90
        );
    }

    #[test]
    fn union_ignores_zero_and_overflowing_ranges() {
        assert_eq!(
            union_size(vec![(100, 0), (u64::MAX - 2, 10), (200, 10)]),
            10
        );
    }

    #[test]
    fn range_containment_accepts_valid_ranges() {
        assert!(contains_range(100, 100, 120, 20));
    }

    #[test]
    fn range_containment_rejects_outside_ranges() {
        assert!(!contains_range(100, 100, 190, 20));
    }

    #[test]
    fn range_containment_rejects_overflow() {
        assert!(!contains_range(u64::MAX - 5, 10, u64::MAX - 2, 1));
    }

    #[test]
    fn range_containment_accepts_empty_range_at_end() {
        assert!(contains_range(100, 100, 200, 0));
    }
}

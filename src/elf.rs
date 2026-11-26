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
    let mut flash = 0u64;
    let mut ram = 0u64;

    for (index, segment) in elf.program_headers.iter().enumerate() {
        if segment.p_type != program_header::PT_LOAD {
            continue;
        }

        // PT_LOAD file contents represent the bytes that must be loaded.
        let segment_flash = segment.p_filesz;

        // Writable segments represent runtime memory.
        // Executable segments relocated to another address also require RAM.
        let writable = segment.p_flags & program_header::PF_W != 0;
        let relocated = segment.p_vaddr != segment.p_paddr;

        let segment_ram = if writable || relocated {
            segment.p_memsz
        } else {
            0
        };

        flash = flash.saturating_add(segment_flash);
        ram = ram.saturating_add(segment_ram);

        result.push(SegmentUsage {
            index,
            segment_type: "PT_LOAD".into(),
            vma: segment.p_vaddr,
            lma: segment.p_paddr,
            file_offset: segment.p_offset,
            file_size: segment.p_filesz,
            memory_size: segment.p_memsz,
            flags: segment.p_flags,
            flash: segment_flash,
            ram: segment_ram,
        });
    }

    (result, flash, ram)
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

        if !allocated || section.sh_size == 0 || name.ends_with("_dummy") {
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

        // These are section-level contributions, not additional totals.
        let section_flash = if file_backed { section.sh_size } else { 0 };

        // ESP32-C3 address map: IRAM and DRAM are distinct linker windows over
        // the internal SRAM; count actual allocated sections, never linker dummy reservations.
        let esp_iram = (0x4037_c000..0x403e_0000).contains(&section.sh_addr);
        let esp_dram = (0x3fc8_0000..0x3fce_0000).contains(&section.sh_addr);
        let esp_rtc = (0x5000_0000..0x5000_2000).contains(&section.sh_addr);
        let esp_flash = (0x4200_0000..0x4280_0000).contains(&section.sh_addr)
            || (0x3c00_0000..0x3c80_0000).contains(&section.sh_addr);
        let esp_layout = esp_iram || esp_dram || esp_rtc || esp_flash;
        let section_ram = if esp_layout {
            if esp_iram || esp_dram || esp_rtc { section.sh_size } else { 0 }
        } else if writable || (executable && relocated) { section.sh_size } else { 0 };
        let memory_region = if esp_iram { "IRAM" } else if esp_dram { "DRAM" }
            else if esp_rtc { "RTC_FAST" } else if (0x4200_0000..0x4280_0000).contains(&section.sh_addr) { "IROM" }
            else if (0x3c00_0000..0x3c80_0000).contains(&section.sh_addr) { "DROM" }
            else { "unknown" };

        let source = if segment.is_some() {
            "PT_LOAD"
        } else {
            "section_flags_only"
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
            accounting_source: source.into(),
            memory_region: memory_region.into(),
        });
    }

    result
}

fn inspect_symbols(elf: &Elf, top: usize) -> Vec<SymbolUsage> {
    let mut symbols = Vec::new();

    for symbol in elf.syms.iter().chain(elf.dynsyms.iter()) {
        if symbol.st_type() != sym::STT_FUNC || symbol.st_size == 0 {
            continue;
        }

        let raw = elf
            .strtab
            .get_at(symbol.st_name)
            .or_else(|| elf.dynstrtab.get_at(symbol.st_name))
            .unwrap_or("<unknown>");

        if raw == "<unknown>" {
            continue;
        }

        symbols.push(SymbolUsage {
            name: demangle(raw).to_string(),
            size: symbol.st_size,
            address: symbol.st_value,
        });
    }

    // Avoid reporting the same symbol twice when it appears in both tables.
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

    let (segments, _segment_flash, _segment_ram) = inspect_segments(&elf);
    let sections = inspect_sections(&elf);
    let flash_bytes = sections.iter().map(|s| s.flash).sum();
    let ram_bytes = sections.iter().map(|s| s.ram).sum();
    let top_symbols = inspect_symbols(&elf, top);

    let accounting_method = if segments.is_empty() {
        "allocated_section_accounting"
    } else {
        "allocated_section_accounting"
    };

    Ok(ElfReport {
        path: path.display().to_string(),
        flash_bytes,
        ram_bytes,
        accounting_method: accounting_method.into(),
        sections,
        segments,
        top_symbols,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn bss_is_not_file_backed() {
        let bss = SectionUsage {
            name: ".bss".into(),
            address: 0x2000,
            vma: 0x2000,
            lma: None,
            size: 128,
            file_size: 0,
            memory_size: 128,
            flash: 0,
            ram: 128,
            accounting_source: "test".into(),
            memory_region: "DRAM".into(),
        };

        assert_eq!(bss.file_size, 0);
        assert_eq!(bss.ram, 128);
        assert_eq!(bss.flash, 0);
    }

    #[test]
    fn initialized_data_can_occupy_both_memories() {
        let data = SectionUsage {
            name: ".data".into(),
            address: 0x2000,
            vma: 0x2000,
            lma: Some(0x1000),
            size: 256,
            file_size: 256,
            memory_size: 256,
            flash: 256,
            ram: 256,
            accounting_source: "test".into(),
            memory_region: "DRAM".into(),
        };

        assert_eq!(data.flash, 256);
        assert_eq!(data.ram, 256);
        assert_ne!(data.vma, data.lma.unwrap());
    }
}

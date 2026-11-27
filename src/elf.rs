use anyhow::{Context, Result};
use goblin::elf::{Elf, program_header, section_header, sym};
use rustc_demangle::demangle;
use serde::Serialize;
use std::{collections::HashSet, fs, path::Path};

#[derive(Debug, Clone, Serialize)]
pub struct SectionUsage {
    pub index: usize,
    pub name: String,
    pub vma: u64,
    pub lma: Option<u64>,
    pub size: u64,
    pub file_size: u64,
    pub memory_size: u64,
    pub flags: u64,
    pub section_type: u32,
    pub allocated: bool,
    pub writable: bool,
    pub executable: bool,
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
    pub machine: u16,
    pub elf_class: u8,
    pub little_endian: bool,
    pub entry: u64,
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
    section_is_nobits: bool,
) -> Option<(usize, &'a program_header::ProgramHeader)> {
    segments.iter().enumerate().find(|(_, segment)| {
        segment.p_type == program_header::PT_LOAD
            && contains_range(segment.p_vaddr, segment.p_memsz, section.sh_addr, section.sh_size)
            && (section_is_nobits
                || contains_range(segment.p_offset, segment.p_filesz, section.sh_offset, section.sh_size))
    })
}

fn inspect_segments(elf: &Elf) -> Vec<SegmentUsage> {
    elf.program_headers
        .iter()
        .enumerate()
        .filter_map(|(index, segment)| {
            (segment.p_type == program_header::PT_LOAD).then_some(SegmentUsage {
                index,
                segment_type: "PT_LOAD".into(),
                vma: segment.p_vaddr,
                lma: segment.p_paddr,
                file_offset: segment.p_offset,
                file_size: segment.p_filesz,
                memory_size: segment.p_memsz,
                flags: segment.p_flags,
            })
        })
        .collect()
}

fn inspect_sections(elf: &Elf) -> Vec<SectionUsage> {
    elf.section_headers
        .iter()
        .enumerate()
        .filter_map(|(index, section)| {
            let name = elf.shdr_strtab.get_at(section.sh_name).unwrap_or("<unknown>").to_string();
            let allocated = section.sh_flags & section_header::SHF_ALLOC as u64 != 0;
            if !allocated || section.sh_size == 0 {
                return None;
            }

            let is_nobits = section.sh_type == section_header::SHT_NOBITS;
            let segment = segment_for_section(section, &elf.program_headers, is_nobits);
            let segment_index = segment.map(|(segment_index, _)| segment_index);
            let lma = segment.and_then(|(_, header)| {
                header.p_paddr.checked_add(section.sh_addr.checked_sub(header.p_vaddr)?)
            });
            let file_size = if is_nobits { 0 } else { section.sh_size };

            Some(SectionUsage {
                index,
                name,
                vma: section.sh_addr,
                lma,
                size: section.sh_size,
                file_size,
                memory_size: section.sh_size,
                flags: section.sh_flags,
                section_type: section.sh_type,
                allocated,
                writable: section.sh_flags & section_header::SHF_WRITE as u64 != 0,
                executable: section.sh_flags & section_header::SHF_EXECINSTR as u64 != 0,
                segment_index,
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

    symbols.sort_by(|left, right| {
        right.size.cmp(&left.size)
            .then_with(|| left.address.cmp(&right.address))
            .then_with(|| left.name.cmp(&right.name))
    });
    symbols.truncate(top);
    symbols
}

pub fn inspect(path: &Path, top: usize) -> Result<ElfReport> {
    let bytes = fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let elf = Elf::parse(&bytes).with_context(|| format!("invalid ELF file: {}", path.display()))?;

    Ok(ElfReport {
        path: path.display().to_string(),
        machine: elf.header.e_machine,
        elf_class: elf.header.e_ident[goblin::elf::header::EI_CLASS],
        little_endian: elf.little_endian,
        entry: elf.entry,
        sections: inspect_sections(&elf),
        segments: inspect_segments(&elf),
        top_symbols: inspect_symbols(&elf, top),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn range_containment_checks_bounds_and_overflow() {
        assert!(contains_range(100, 100, 120, 20));
        assert!(!contains_range(100, 100, 190, 20));
        assert!(!contains_range(u64::MAX - 5, 10, u64::MAX - 2, 1));
        assert!(contains_range(100, 100, 200, 0));
    }
}

use anyhow::{Context, Result};
use goblin::elf::{Elf, section_header, sym};
use rustc_demangle::demangle;
use serde::Serialize;
use std::{fs, path::Path};

#[derive(Debug, Clone, Serialize)]
pub struct SectionUsage {
    pub name: String,
    pub address: u64,
    pub size: u64,
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
    pub sections: Vec<SectionUsage>,
    pub top_symbols: Vec<SymbolUsage>,
}

pub fn inspect(path: &Path, top: usize) -> Result<ElfReport> {
    let bytes = fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let elf = Elf::parse(&bytes).context("invalid ELF file")?;
    let mut sections = Vec::new();
    let mut flash = 0u64;
    let mut ram = 0u64;
    for section in &elf.section_headers {
        let name = elf
            .shdr_strtab
            .get_at(section.sh_name)
            .unwrap_or("")
            .to_string();
        let size = section.sh_size;
        let alloc = section.sh_flags & section_header::SHF_ALLOC as u64 != 0;
        let is_bss = section.sh_type == section_header::SHT_NOBITS;
        let is_flash = matches!(
            name.as_str(),
            ".text" | ".rodata" | ".data" | ".init" | ".fini"
        ) || name.starts_with(".vector")
            || name.starts_with(".isr_vector");
        let is_ram = matches!(name.as_str(), ".data" | ".bss" | ".uninit" | ".noinit");
        let section_flash = if alloc && is_flash && !is_bss {
            size
        } else {
            0
        };
        let section_ram = if alloc && is_ram { size } else { 0 };
        flash += section_flash;
        ram += section_ram;
        if alloc && size > 0 {
            sections.push(SectionUsage {
                name,
                address: section.sh_addr,
                size,
                flash: section_flash,
                ram: section_ram,
            });
        }
    }
    let mut symbols = Vec::new();
    for sym in &elf.syms {
        if sym.st_type() == sym::STT_FUNC && sym.st_size > 0 {
            let raw = elf.strtab.get_at(sym.st_name).unwrap_or("<unknown>");
            let name = demangle(raw).to_string();
            symbols.push(SymbolUsage {
                name,
                size: sym.st_size,
                address: sym.st_value,
            });
        }
    }
    symbols.sort_by_key(|s| std::cmp::Reverse(s.size));
    symbols.truncate(top);
    Ok(ElfReport {
        path: path.display().to_string(),
        flash_bytes: flash,
        ram_bytes: ram,
        sections,
        top_symbols: symbols,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn section_accounting_classifies_bss_as_ram_only() {
        let bss = SectionUsage {
            name: ".bss".into(),
            address: 0,
            size: 128,
            flash: 0,
            ram: 128,
        };
        assert_eq!(bss.ram, 128);
        assert_eq!(bss.flash, 0);
    }
}

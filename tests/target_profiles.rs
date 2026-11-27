use cargo_bin_health::{
    accounting::analyze,
    cli::MemoryProfileArg,
    elf::{ElfReport, SectionUsage},
    memory_profiles::resolve_profile,
    memory_x::parse_memory_x,
};

fn elf_with_sections(sections: Vec<SectionUsage>) -> ElfReport {
    ElfReport {
        path: "fixture.elf".into(),
        machine: 0,
        elf_class: 1,
        little_endian: true,
        entry: 0,
        sections,
        segments: vec![],
        top_symbols: vec![],
    }
}

fn section(name: &str, address: u64, size: u64, file_size: u64, writable: bool) -> SectionUsage {
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
        writable,
        executable: !writable,
        segment_index: Some(0),
    }
}

#[test]
fn rp2040_uses_264_kib_sram_and_linker_flash_capacity() {
    let linker = parse_memory_x(include_str!("fixtures/rp2040-memory.x"));
    let profile = resolve_profile(MemoryProfileArg::Rp2040, Some("thumbv6m-none-eabi"), Some(&linker));
    let elf = elf_with_sections(vec![
        section(".text", 0x1000_0000, 20_000, 20_000, false),
        section(".data", 0x2000_0000, 1_024, 1_024, true),
    ]);
    let report = analyze(&elf, &profile, Some(&linker));

    assert_eq!(report.profile_id, "rp2040");
    assert_eq!(report.flash_capacity_bytes, Some(2 * 1024 * 1024));
    assert_eq!(report.ram_capacity_bytes, Some(264 * 1024));
    assert_eq!(report.ram_used_bytes, 1_024);
}

#[test]
fn rp2350_uses_520_kib_across_four_address_pools() {
    let linker = parse_memory_x(include_str!("fixtures/rp2350-memory.x"));
    let profile = resolve_profile(MemoryProfileArg::Rp2350, Some("thumbv8m.main-none-eabihf"), Some(&linker));
    let elf = elf_with_sections(vec![
        section(".text", 0x1000_0000, 30_000, 30_000, false),
        section(".data", 0x2000_0000, 4_096, 4_096, true),
        section(".bss", 0x2008_1000, 512, 0, true),
    ]);
    let report = analyze(&elf, &profile, Some(&linker));

    assert_eq!(report.ram_capacity_bytes, Some(520 * 1024));
    assert_eq!(report.ram_used_bytes, 4_608);
    assert_eq!(report.ram_pools.len(), 4);
    assert!(report.ram_pools.iter().any(|pool| pool.id == "rp2350_sram9" && pool.used_bytes == 512));
}

#[test]
fn cortex_m_capacity_is_derived_from_linker_memory_regions() {
    let linker = parse_memory_x(include_str!("fixtures/arm-cortex-m-memory.x"));
    let profile = resolve_profile(MemoryProfileArg::CortexM, Some("thumbv7em-none-eabihf"), Some(&linker));
    let elf = elf_with_sections(vec![
        section(".text", 0x0800_0000, 32_000, 32_000, false),
        section(".data", 0x2000_0000, 2_000, 2_000, true),
        section(".ccm", 0x1000_0000, 4_000, 0, true),
    ]);
    let report = analyze(&elf, &profile, Some(&linker));

    assert_eq!(profile.id, "cortex-m");
    assert_eq!(report.flash_capacity_bytes, Some(1024 * 1024));
    assert_eq!(report.ram_capacity_bytes, Some(320 * 1024));
    assert_eq!(report.ram_used_bytes, 6_000);
}

#[test]
fn esp32c3_uses_one_budget_for_iram_dram_and_dram2() {
    let linker = parse_memory_x(include_str!("fixtures/esp32c3-memory.x"));
    let profile = resolve_profile(MemoryProfileArg::Auto, Some("riscv32imc-unknown-none-elf"), Some(&linker));
    let elf = elf_with_sections(vec![
        section(".rwtext", 0x4038_0000, 40_712, 40_712, false),
        section(".data", 0x3fc8_0000, 6_920, 6_920, true),
        section(".bss", 0x3fc8_1b08, 200_000, 0, true),
        section(".dram2_uninit", 0x3fcc_e400, 65_536, 0, true),
    ]);
    let report = analyze(&elf, &profile, Some(&linker));

    assert_eq!(profile.id, "esp32c3");
    assert_eq!(report.ram_capacity_bytes, Some(395_024));
    assert_eq!(report.ram_used_bytes, 313_168);
    assert_eq!(report.flash_capacity_bytes, Some(0x400000 - 0x20));
}


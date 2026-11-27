use cargo_bin_health::{
    accounting::analyze,
    cli::MemoryProfileArg,
    elf::{ElfReport, SectionUsage},
    memory_profiles::{MemoryClass, resolve_profile},
    memory_x::parse_memory_x,
};

const ESP32C3_MEMORY_X: &str = include_str!("fixtures/esp32c3-memory.x");

#[test]
fn parses_esp32c3_linker_fixture() {
    let memory = parse_memory_x(ESP32C3_MEMORY_X);

    assert_eq!(memory.flash_bytes, Some(4_194_272));
    assert_eq!(memory.ram_bytes, Some(320_512));
    assert_eq!(memory.regions.len(), 7);

    let dram2 = memory
        .regions
        .iter()
        .find(|region| region.name == "dram2_seg")
        .unwrap();
    let profile = resolve_profile(MemoryProfileArg::Esp32c3, None, Some(&memory));
    assert_eq!(
        profile.classify_linker_region(&dram2.name),
        Some((MemoryClass::Ram, "esp32c3_internal_sram".into()))
    );
    assert_eq!(dram2.origin, Some(0x3fcc_e400));
    assert_eq!(dram2.length, Some(66_320));
    assert_eq!(dram2.end, Some(0x3fcd_e710));
}

#[test]
fn parser_preserves_overlapping_linker_windows() {
    let memory = parse_memory_x(
        "MEMORY {\nRAM_A : ORIGIN = 0x1000, LENGTH = 4K\nRAM_B : ORIGIN = 0x1800, LENGTH = 4K\n}",
    );
    assert_eq!(memory.ram_bytes, Some(0x1000));
    assert_eq!(memory.regions[0].origin, Some(0x1000));
    assert_eq!(memory.regions[0].end, Some(0x2000));
    assert_eq!(memory.regions[1].origin, Some(0x1800));
    assert_eq!(memory.regions[1].end, Some(0x2800));
}

#[test]
fn esp32c3_iram_dram_and_dram2_share_one_physical_sram_budget() {
    let memory = parse_memory_x(ESP32C3_MEMORY_X);
    let profile = resolve_profile(MemoryProfileArg::Esp32c3, None, Some(&memory));

    let sections = [
        (".rwtext", 0x4038_0000, 0x9b34),
        (".data", 0x3fc8_9b38, 0x1ac8),
        (".data.wifi", 0x3fc8_b600, 0x01f0),
        (".bss", 0x3fc8_b7f0, 0xc960),
        (".stack", 0x3fc9_8150, 0x362b0),
        (".dram2_uninit", 0x3fcc_e400, 0x10000),
    ]
    .into_iter()
    .map(|(name, address, size)| SectionUsage {
        index: 0,
        name: name.into(),
        vma: address,
        lma: Some(address),
        size,
        file_size: 0,
        memory_size: size,
        flags: 0,
        section_type: 1,
        allocated: true,
        writable: true,
        executable: false,
        segment_index: Some(0),
    })
    .collect();
    let elf = ElfReport {
        path: "fixture.elf".into(),
        machine: 0,
        elf_class: 1,
        little_endian: true,
        entry: 0,
        sections,
        segments: vec![],
        top_symbols: vec![],
    };
    let usage = analyze(&elf, &profile, Some(&memory));

    assert_eq!(usage.ram_capacity_bytes, Some(395_024));
    assert_eq!(usage.ram_used_bytes, 386_044);

    let sram = usage
        .ram_pools
        .iter()
        .find(|group| group.id == "esp32c3_internal_sram")
        .unwrap();
    assert_eq!(sram.capacity_bytes, Some(386_832));
    assert_eq!(sram.used_bytes, 386_044);
    assert!(sram.utilization_pct.unwrap() > 99.7);
    assert!(sram.utilization_pct.unwrap() < 99.9);
    assert!(usage.worst_ram_pool_pct.unwrap() > 99.7);
    assert!(usage.worst_ram_pool_pct.unwrap() < 99.9);
}

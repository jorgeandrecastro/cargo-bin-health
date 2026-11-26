use cargo_bin_health::memory_x::parse_memory_x;

const ESP32C3_MEMORY_X: &str = include_str!("fixtures/esp32c3-memory.x");

#[test]
fn parses_esp32c3_linker_fixture() {
    let memory = parse_memory_x(ESP32C3_MEMORY_X);

    assert_eq!(memory.flash_bytes, Some(4_194_272));
    assert_eq!(memory.ram_bytes, Some(395_024));
    assert_eq!(memory.regions.len(), 7);

    let dram2 = memory
        .regions
        .iter()
        .find(|region| region.name == "dram2_seg")
        .unwrap();
    assert_eq!(dram2.kind, "ram");
    assert_eq!(dram2.origin, Some(0x3fcc_e400));
    assert_eq!(dram2.length, Some(66_320));
    assert_eq!(dram2.end, Some(0x3fcd_e710));
}

#[test]
fn ram_capacity_deduplicates_overlapping_linker_windows() {
    let memory = parse_memory_x(
        "MEMORY {\nRAM_A : ORIGIN = 0x1000, LENGTH = 4K\nRAM_B : ORIGIN = 0x1800, LENGTH = 4K\n}",
    );
    assert_eq!(memory.ram_bytes, Some(0x1800));
}

#[test]
fn esp32c3_iram_dram_and_dram2_share_one_physical_sram_budget() {
    let mut memory = parse_memory_x(ESP32C3_MEMORY_X);

    let sections = [
        (0x4038_0000, 0x9b34), // .trap + .rwtext + .rwtext.wifi in IRAM
        (0x3fc8_9b38, 0x1ac8), // .data
        (0x3fc8_b600, 0x01f0), // .data.wifi
        (0x3fc8_b7f0, 0xc960), // .bss
        (0x3fc9_8150, 0x362b0), // .stack
        (0x3fcc_e400, 0x10000), // .dram2_uninit
    ];

    for (address, size) in sections {
        let index = memory.find_region_index(address, size).unwrap();
        memory.record_section(index, address, size, 0);
    }

    memory.finalize_usage(0, 0);

    assert_eq!(memory.ram_bytes, Some(395_024));
    assert_eq!(memory.ram_used_bytes, 386_044);

    let sram = memory
        .ram_groups
        .iter()
        .find(|group| group.name == "esp32c3_internal_sram")
        .unwrap();
    assert_eq!(sram.capacity_bytes, 386_832);
    assert_eq!(sram.used_bytes, 386_044);
    assert!(sram.utilization_pct.unwrap() > 99.7);
    assert!(sram.utilization_pct.unwrap() < 99.9);
    assert!(memory.ram_worst_group_pct.unwrap() > 99.7);
    assert!(memory.ram_worst_group_pct.unwrap() < 99.9);
}

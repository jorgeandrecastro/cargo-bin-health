use cargo_bin_health::memory_x::parse_memory_x;

#[test]
fn parses_flash_and_ram_capacities() {
    let script = "MEMORY { FLASH : ORIGIN = 0x08000000, LENGTH = 1M\n RAM : ORIGIN = 0x20000000, LENGTH = 256K }";
    let regions = parse_memory_x(script);
    assert_eq!(regions.flash_bytes, Some(1024 * 1024));
    assert_eq!(regions.ram_bytes, Some(256 * 1024));
}

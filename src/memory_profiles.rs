use crate::{cli::MemoryProfileArg, memory_x::MemoryRegions};
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryClass {
    Flash,
    Ram,
    Reserved,
    Unknown,
}

#[derive(Debug, Clone, Serialize)]
pub struct MemoryWindow {
    pub name: String,
    pub start: u64,
    pub end: u64,
    pub class: MemoryClass,
    pub pool_id: Option<String>,
    /// False for address mirrors whose physical alias transform is not modeled.
    pub supported_for_capacity: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct MemoryPoolSpec {
    pub id: String,
    pub label: String,
    /// Fixed hardware capacity, when the target datasheet defines one.
    pub fixed_capacity_bytes: Option<u64>,
    /// Linker regions that contribute to the pool capacity.
    pub capacity_regions: Vec<String>,
    /// Linker regions whose allocated sections consume this pool.
    /// This differs from capacity_regions on targets such as ESP32-C3.
    pub usage_regions: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MemoryProfile {
    pub id: String,
    pub display_name: String,
    pub description: String,
    pub windows: Vec<MemoryWindow>,
    pub ram_pools: Vec<MemoryPoolSpec>,
    /// Linker-region names used to determine Flash capacity. Capacity is the
    /// largest declared window, not the sum of address aliases.
    pub flash_capacity_regions: Vec<String>,
    pub notes: Vec<String>,
}

impl MemoryWindow {
    pub fn contains(&self, address: u64, size: u64) -> bool {
        let Some(end) = address.checked_add(size) else {
            return false;
        };
        address >= self.start && end <= self.end
    }
}

impl MemoryProfile {
    pub fn find_window(&self, address: u64, size: u64) -> Option<&MemoryWindow> {
        self.windows
            .iter()
            .filter(|window| window.contains(address, size))
            .min_by_key(|window| window.end.saturating_sub(window.start))
    }

    pub fn classify_linker_region(&self, name: &str) -> Option<(MemoryClass, String)> {
        let upper = name.to_ascii_uppercase();
        match self.id.as_str() {
            "esp32c3" => match upper.as_str() {
                "IRAM" | "DRAM" | "DRAM2_SEG" => {
                    Some((MemoryClass::Ram, "esp32c3_internal_sram".into()))
                }
                "RTC_FAST" => Some((MemoryClass::Ram, "esp32c3_rtc_fast".into())),
                "IROM" | "DROM" | "FLASH" => {
                    Some((MemoryClass::Flash, "external_flash".into()))
                }
                "ICACHE" => Some((MemoryClass::Reserved, "icache_reserved".into())),
                _ => None,
            },
            "rp2040" => match upper.as_str() {
                "FLASH" | "XIP" | "ROM" => {
                    Some((MemoryClass::Flash, "external_flash".into()))
                }
                "RAM" | "SRAM" => Some((MemoryClass::Ram, "rp2040_sram".into())),
                "USB_DPRAM" | "USB_DPRAM0" => {
                    Some((MemoryClass::Ram, "rp2040_usb_dpram".into()))
                }
                _ => None,
            },
            "rp2350" => match upper.as_str() {
                "FLASH" | "XIP" | "ROM" => {
                    Some((MemoryClass::Flash, "external_flash".into()))
                }
                "SRAM0_3" => Some((MemoryClass::Ram, "rp2350_sram0_3".into())),
                "SRAM4_7" => Some((MemoryClass::Ram, "rp2350_sram4_7".into())),
                "SRAM8" => Some((MemoryClass::Ram, "rp2350_sram8".into())),
                "SRAM9" => Some((MemoryClass::Ram, "rp2350_sram9".into())),
                "RAM" | "SRAM" => Some((MemoryClass::Ram, "rp2350_sram".into())),
                _ => None,
            },
            _ => generic_region_class(name).map(|class| {
                let pool = match class {
                    MemoryClass::Ram => format!("ram:{}", upper),
                    MemoryClass::Flash => "external_flash".into(),
                    MemoryClass::Reserved => format!("reserved:{}", upper),
                    MemoryClass::Unknown => format!("unknown:{}", upper),
                };
                (class, pool)
            }),
        }
    }

    /// Applies address-level bank selection when a linker declares one broad
    /// RAM region spanning several physical RP2350 SRAM bank windows.
    pub fn classify_linker_region_at(
        &self,
        name: &str,
        address: u64,
        size: u64,
    ) -> Option<(MemoryClass, String)> {
        if matches!(self.id.as_str(), "rp2040" | "rp2350")
            && generic_region_class(name) == Some(MemoryClass::Ram)
            && let Some(window) = self.find_window(address, size)
        {
            return Some((window.class, window.pool_id.clone().unwrap_or_default()));
        }
        self.classify_linker_region(name)
    }
}

fn generic_region_class(name: &str) -> Option<MemoryClass> {
    let upper = name.to_ascii_uppercase();
    if upper.contains("CACHE") || upper.contains("RESERVED") {
        Some(MemoryClass::Reserved)
    } else if upper.contains("FLASH")
        || matches!(upper.as_str(), "IROM" | "DROM" | "ROM" | "XIP")
    {
        Some(MemoryClass::Flash)
    } else if upper.contains("RAM")
        || matches!(upper.as_str(), "SRAM" | "IRAM" | "DRAM" | "DTCM" | "ITCM" | "CCM")
    {
        Some(MemoryClass::Ram)
    } else {
        None
    }
}

pub fn resolve_profile(
    requested: MemoryProfileArg,
    target_triple: Option<&str>,
    linker_map: Option<&MemoryRegions>,
) -> MemoryProfile {
    let selected = match requested {
        MemoryProfileArg::Esp32c3 => "esp32c3",
        MemoryProfileArg::Rp2040 => "rp2040",
        MemoryProfileArg::Rp2350 => "rp2350",
        MemoryProfileArg::CortexM => "cortex-m",
        MemoryProfileArg::Generic => "generic",
        MemoryProfileArg::Auto => {
            let is_esp32c3_map = linker_map.is_some_and(|map| {
                ["ICACHE", "IRAM", "DRAM", "IROM", "DROM"]
                    .iter()
                    .all(|wanted| map.regions.iter().any(|region| region.name.eq_ignore_ascii_case(wanted)))
            });
            let triple = target_triple.unwrap_or_default().to_ascii_lowercase();
            if is_esp32c3_map {
                "esp32c3"
            } else if triple.contains("rp2040") {
                "rp2040"
            } else if triple.contains("rp2350") {
                "rp2350"
            } else if triple.starts_with("thumb") {
                "cortex-m"
            } else {
                "generic"
            }
        }
    };

    match selected {
        "esp32c3" => esp32c3_profile(),
        "rp2040" => rp2040_profile(),
        "rp2350" => rp2350_profile(),
        "cortex-m" => cortex_m_profile(),
        _ => generic_profile(),
    }
}

fn window(name: &str, start: u64, end: u64, class: MemoryClass, pool: Option<&str>) -> MemoryWindow {
    MemoryWindow {
        name: name.into(),
        start,
        end,
        class,
        pool_id: pool.map(str::to_string),
        supported_for_capacity: true,
    }
}

fn pool(
    id: &str,
    label: &str,
    fixed_capacity_bytes: Option<u64>,
    capacity_regions: &[&str],
    usage_regions: &[&str],
) -> MemoryPoolSpec {
    MemoryPoolSpec {
        id: id.into(),
        label: label.into(),
        fixed_capacity_bytes,
        capacity_regions: capacity_regions.iter().map(|name| (*name).into()).collect(),
        usage_regions: usage_regions.iter().map(|name| (*name).into()).collect(),
    }
}

pub fn esp32c3_profile() -> MemoryProfile {
    MemoryProfile {
        id: "esp32c3".into(),
        display_name: "Espressif ESP32-C3".into(),
        description: "ESP32-C3 linker windows with a shared IRAM/DRAM allocation budget.".into(),
        windows: vec![
            window("IRAM", 0x4038_0000, 0x403e_0000, MemoryClass::Ram, Some("esp32c3_internal_sram")),
            window("DRAM", 0x3fc8_0000, 0x3fce_0000, MemoryClass::Ram, Some("esp32c3_internal_sram")),
            window("IROM", 0x4200_0000, 0x4280_0000, MemoryClass::Flash, Some("external_flash")),
            window("DROM", 0x3c00_0000, 0x3c80_0000, MemoryClass::Flash, Some("external_flash")),
            window("RTC_FAST", 0x5000_0000, 0x5000_2000, MemoryClass::Ram, Some("esp32c3_rtc_fast")),
        ],
        ram_pools: vec![
            pool(
                "esp32c3_internal_sram",
                "ESP32-C3 internal SRAM budget",
                None,
                &["DRAM", "dram2_seg"],
                &["IRAM", "DRAM", "dram2_seg"],
            ),
            pool(
                "esp32c3_rtc_fast",
                "ESP32-C3 RTC_FAST",
                None,
                &["RTC_FAST"],
                &["RTC_FAST"],
            ),
        ],
        flash_capacity_regions: vec!["IROM".into(), "DROM".into(), "FLASH".into()],
        notes: vec![
            "IRAM consumes the shared internal SRAM allocation budget; it is counted as usage but does not add a second capacity window.".into(),
            "DRAM plus dram2_seg define the linker-derived internal SRAM budget; RTC_FAST is separate.".into(),
            "IROM and DROM are address windows into external Flash; capacity uses the largest declared linker window, not their sum.".into(),
        ],
    }
}

pub fn rp2040_profile() -> MemoryProfile {
    MemoryProfile {
        id: "rp2040".into(),
        display_name: "Raspberry Pi RP2040".into(),
        description: "RP2040 main SRAM, USB DPRAM, and XIP address window.".into(),
        windows: vec![
            window("SRAM", 0x2000_0000, 0x2004_2000, MemoryClass::Ram, Some("rp2040_sram")),
            window("USB_DPRAM", 0x5010_0000, 0x5010_1000, MemoryClass::Ram, Some("rp2040_usb_dpram")),
            window("XIP", 0x1000_0000, 0x1100_0000, MemoryClass::Flash, Some("external_flash")),
            // Non-striped bank mirrors require bank-aware section splitting.
            // The profile intentionally does not claim their physical usage yet.
        ],
        ram_pools: vec![
            pool("rp2040_sram", "RP2040 main SRAM", Some(264 * 1024), &["RAM", "SRAM"], &["RAM", "SRAM"]),
        ],
        flash_capacity_regions: vec!["FLASH".into()],
        notes: vec![
            "RP2040 main SRAM is 264 KiB at 0x20000000; the address range is contiguous in the standard striped mapping.".into(),
            "The 16 MiB XIP window is an address aperture, not the physical Flash capacity of the board.".into(),
            "Non-striped SRAM bank mirrors at 0x21000000 are not yet mapped to canonical bank offsets; use the standard SRAM window for authoritative accounting.".into(),
            "USB DPRAM (4 KiB) and XIP cache-as-SRAM (16 KiB when XIP caching is disabled) are optional and are counted only when explicitly declared by the linker map.".into(),
        ],
    }
}

pub fn rp2350_profile() -> MemoryProfile {
    MemoryProfile {
        id: "rp2350".into(),
        display_name: "Raspberry Pi RP2350".into(),
        description: "RP2350 520 KiB SRAM split into the documented bank address windows.".into(),
        windows: vec![
            window("SRAM0_3", 0x2000_0000, 0x2004_0000, MemoryClass::Ram, Some("rp2350_sram0_3")),
            window("SRAM4_7", 0x2004_0000, 0x2008_0000, MemoryClass::Ram, Some("rp2350_sram4_7")),
            window("SRAM8", 0x2008_0000, 0x2008_1000, MemoryClass::Ram, Some("rp2350_sram8")),
            window("SRAM9", 0x2008_1000, 0x2008_2000, MemoryClass::Ram, Some("rp2350_sram9")),
            window("XIP", 0x1000_0000, 0x1100_0000, MemoryClass::Flash, Some("external_flash")),
        ],
        ram_pools: vec![
            pool("rp2350_sram0_3", "RP2350 SRAM0-3", Some(256 * 1024), &["SRAM0_3"], &["SRAM0_3"]),
            pool("rp2350_sram4_7", "RP2350 SRAM4-7", Some(256 * 1024), &["SRAM4_7"], &["SRAM4_7"]),
            pool("rp2350_sram8", "RP2350 SRAM8", Some(4 * 1024), &["SRAM8"], &["SRAM8"]),
            pool("rp2350_sram9", "RP2350 SRAM9", Some(4 * 1024), &["SRAM9"], &["SRAM9"]),
        ],
        flash_capacity_regions: vec!["FLASH".into()],
        notes: vec![
            "RP2350 contains 520 KiB SRAM: two 256 KiB striped regions plus SRAM8 and SRAM9 at 4 KiB each.".into(),
            "The 16 MiB XIP window is an address aperture, not the physical Flash capacity of the board.".into(),
            "The XIP cache is not reported as general-purpose SRAM.".into(),
        ],
    }
}

pub fn cortex_m_profile() -> MemoryProfile {
    MemoryProfile {
        id: "cortex-m".into(),
        display_name: "Generic ARM Cortex-M".into(),
        description: "No silicon capacities are assumed; the linker MEMORY map is authoritative.".into(),
        windows: Vec::new(),
        ram_pools: Vec::new(),
        flash_capacity_regions: Vec::new(),
        notes: vec![
            "Cortex-M identifies an instruction-set family, not a specific MCU memory configuration.".into(),
            "Flash and RAM capacities are taken from the project's linker MEMORY declarations.".into(),
        ],
    }
}

pub fn generic_profile() -> MemoryProfile {
    MemoryProfile {
        id: "generic".into(),
        display_name: "Generic ELF".into(),
        description: "Generic linker-map-driven memory accounting.".into(),
        windows: Vec::new(),
        ram_pools: Vec::new(),
        flash_capacity_regions: Vec::new(),
        notes: vec!["No target-specific silicon capacity is assumed.".into()],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory_x::parse_memory_x;

    #[test]
    fn rp2040_profile_uses_264_kib_main_sram_and_does_not_assume_flash_size() {
        let profile = resolve_profile(MemoryProfileArg::Rp2040, None, None);
        assert_eq!(profile.id, "rp2040");
        assert_eq!(profile.ram_pools[0].fixed_capacity_bytes, Some(264 * 1024));
        assert_eq!(profile.flash_capacity_regions, vec!["FLASH"]);
    }

    #[test]
    fn rp2350_profile_exposes_520_kib_as_four_address_pools() {
        let profile = resolve_profile(MemoryProfileArg::Rp2350, None, None);
        let total: u64 = profile.ram_pools.iter().filter_map(|pool| pool.fixed_capacity_bytes).sum();
        assert_eq!(total, 520 * 1024);
        assert_eq!(profile.ram_pools.len(), 4);
    }

    #[test]
    fn auto_detects_esp32c3_from_its_memory_regions() {
        let linker = parse_memory_x(
            "MEMORY {\nICACHE : ORIGIN=0x4037c000, LENGTH=16K\nIRAM : ORIGIN=0x40380000, LENGTH=296K\nDRAM : ORIGIN=0x3fc80000, LENGTH=313K\nIROM : ORIGIN=0x42000020, LENGTH=4M\nDROM : ORIGIN=0x3c000020, LENGTH=4M\n}",
        );
        let profile = resolve_profile(MemoryProfileArg::Auto, Some("riscv32imc-unknown-none-elf"), Some(&linker));
        assert_eq!(profile.id, "esp32c3");
    }

    #[test]
    fn generic_cortex_m_does_not_invent_hardware_capacities() {
        let profile = resolve_profile(MemoryProfileArg::CortexM, Some("thumbv7em-none-eabihf"), None);
        assert!(profile.ram_pools.is_empty());
        assert!(profile.windows.is_empty());
    }
}

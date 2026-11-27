# cargo-bin-health

[![Crates.io](https://img.shields.io/crates/v/cargo-bin-health.svg)](https://crates.io/crates/cargo-bin-health)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/License-MIT%20OR%20Apache--2.0-blue.svg)](LICENSE-APACHE)
[![Documentation](https://docs.rs/cargo-bin-health/badge.svg)](https://docs.rs/cargo-bin-health)

**`cargo-bin-health`** is a Cargo subcommand for inspecting compiled Rust executables and embedded firmware.

It analyzes ELF binaries to estimate Flash and RAM allocation, inspect linker memory regions, identify function sizes, and detect dependency or build configuration issues.

It is designed for embedded Rust projects where binary size, memory allocation, and linker configuration are important engineering constraints.

Version 0.4 provides explicit memory profiles for the **RP2350** and **ESP32-C3**, linker-aware memory accounting, CI thresholds, and machine-readable JSON reports.

## Features

- **ELF analysis** — inspect sections, symbols, program headers, and binary layout.
- **Flash and RAM accounting** — estimate allocated memory from ELF sections and linker information.
- **Function size analysis** — identify the largest functions when symbol information is available.
- **Linker memory inspection** — parse linker memory regions and report their capacities and utilization.
- **Target memory profiles** — use explicit memory models for the RP2350 and ESP32-C3.
- **Memory alias handling** — account for shared physical memory exposed through multiple linker regions.
- **Cargo metadata inspection** — inspect packages, targets, features, and dependencies.
- **Dependency diagnostics** — identify duplicate dependency versions.
- **CI thresholds** — fail a build when configured Flash or RAM limits are exceeded.
- **JSON output** — export reports for automation and further analysis.
- **No-build mode** — analyze an existing ELF without rebuilding the project.

## Installation

```bash
cargo install cargo-bin-health
```

## Quick start

Analyze the current Cargo project:

```bash
cargo bin-health --release
```

Analyze an existing ELF without rebuilding:

```bash
cargo bin-health \
    --no-build \
    --binary target/release/my-firmware \
    --json
```

Select a target-specific memory profile:

```bash
cargo bin-health \
    --memory-profile rp2350 \
    --no-build \
    --binary target/riscv32imac-unknown-none-elf/release/pico2-riscv
```

## Usage examples

### Analyze an ESP32-C3 firmware

```bash
cargo bin-health \
    --manifest-path ~/projets/esp32c3mini/Cargo.toml \
    --binary ~/projets/esp32c3mini/target/riscv32imc-unknown-none-elf/release/esp32c3mini \
    --memory-profile esp32c3 \
    --no-build \
    --json
```

### Analyze an RP2350 firmware

```bash
cargo bin-health \
    --manifest-path ~/projets/pico2-riscv/Cargo.toml \
    --binary ~/projets/pico2-riscv/target/riscv32imac-unknown-none-elf/release/pico2-riscv \
    --memory-profile rp2350 \
    --no-build \
    --json
```

### Enforce CI thresholds

```bash
cargo bin-health \
    --release \
    --max-flash-pct 80 \
    --max-ram-pct 80 \
    --ci
```

The RAM threshold is evaluated against the highest utilization among recognized RAM groups.

When a RAM threshold is configured but memory accounting cannot be reliably mapped, the CI check fails rather than treating an incomplete accounting result as a successful validation.

### Generate a JSON report

```bash
cargo bin-health --release --json
```

JSON output is intended for CI pipelines, scripts, dashboards, and other automated tooling.

## CLI options

| Option | Description |
|---|---|
| `--release` | Use the release profile |
| `--profile NAME` | Select a Cargo profile |
| `--target TRIPLE` | Select a compilation target |
| `--manifest-path PATH` | Path to an external Cargo manifest |
| `--package NAME`, `-p NAME` | Select a package |
| `--bin NAME` | Select a binary target |
| `--example NAME` | Select an example target |
| `--features a,b` | Enable Cargo features |
| `--no-default-features` | Disable default features |
| `--no-build` | Analyze an existing binary without building |
| `--binary PATH` | Explicit path to the ELF binary |
| `--memory-profile NAME` | Select a target memory profile (`rp2350`, `esp32c3`) |
| `--top N` | Display the N largest functions |
| `--max-flash-pct N` | Set the maximum Flash utilization percentage |
| `--max-ram-pct N` | Set the maximum RAM utilization percentage |
| `--ci` | Enable CI-oriented threshold handling |
| `--json` | Emit a JSON report |
| `--no-color` | Disable colored terminal output |

## Supported target memory profiles

Version 0.4 introduces explicit target memory profiles.

A profile defines target-specific memory regions, capacities, grouping policies, and accounting rules. It complements ELF and linker information rather than assuming that every linker region represents independent physical memory.

| Target | Profile | Validation |
|---|---|---|
| Raspberry Pi RP2350 | `rp2350` | Tested against a real RP2350 firmware ELF |
| ESP32-C3 | `esp32c3` | Tested against a real ESP32-C3 firmware ELF |

The profiles address embedded memory layouts where linker regions may overlap, alias, or represent different views of the same physical memory.

### RP2350

The RP2350 profile models a total of **520 KiB** across its configured memory pools:

- 256 KiB
- 256 KiB
- 4 KiB
- 4 KiB

The profile was tested against the `pico2-riscv` firmware built for:

`riscv32imac-unknown-none-elf`

Example:

```bash
cargo bin-health \
    --manifest-path ~/projets/pico2-riscv/Cargo.toml \
    --binary ~/projets/pico2-riscv/target/riscv32imac-unknown-none-elf/release/pico2-riscv \
    --memory-profile rp2350 \
    --no-build \
    --json
```

**Observed test results**

| Metric | Result |
|---|---:|
| Flash allocation estimate | 308,802 bytes |
| RAM allocation estimate | 18,760 bytes |
| Unclassified memory | 8,192 bytes |
| Configured profile capacity | 520 KiB |
| Symbol information | Available |

The tested ELF contains 8,192 bytes that remain unclassified by the RAM accounting model. This amount is associated with the `.stack` reservation in the tested report.

The profile is loaded and the ELF is analyzed, but this result does **not** demonstrate that all RAM allocations have been mapped successfully.

The Flash capacity is not provided by this profile result, so no Flash utilization percentage is reported here.

The RP2350 figures are ELF-based estimates, not measurements of runtime RAM consumption or stack high-water usage.

### ESP32-C3

The ESP32-C3 profile was tested against the `esp32c3mini` firmware built for:

`riscv32imc-unknown-none-elf`

Example:

```bash
cargo bin-health \
    --manifest-path ~/projets/esp32c3mini/Cargo.toml \
    --binary ~/projets/esp32c3mini/target/riscv32imc-unknown-none-elf/release/esp32c3mini \
    --memory-profile esp32c3 \
    --no-build \
    --json
```

**Observed test results**

| Metric | Result |
|---|---:|
| Flash allocation estimate | 477,420 bytes |
| Flash capacity | 4,194,272 bytes |
| Flash utilization | 11.38% |
| RAM allocation estimate | 386,048 bytes |
| RAM capacity | 395,024 bytes |
| RAM utilization | 97.73% |
| Highest RAM group utilization | 99.80% |
| Unclassified RAM | 0 bytes |

The profile recognizes the generated release `memory.x` and uses linker-derived memory regions.

The following regions were observed:

- `IRAM`
- `DRAM`
- `IROM`
- `DROM`
- `ICACHE`
- `RTC_FAST`
- `dram2_seg`

The profile groups `IRAM`, `DRAM`, and `dram2_seg` into a shared internal SRAM budget. This avoids treating linker aliases as independent physical RAM.

`RTC_FAST` is accounted for separately, while `ICACHE` is excluded from the RAM budget.

The Flash capacity is derived from the recognized linker Flash window. It is not calculated by summing potentially overlapping `IROM` and `DROM` regions.

The modeled RAM capacity is 395,024 bytes:

- Shared internal SRAM: 386,832 bytes
- `RTC_FAST`: 8,192 bytes

These values describe the configured linker and memory accounting model. They should not be confused with a direct measurement of physical silicon capacity or runtime memory consumption.

The 99.80% figure represents the highest utilization among recognized RAM groups. It highlights the most constrained group rather than hiding it inside an aggregate RAM percentage.

## Memory accounting

### ELF-based accounting

The primary source for memory allocation is the ELF section table.

The analyzer considers:

- Section allocation flags and types.
- `SHT_NOBITS` sections.
- Virtual memory addresses (VMA).
- Load memory addresses (LMA).
- File-backed sections.
- Uninitialized allocations such as BSS-like sections.
- Target-specific memory placement.
- Linker reservation sections.

ELF program headers (`PT_LOAD`) provide supporting information.

The accounting model avoids counting the same allocation twice when section and segment information describe overlapping content.

### Flash and RAM estimates

The tool distinguishes between:

- Initialized data.
- Read-only data.
- Executable code.
- Uninitialized allocations such as BSS-like sections.
- Linker-only reservation sections.

Linker-only sections such as `.rwdata_dummy` and `.rotext_dummy` can be excluded from normal allocation estimates when they represent reservations rather than actual allocated content.

A `.stack` reservation is not equivalent to measured runtime stack usage.

The reported values describe the ELF and linker memory model. They do not measure actual runtime heap consumption, stack high-water marks, or the memory behavior of a running device.

### Linker memory regions

The linker memory parser supports:

- Named `MEMORY` regions.
- Hexadecimal and decimal values.
- `K` and `M` suffixes.
- Addition and subtraction.
- Parenthesized expressions.
- `ORIGIN` and `LENGTH`.
- `origin`, `length`, and `end` expressions.
- `len =` declarations.

The tool can discover generated `memory.x` files under Cargo target build outputs and match them to the selected build profile, including debug and release.

This parser implements a focused subset of GNU ld linker syntax. It is not a complete GNU ld language implementation.

### Memory aliases and utilization

Some embedded targets expose multiple linker regions that refer to the same physical memory.

Counting each region independently can produce incorrect capacity and utilization estimates.

Target profiles therefore define grouping and accounting policies to represent shared memory budgets.

When a RAM threshold is configured, the CI check uses the highest utilization among recognized RAM groups.

This prevents a heavily used memory pool from being hidden by the aggregate capacity of unrelated pools.

## Dependency and build diagnostics

`cargo-bin-health` also inspects Cargo metadata to provide context about the analyzed project.

It can report:

- Package and target information.
- Enabled features.
- Dependency information.
- Duplicate dependency versions.
- Build configuration details relevant to binary analysis.

These diagnostics complement ELF inspection: the final executable remains the source of truth for the binary layout.

Dependency diagnostics are heuristic. They help identify potential issues but do not replace a complete dependency audit.

## Limitations

The tool has several deliberate limitations:

- ELF binaries are supported; PE, Mach-O, raw binaries, and UF2 files are not directly analyzed.
- Automatic binary discovery relies on conventional Cargo output paths.
- Linker parsing supports a focused subset of GNU ld syntax.
- Target-specific memory profiles are needed for meaningful interpretation of complex memory aliases.
- The RP2350 and ESP32-C3 are the targets validated for the version 0.4 memory profiles.
- RP2350 accounting still reports 8,192 bytes of unclassified memory in the tested firmware.
- Stripped ELF files may not contain enough symbol information for detailed function-size reporting.
- Dependency diagnostics are heuristic and do not replace a complete dependency audit.
- Linker garbage collection, enabled features, and build configuration can change the final binary layout.
- `no_std` detection and related diagnostics are advisory; they do not prove that firmware is suitable for a specific embedded environment.

Memory estimates are not a substitute for linker map files, hardware documentation, runtime stack/heap instrumentation, or verification of the final flashed image.

## Version history

### v0.4.0

**Target-specific memory profiles and embedded validation.**

Version 0.4 introduces explicit memory profiles and validates the analysis workflow against real RP2350 and ESP32-C3 firmware ELF artifacts.

Implemented:

- Added the `--memory-profile` CLI option.
- Added the `rp2350` memory profile.
- Added the `esp32c3` memory profile.
- Added target-specific memory capacity and grouping policies.
- Integrated profile-based memory accounting with ELF analysis.
- Improved linker memory discovery and interpretation.
- Added support for analyzing existing firmware ELF files without rebuilding.
- Integrated target-aware memory reporting into terminal and JSON output.
- Applied memory-group utilization to CI threshold checks.
- Tested the RP2350 profile against the `pico2-riscv` firmware.
- Tested the ESP32-C3 profile against the `esp32c3mini` firmware.

**Validation summary**

| Target | Flash allocation | RAM allocation | Validation result |
|---|---:|---:|---|
| RP2350 | 308,802 B | 18,760 B + 8,192 B unclassified | Profile loaded; RAM mapping remains incomplete |
| ESP32-C3 | 477,420 B | 386,048 B / 395,024 B | Linker-aware accounting; no unclassified RAM |

The RP2350 result explicitly exposes the remaining unclassified memory instead of silently treating the memory as fully accounted for.

The ESP32-C3 result validates the shared SRAM grouping and linker-derived capacity model against the tested ELF and linker artifacts.

These tests validate the analysis workflow and its accounting model. They are not runtime measurements performed on the physical chips.

### v0.3.0

**Linker-aware memory accounting.**

- Added ESP32-C3 linker-aware analysis.
- Added support for generated ESP32-C3 `memory.x` files.
- Added linker expression evaluation.
- Added JSON reporting for memory regions.
- Added build-profile matching for linker files.
- Added address-based section classification.
- Added RAM grouping for aliased regions.
- Improved handling of linker-only dummy reservations.
- Distinguished Flash-backed content from runtime RAM allocations.
- Added alias-aware RAM utilization for CI thresholds.

### v0.2.0

**Improved ELF memory accounting.**

- Added `PT_LOAD` program header inspection.
- Improved VMA and LMA handling.
- Added `SHT_NOBITS` support.
- Improved section flag interpretation.
- Avoided double counting between sections and segments.
- Improved linker section handling.
- Added fixture-based tests.

### v0.1.0

**Initial release.**

- Initial Cargo subcommand.
- Cargo metadata inspection.
- ELF inspection.
- Basic Flash and RAM estimates.
- Dependency diagnostics.
- JSON output.
- `--no-build` support.
- Explicit `--binary` support.

## Roadmap

The core v0.4 target-profile workflow is implemented and tested against RP2350 and ESP32-C3 firmware ELF artifacts.

Potential future improvements:

- More target-specific memory profiles.
- More complete linker-script grammar support.
- Better stack and heap reservation interpretation.
- Binary size regression tracking between builds.
- CI comparisons across commits.
- More detailed function-size analysis.
- Additional Cargo feature and dependency diagnostics.
- Build health reporting.
- Machine-readable dashboards and historical reports.

## Contributing

Contributions are welcome.

For target-specific memory profile improvements, useful test material includes:

- Target triple.
- ELF firmware artifact.
- Linker script or generated `memory.x`.
- Cargo build profile.
- Expected memory regions and capacities.
- `cargo-bin-health` JSON report.

Real embedded ELF artifacts are especially useful for validating memory accounting and preventing regressions.

## License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.
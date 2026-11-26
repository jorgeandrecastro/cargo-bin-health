# cargo-bin-health

[![Crates.io](https://img.shields.io/crates/v/cargo-bin-health.svg)](https://crates.io/crates/cargo-bin-health)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/License-MIT%20OR%20Apache--2.0-blue.svg)](LICENSE-APACHE)
[![Documentation](https://docs.rs/cargo-bin-health/badge.svg)](https://docs.rs/cargo-bin-health)

`cargo-bin-health` is a Cargo subcommand for inspecting compiled Rust executables and embedded firmware.

It analyzes ELF binaries, estimates Flash and RAM allocation, reports function symbol sizes when available, inspects linker-defined memory regions, and identifies potential dependency and build configuration issues.

The project targets embedded Rust developers who need visibility into binary size, memory allocation, and build health during local development and CI.

## Why cargo-bin-health?

A successful compilation does not guarantee that a firmware fits within its target's memory constraints.

Binary size, linker reservations, memory placement, and dependency configuration all affect the final firmware.

`cargo-bin-health` provides a repeatable report to help developers:

- Inspect compiled ELF artifacts.
- Estimate Flash and RAM allocation.
- Identify the largest function symbols when symbol information is available.
- Inspect linker-defined memory regions.
- Detect duplicate dependency versions.
- Enforce memory thresholds in CI workflows.
- Export structured JSON reports.

## Installation

Install from crates.io:

```sh
cargo install cargo-bin-health
```

Run it as a Cargo subcommand:

```sh
cargo bin-health --release
```

You can also invoke the executable directly:

```sh
cargo-bin-health --help
```

## Usage

### Inspect a firmware

```sh
cargo bin-health --release
```

### Inspect a specific target

```sh
cargo bin-health \
    --target thumbv7em-none-eabihf \
    --release \
    --top 20
```

### Inspect an existing ELF without rebuilding

```sh
cargo bin-health \
    --no-build \
    --binary target/thumbv7em-none-eabihf/release/firmware \
    --json
```

### Inspect a firmware from another Cargo project

```sh
cargo bin-health \
    --manifest-path ../esp32c3mini/Cargo.toml \
    --binary ../esp32c3mini/target/riscv32imc-unknown-none-elf/release/esp32c3mini \
    --no-build \
    --json
```

This mode is useful when the firmware has already been compiled by another toolchain or build workflow.

### CI usage

Configure Flash and RAM thresholds:

```sh
cargo bin-health \
    --release \
    --max-flash-pct 80 \
    --max-ram-pct 80 \
    --ci
```

The `--ci` option enables a non-zero exit status when a configured threshold is exceeded.

RAM threshold evaluation uses the highest utilization among the recognized RAM allocation groups. This avoids hiding a nearly exhausted memory pool behind a lower aggregate utilization.

When a RAM threshold is configured but RAM usage cannot be mapped reliably, the CI check fails closed rather than reporting a misleading successful result.

### JSON output

```sh
cargo bin-health --release --json
```

JSON output is intended for automated analysis, CI pipelines, dashboards, and custom reporting tools.

## Command-line options

| Option | Description |
|---|---|
| `--release` | Build using the release profile |
| `--profile NAME` | Select a Cargo profile |
| `--target TRIPLE` | Select the compilation target |
| `--manifest-path PATH` | Select a Cargo manifest |
| `--package NAME`, `-p NAME` | Select a Cargo package |
| `--bin NAME` | Select a binary target |
| `--example NAME` | Select an example target |
| `--features a,b` | Enable Cargo features |
| `--no-default-features` | Disable default features |
| `--no-build` | Inspect an existing artifact without building |
| `--binary PATH` | Explicitly select an ELF artifact |
| `--top N` | Display the N largest function symbols |
| `--max-flash-pct N` | Set a Flash usage threshold |
| `--max-ram-pct N` | Set a RAM usage threshold, evaluated against the most utilized RAM group |
| `--ci` | Return a non-zero status when a configured threshold is exceeded |
| `--json` | Emit machine-readable JSON |
| `--no-color` | Disable ANSI color output |

Run `cargo bin-health --help` for the complete list of supported options.

## ESP32-C3 real-world test

`cargo-bin-health` has been tested against a compiled ESP32-C3 firmware built for the RISC-V architecture.

### Test environment

| Property | Value |
|---|---|
| Project | `esp32c3mini` |
| Architecture | RISC-V 32-bit |
| Target | `riscv32imc-unknown-none-elf` |
| Build profile | `release` |
| Artifact | ELF executable |
| Analysis mode | Existing binary, without rebuilding |

The firmware was inspected using:

```sh
cargo bin-health \
    --manifest-path ~/projets/esp32c3mini/Cargo.toml \
    --binary ~/projets/esp32c3mini/target/riscv32imc-unknown-none-elf/release/esp32c3mini \
    --no-build \
    --json
```

### Results

The current implementation successfully:

- Loaded Cargo metadata from an external project.
- Located and inspected an existing RISC-V ELF executable.
- Extracted ELF section names, addresses, and sizes.
- Analyzed allocated ELF sections.
- Classified sections using ESP32-C3 linker address windows.
- Located the generated release `memory.x` linker script.
- Parsed linker memory regions, including origin, length, and end addresses.
- Applied target-specific RAM grouping and Flash capacity policies.
- Generated a structured JSON report.
- Detected duplicate dependency versions in the resolved Cargo package graph.

Example measurements from the tested ELF:

| Metric | Reported value |
|---|---:|
| ELF architecture | RISC-V 32-bit |
| ELF Flash allocation estimate | 477,420 bytes |
| ELF RAM allocation estimate | 386,048 bytes |
| Flash capacity from linker model | 4,194,272 bytes |
| RAM capacity from linker model | 395,024 bytes |
| Highest RAM group utilization | 99.80% |
| ELF accounting method | `allocated_sections_with_linker_region_mapping` |
| Generated release `memory.x` | Detected |

The reported values describe the analyzed ELF and its linker configuration. They are not measurements of runtime stack high-water usage or a complete physical memory utilization report.

Function symbol reporting depends on the symbol information available in the ELF. Stripped binaries may not provide enough information to identify the largest functions.

### Linker memory information

The generated ESP32-C3 linker configuration contains several memory regions, including:

- `IRAM`
- `DRAM`
- `IROM`
- `DROM`
- `ICACHE`
- `RTC_FAST`
- `dram2_seg`

The parser exposes the discovered regions and their properties in the JSON report.

For the ESP32-C3 memory model:

- `IRAM`, `DRAM`, and `dram2_seg` are grouped into a shared internal SRAM allocation group.
- `RTC_FAST` is accounted for separately.
- `ICACHE` is excluded from the RAM capacity model.
- Flash capacity is based on the largest recognized Flash window instead of summing the IROM and DROM windows.

For the tested configuration, the modeled RAM capacity is 395,024 bytes: a shared internal SRAM group of 386,832 bytes and a separate `RTC_FAST` region of 8,192 bytes.

**Important:** these values represent a linker-derived accounting model. They must not be interpreted as a complete statement of the ESP32-C3's physical silicon memory capacity. Region aliases and address windows require target-specific interpretation.

## Memory accounting (v0.3.0)

Version 0.3.0 improves ELF accounting and introduces linker-aware memory inspection.

### ELF accounting

The accounting pipeline uses allocated ELF sections as its primary source and retains `PT_LOAD` program segments as supporting metadata.

It considers:

- ELF section allocation flags.
- Section types, including `SHT_NOBITS`.
- Section virtual and load address information where available.
- File-backed data versus runtime allocation.
- Target-specific section placement.
- Linker reservation sections that should not be counted as firmware payload.

The implementation avoids blindly adding section sizes and program segment sizes together, which would double-count parts of the same binary.

### Flash and RAM estimates

The tool distinguishes between:

- File-backed allocated sections contributing to the Flash image estimate.
- Runtime sections contributing to RAM allocation.
- Initialized data whose initial contents are stored in Flash and copied to RAM during startup.
- Uninitialized sections that occupy runtime memory without corresponding file-backed payload.

For ESP32-C3, sections can be classified using linker address windows such as IRAM, DRAM, RTC_FAST, IROM, and DROM.

Linker-only reservation sections such as `.rwdata_dummy` and `.rotext_dummy` are excluded from payload accounting.

The reported RAM allocation is not runtime stack usage. For example, a `.stack` section represents a reservation, not evidence that the entire stack has been used.

### RAM capacity groups and aliases

Some linker memory regions can represent overlapping or related views of the same physical memory pool. Summing every declared region independently can therefore produce an incorrect capacity estimate.

The ESP32-C3 model groups recognized regions before evaluating RAM utilization:

- `IRAM`, `DRAM`, and `dram2_seg` belong to the shared internal SRAM group.
- `RTC_FAST` remains a separate group.
- RAM threshold checks use the highest utilization among the recognized groups.

This is also the value used by `--max-ram-pct` for CI threshold evaluation.

The grouping is target-specific. It should not be assumed to apply to other microcontrollers without an appropriate memory profile.

### Linker memory parsing

The linker-script reader supports common GNU ld-style `MEMORY` declarations.

Supported constructs include:

- Named memory regions.
- Hexadecimal addresses.
- Decimal values.
- `K` and `M` size suffixes.
- Basic addition and subtraction.
- Parenthesized arithmetic expressions.
- `ORIGIN(NAME)` and `LENGTH(NAME)` references.
- Region origin, length, and end extraction.
- Region length expressions using `len =`.

Example expressions:

```ld
IRAM : ORIGIN = 0x4037C000 + 0x4000,
       LENGTH = 313K - 0x4000
```

The parser searches project linker scripts and generated Cargo build output, including generated `memory.x` files under target build directories.

Generated scripts are selected with the ELF build profile in mind to avoid mixing debug and release linker configurations.

This is a focused parser for common memory declarations, not a complete implementation of the GNU ld linker-script grammar.

### Interpretation

The reported values are estimates derived from ELF and linker information.

They should not be treated as a replacement for:

- The target linker's complete map file.
- Hardware-specific memory documentation.
- Runtime stack usage analysis.
- Runtime heap instrumentation.
- Final flashed image inspection.

## Limitations

- ELF is required. PE, Mach-O, raw binary, and UF2 formats are not currently parsed.
- Automatic artifact discovery supports conventional Cargo output paths. Custom runners, build scripts, and renamed artifacts may require `--binary`.
- Linker-script parsing supports common `MEMORY` declarations and simple expressions, not the complete linker-script language.
- Memory interpretation is target-dependent. ESP32-C3 handling is more developed than generic handling for other embedded architectures.
- Memory region aliases and separate address windows require target-specific capacity policies.
- Stripped ELF files may not contain usable function symbols.
- Dependency diagnostics are heuristic. A dependency in Cargo metadata does not prove that its code contributes to the final executable.
- Linker garbage collection and Cargo feature unification can affect the final binary.
- `no_std` detection is advisory. Cargo metadata alone cannot prove that every linked dependency is `no_std`.

## Roadmap

### v0.1 — Initial implementation

- Cargo subcommand.
- Cargo metadata inspection.
- ELF parsing.
- Basic section-based Flash/RAM estimates.
- Dependency diagnostics.
- JSON output.
- Existing artifact inspection with `--no-build` and `--binary`.

### v0.2 — Improved ELF accounting

- Interpret ELF program headers (`PT_LOAD`).
- Distinguish VMA and LMA information.
- Handle `SHT_NOBITS`.
- Interpret section allocation and write/execute flags.
- Avoid double-counting sections and loadable segments.
- Improve custom linker section handling.
- Add regression tests using ELF fixtures.

### v0.3.0 — ESP32-C3 linker-aware analysis

- Parse generated esp-hal `memory.x` files.
- Parse linker region expressions.
- Expose memory regions in JSON.
- Match linker scripts to the ELF build profile.
- Classify ESP32-C3 sections using IRAM, DRAM, RTC_FAST, IROM, and DROM address windows.
- Group recognized RAM regions using a target-specific capacity model.
- Exclude linker-only dummy reservations from payload totals.
- Distinguish Flash-backed data from runtime RAM allocation.
- Add alias-aware RAM threshold evaluation for CI.

### v0.4 — Target memory profiles

- Introduce a more extensible target memory profile architecture.
- Add RP2040 and RP2350 memory profiles.
- Add ARM Cortex-M memory profiles.
- Expand target-specific alias and capacity policies.
- Improve linker-script discovery and matching.
- Expand regression tests with real embedded ELF fixtures.

### Future — Binary and CI analysis

- Binary size regression baselines.
- CI comparison between builds.
- Improved function-level size analysis.
- Better Cargo feature and dependency diagnostics.
- Additional build configuration health checks.
- Machine-readable reports for external dashboards.

## Contributing

Contributions, bug reports, and reproducible ELF test cases are welcome.

For target-specific accounting issues, include:

- The target triple.
- The ELF artifact.
- The linker script or generated `memory.x`.
- The build profile.
- The relevant JSON report.

These details make memory accounting issues reproducible.

## License

Licensed under either of:

- Apache License, Version 2.0 (`LICENSE-APACHE`)
- MIT license (`LICENSE-MIT`)

at your option.
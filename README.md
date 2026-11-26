# cargo-bin-health

[![Crates.io](https://img.shields.io/crates/v/cargo-bin-health.svg)](https://crates.io/crates/cargo-bin-health)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/License-MIT%20or%20Apache--2.0-blue.svg)](https://opensource.org/licenses/MIT)
[![docs.rs](https://docs.rs/cargo-bin-health/badge.svg)](https://docs.rs/cargo-bin-health)

`cargo-bin-health` is a Cargo subcommand that inspects compiled Rust executables and firmware. It analyzes ELF sections, estimates Flash/RAM usage, reports the largest function symbols when available, and identifies selected dependency and build configuration issues.

The project is designed for embedded Rust developers who need visibility into binary size, memory usage, and build health.

## Why cargo-bin-health?

Embedded memory budgets are easy to exceed without noticing.

A successful compilation does not necessarily mean that a firmware image fits within its target's memory constraints. `cargo-bin-health` provides a repeatable local and CI-oriented report to help developers inspect compiled artifacts and identify potential issues early.

## Installation

```sh
cargo install cargo-bin-health
```

Run it from a Cargo package:

```sh
cargo bin-health --release
cargo bin-health --target thumbv7em-none-eabihf --release --top 20
cargo bin-health --json --no-build --binary target/thumbv7em-none-eabihf/release/firmware
```

The executable can also be invoked directly:

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

This is useful when the artifact has already been built by another toolchain or build workflow.

## Options

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
| `--max-ram-pct N` | Set a RAM usage threshold |
| `--ci` | Return a non-zero status when a configured threshold is exceeded |
| `--json` | Emit machine-readable JSON |
| `--no-color` | Disable ANSI color output |

Run `cargo bin-health --help` for the complete list of options.

## ESP32-C3 real-world test

`cargo-bin-health` has been tested against a compiled ESP32-C3 firmware.

**Test environment**

- Project: `esp32c3mini`
- Architecture: RISC-V 32-bit
- Target: `riscv32imc-unknown-none-elf`
- Build profile: `release`
- Artifact: ELF executable
- Analysis mode: existing binary, without rebuilding

The firmware was inspected using:

```sh
cargo bin-health \
  --manifest-path ~/projets/esp32c3mini/Cargo.toml \
  --binary ~/projets/esp32c3mini/target/riscv32imc-unknown-none-elf/release/esp32c3mini \
  --no-build \
  --json
```

### Results

The tool successfully:

- Loaded Cargo metadata from an external project.
- Located and inspected an existing RISC-V ELF executable.
- Extracted ELF section names, addresses, and sizes.
- Generated a structured JSON report.
- Detected duplicate dependency versions in the resolved Cargo package graph.

Example measurements reported by the current implementation:

| Metric | Reported value |
|---|---:|
| ELF architecture | RISC-V 32-bit |
| ELF sections detected | 14 |
| Estimated Flash usage | 397,288 bytes |
| Estimated RAM usage | 58,408 bytes |
| Largest symbols | Not available (stripped ELF) |
| `memory.x` information | Not available |

**Important:** these Flash/RAM figures are preliminary estimates from the current section-based accounting implementation. They are not yet authoritative measurements of ESP32-C3 memory consumption. Some target-specific sections and linker placement rules are not fully interpreted yet.

The test validates the CLI workflow and ELF inspection pipeline, not the accuracy of the memory accounting.

## Memory accounting

The current implementation uses ELF section information to estimate memory usage.

It recognizes common sections such as:

- `.text`
- `.rodata`
- `.data`
- `.bss`
- `.init`
- `.fini`
- vector sections
- common uninitialized data sections

Initialized `.data` can occupy both Flash and RAM: its initial contents are stored in non-volatile memory and copied into RAM during startup.

When a simple GNU ld-style `MEMORY` declaration is found, the tool can also read memory region capacities from `memory.x`.

This accounting is currently an estimate, not a replacement for the target linker's map file.

The next development milestone is to improve ELF accounting by interpreting load and execution addresses, allocated sections, program headers, and target-specific linker layouts.

## Limitations

- ELF is required. PE, Mach-O, raw binary, and UF2 formats are not currently parsed.
- Automatic artifact discovery supports conventional Cargo output paths. Custom target runners, build scripts, and renamed artifact paths may require `--binary`.
- `memory.x` parsing handles common `FLASH`/`RAM` declarations and simple K/M units, not the complete linker-script grammar.
- Memory accounting is not yet architecture-aware. ESP32, RP2040, RP2350, and STM32 linker layouts may require dedicated interpretation.
- Stripped ELF files may not contain usable function symbols.
- Dependency diagnostics are heuristic. A Cargo metadata graph does not prove that a dependency contributes code to the final artifact; linker garbage collection and feature unification affect the result.
- `std` is not normally a package in Cargo's resolved package list, so `no_std` detection is advisory.

## Roadmap

### v0.1  Initial implementation

- Cargo subcommand.
- Cargo metadata inspection.
- ELF parsing.
- Basic section-based Flash/RAM estimates.
- Dependency diagnostics.
- JSON output.
- Existing artifact inspection with `--no-build` and `--binary`.

### v0.2 Accurate ELF accounting

- Interpret ELF program headers (`PT_LOAD`).
- Distinguish VMA and LMA.
- Handle `SHT_NOBITS` correctly.
- Interpret section allocation and write/execute flags.
- Avoid double-counting sections and loadable segments.
- Improve handling of custom linker sections.
- Add regression tests using real ELF fixtures.

### Future milestones

- Architecture-aware memory profiles: RISC-V, ARM Cortex-M, and Xtensa.
- Improved symbol and function-size analysis.
- Better Cargo feature and dependency diagnostics.
- Build configuration health checks.
- CI baselines and binary size regression detection.

## License

Licensed under either of:

- Apache License, Version 2.0 (`LICENSE-APACHE`)
- MIT license (`LICENSE-MIT`)

at your option.
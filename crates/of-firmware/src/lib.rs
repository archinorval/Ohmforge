//! Firmware loading: ELF images (now), flash images, UF2 and DWARF symbols
//! (later).
//!
//! [`elf::ElfImage::parse`] reads a 32-bit little-endian RISC-V ELF
//! executable as produced by ESP-IDF, Arduino-ESP32, esp-hal or a bare
//! assembler: the entry point, the loadable segments and the symbol table.

pub mod elf;

pub use elf::{ElfError, ElfImage, Segment, Symbol};

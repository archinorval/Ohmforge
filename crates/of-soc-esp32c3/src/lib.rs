//! ESP32-C3 SoC: memory map, peripherals, ROM hooks.
//!
//! [`Esp32c3`] wires an [`of_cpu_riscv::Hart`] to [`SocBus`], the C3 memory
//! map (ROM, SRAM through its IRAM and DRAM windows, the flash cache windows
//! and RTC fast memory) and a first set of peripherals: GPIO with a minimal
//! IO MUX and GPIO matrix, UART0 transmit, and SYSTIMER unit 0.
//! [`Esp32c3Engine`] runs it under the [`of_core::Kernel`] and turns pin
//! changes into events for the analog engine.
//!
//! Firmware is loaded straight from an ELF ([`Esp32c3::load_elf`]); the
//! boot ROM is not modelled yet (direct boot).
//!
//! Register addresses follow the ESP32-C3 Technical Reference Manual. Reset
//! values and behaviour of anything not listed in the module docs are
//! approximations: unmodelled peripheral registers read back what was last
//! written and are counted in [`SocBus::unmodelled`].

pub mod bus;
pub mod engine;
pub mod gpio;
pub mod map;
pub mod soc;
pub mod systimer;
pub mod uart;

pub use bus::SocBus;
pub use engine::{Esp32c3Engine, PinLink, all_pin_codes, decode_pin_code, pin_code};
pub use gpio::{GPIO_COUNT, PinOut};
pub use soc::{Esp32c3, LoadError, PinEdge, UartByte};

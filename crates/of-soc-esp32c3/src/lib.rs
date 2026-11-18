//! ESP32-C3 SoC: memory map, peripherals, direct ELF boot and the kernel engine.
//!
//! [`Esp32c3`] wires an [`of_cpu_riscv::Hart`] to [`SocBus`], the C3 memory
//! map (ROM, SRAM through its IRAM and DRAM windows, the flash cache windows
//! and RTC fast memory) and its peripherals: GPIO with a minimal IO MUX and
//! GPIO matrix, UART0 transmit, SYSTIMER (counters and alarms), timer groups
//! TIMG0/1 (timer and MWDT), the RTC and super watchdogs, and the interrupt
//! matrix with the CPU interrupt controller. Timer and watchdog events stop
//! the hart at their exact time, so interrupts and resets land where they
//! would on hardware (to within one instruction).
//! [`Esp32c3Engine`] runs it under the [`of_core::Kernel`] and turns pin
//! changes into events for the analog engine.
//!
//! Firmware is loaded straight from an ELF ([`Esp32c3::load_elf`]) or an ESP
//! flash image ([`Esp32c3::load_flash_image`]); the boot ROM is not modelled
//! yet (direct boot).
//!
//! Register addresses follow the ESP32-C3 Technical Reference Manual. Reset
//! values and behaviour of anything not listed in the module docs are
//! approximations: unmodelled peripheral registers read back what was last
//! written and are counted in [`SocBus::unmodelled`].

pub mod bus;
pub mod engine;
pub mod gpio;
pub mod intmatrix;
pub mod map;
pub mod rtc_cntl;
pub mod soc;
pub mod systimer;
pub mod timg;
pub mod uart;
pub mod watchdog;

pub use bus::SocBus;
pub use engine::{Esp32c3Engine, PinLink, all_pin_codes, decode_pin_code, pin_code};
pub use gpio::{GPIO_COUNT, PinOut};
pub use intmatrix::IntMatrix;
pub use soc::{CHIP_ID, Esp32c3, LoadError, PinEdge, UartByte};
pub use watchdog::{ResetKind, ResetSource, WatchdogReset};

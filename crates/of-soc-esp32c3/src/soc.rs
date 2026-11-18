//! The ESP32-C3 as a whole: hart, bus and time-stamped outputs.

use of_core::SimTime;
use of_cpu_riscv::{BusError, Hart, HartConfig, RunExit};
use of_firmware::ElfImage;
use thiserror::Error;

use crate::bus::SocBus;
use crate::gpio::{GPIO_COUNT, PinOut};

/// A pin output change, stamped with the hart time of the instruction
/// that caused it (the end of that instruction).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PinEdge {
    /// When.
    pub at: SimTime,
    /// GPIO number.
    pub pin: u8,
    /// New output state.
    pub out: PinOut,
}

/// A byte written to UART0, stamped like [`PinEdge`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UartByte {
    /// When.
    pub at: SimTime,
    /// The byte.
    pub byte: u8,
}

/// Why firmware could not be loaded.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum LoadError {
    /// A segment does not fit any ESP32-C3 memory.
    #[error("segment at {addr:#010x} ({len} bytes) is outside the ESP32-C3 memory map")]
    Unmapped {
        /// Segment address.
        addr: u32,
        /// Segment size in memory.
        len: u32,
    },
}

/// An ESP32-C3: one RV32IMC hart, its memory map and peripherals.
#[derive(Debug, Clone)]
pub struct Esp32c3 {
    /// The CPU core.
    pub hart: Hart,
    /// Memories and peripherals.
    pub bus: SocBus,
    pins: [PinOut; GPIO_COUNT],
    edges: Vec<PinEdge>,
    uart: Vec<UartByte>,
}

impl Default for Esp32c3 {
    fn default() -> Self {
        Self::new()
    }
}

impl Esp32c3 {
    /// Power-on state. The PC is the ROM reset vector until firmware is
    /// loaded.
    pub fn new() -> Self {
        let bus = SocBus::new();
        let pins = std::array::from_fn(|p| bus.gpio.pin_out(p));
        Self {
            hart: Hart::new(HartConfig::esp32c3()),
            bus,
            pins,
            edges: Vec::new(),
            uart: Vec::new(),
        }
    }

    /// Direct boot from an ELF: copy every segment into its memory (`.bss`
    /// zero-filled) and jump to the entry point, as if the ROM bootloader
    /// had done it.
    pub fn load_elf(&mut self, img: &ElfImage) -> Result<(), LoadError> {
        for s in &img.segments {
            let mut bytes = s.data.clone();
            bytes.resize(s.mem_size as usize, 0);
            self.bus
                .load(s.vaddr, &bytes)
                .map_err(|BusError { .. }| LoadError::Unmapped {
                    addr: s.vaddr,
                    len: s.mem_size,
                })?;
        }
        self.hart.set_pc(img.entry);
        Ok(())
    }

    /// Run one stretch: until `deadline`, a sync point or a stop. Sync
    /// points are handled (pin edges and UART bytes stamped, SYSTIMER
    /// latched) before returning.
    pub fn run_slice(&mut self, deadline: SimTime) -> RunExit {
        let exit = self.hart.run_until(&mut self.bus, deadline);
        if let RunExit::SyncRequested(_) = exit {
            self.sync();
        }
        exit
    }

    /// Run until `deadline` or a stop other than a sync point.
    pub fn run_until(&mut self, deadline: SimTime) -> RunExit {
        loop {
            match self.run_slice(deadline) {
                RunExit::SyncRequested(_) => {}
                other => return other,
            }
        }
    }

    fn sync(&mut self) {
        let now = self.hart.local_time();
        self.bus.systimer.latch(now);
        for byte in self.bus.take_uart_tx() {
            self.uart.push(UartByte { at: now, byte });
        }
        if self.bus.take_pins_dirty() {
            for (pin, old) in self.pins.iter_mut().enumerate() {
                let new = self.bus.gpio.pin_out(pin);
                if new != *old {
                    *old = new;
                    self.edges.push(PinEdge {
                        at: now,
                        pin: pin as u8,
                        out: new,
                    });
                }
            }
        }
    }

    /// Current output state of `pin`.
    pub fn pin(&self, pin: usize) -> PinOut {
        self.pins[pin]
    }

    /// All pin edges so far.
    pub fn pin_edges(&self) -> &[PinEdge] {
        &self.edges
    }

    /// Remove and return the pin edges recorded so far.
    pub fn take_pin_edges(&mut self) -> Vec<PinEdge> {
        std::mem::take(&mut self.edges)
    }

    /// Everything written to UART0 so far.
    pub fn uart_bytes(&self) -> &[UartByte] {
        &self.uart
    }

    /// UART0 output as text (lossy UTF-8).
    pub fn uart_text(&self) -> String {
        String::from_utf8_lossy(&self.uart.iter().map(|b| b.byte).collect::<Vec<_>>()).into_owned()
    }
}

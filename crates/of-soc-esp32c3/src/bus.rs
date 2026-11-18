//! The ESP32-C3 system bus: memories and peripheral dispatch.

use std::collections::BTreeMap;

use of_cpu_riscv::{AccessKind, Bus, BusError, Size};

use crate::gpio::Gpio;
use crate::map::{self, within};
use crate::systimer::Systimer;
use crate::uart;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mem {
    RomI,
    RomD,
    Sram,
    FlashI,
    FlashD,
    Rtc,
}

/// Memories plus peripherals, as seen from the hart.
#[derive(Debug, Clone)]
pub struct SocBus {
    rom: Vec<u8>,
    sram: Vec<u8>,
    flash_i: Vec<u8>,
    flash_d: Vec<u8>,
    rtc: Vec<u8>,
    /// GPIO, GPIO matrix and IO MUX.
    pub gpio: Gpio,
    /// SYSTIMER unit 0.
    pub systimer: Systimer,
    uart_tx: Vec<u8>,
    regs: BTreeMap<u32, u32>,
    unmodelled: BTreeMap<u32, u64>,
    sync: bool,
    pins_dirty: bool,
}

impl Default for SocBus {
    fn default() -> Self {
        Self::new()
    }
}

/// Flash cache windows are backed lazily: this much is allocated up front.
const FLASH_WINDOW_BYTES: usize = 4 << 20;

impl SocBus {
    /// Power-on state: zeroed memories, peripherals at reset.
    pub fn new() -> Self {
        Self {
            rom: vec![0; map::IROM_INTERNAL.1 as usize],
            sram: vec![0; map::IRAM.1 as usize],
            flash_i: vec![0; FLASH_WINDOW_BYTES],
            flash_d: vec![0; FLASH_WINDOW_BYTES],
            rtc: vec![0; map::RTC_FAST.1 as usize],
            gpio: Gpio::new(),
            systimer: Systimer::default(),
            uart_tx: Vec::new(),
            regs: BTreeMap::new(),
            unmodelled: BTreeMap::new(),
            sync: false,
            pins_dirty: false,
        }
    }

    fn locate(&self, addr: u32, len: u32) -> Option<(Mem, usize)> {
        let table = [
            (Mem::RomI, map::IROM_INTERNAL),
            // DROM_INTERNAL is ROM1 (the upper 128 KiB of the ROM).
            (Mem::RomD, map::DROM_INTERNAL),
            (Mem::Sram, map::IRAM),
            (Mem::Sram, map::DRAM),
            (Mem::FlashI, map::IROM_FLASH),
            (Mem::FlashD, map::DROM_FLASH),
            (Mem::Rtc, map::RTC_FAST),
        ];
        let (m, region) = table.into_iter().find(|&(_, r)| within(r, addr, len))?;
        let mut off = (addr - region.0) as usize;
        if m == Mem::RomD {
            off += 0x4_0000;
        }
        let backing = self.mem(m).len();
        (off + len as usize <= backing).then_some((m, off))
    }

    fn mem(&self, m: Mem) -> &[u8] {
        match m {
            Mem::RomI | Mem::RomD => &self.rom,
            Mem::Sram => &self.sram,
            Mem::FlashI => &self.flash_i,
            Mem::FlashD => &self.flash_d,
            Mem::Rtc => &self.rtc,
        }
    }

    fn mem_mut(&mut self, m: Mem) -> &mut [u8] {
        match m {
            Mem::RomI | Mem::RomD => &mut self.rom,
            Mem::Sram => &mut self.sram,
            Mem::FlashI => &mut self.flash_i,
            Mem::FlashD => &mut self.flash_d,
            Mem::Rtc => &mut self.rtc,
        }
    }

    /// Back-door write used by loaders: also fills ROM and the flash cache
    /// windows, which the hart cannot write.
    pub fn load(&mut self, addr: u32, bytes: &[u8]) -> Result<(), BusError> {
        let (m, off) = self.locate(addr, bytes.len() as u32).ok_or(BusError { addr })?;
        self.mem_mut(m)[off..off + bytes.len()].copy_from_slice(bytes);
        Ok(())
    }

    /// Read memory without side effects (peripherals excluded).
    pub fn peek(&self, addr: u32, len: usize) -> Option<&[u8]> {
        let (m, off) = self.locate(addr, len as u32)?;
        Some(&self.mem(m)[off..off + len])
    }

    /// Bytes written to UART0 since the last [`Self::take_uart_tx`].
    pub fn take_uart_tx(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.uart_tx)
    }

    /// Whether GPIO output state may have changed since the last call.
    pub fn take_pins_dirty(&mut self) -> bool {
        std::mem::take(&mut self.pins_dirty)
    }

    /// Accesses to peripheral registers that are not modelled, by word
    /// address, with access counts. Useful to see what firmware expects.
    pub fn unmodelled(&self) -> &BTreeMap<u32, u64> {
        &self.unmodelled
    }

    fn periph_read(&mut self, addr: u32) -> u32 {
        let block = addr & !(map::BLOCK - 1);
        let off = addr - block;
        let v = match block {
            map::GPIO_BASE => self.gpio.read(off),
            map::IO_MUX_BASE => self.gpio.io_mux_read(off),
            map::SYSTIMER_BASE => self.systimer.read(off),
            map::UART0_BASE => match off {
                uart::reg::FIFO | uart::reg::STATUS => Some(0),
                _ => None,
            },
            _ => None,
        };
        v.unwrap_or_else(|| {
            *self.unmodelled.entry(addr).or_default() += 1;
            self.regs.get(&addr).copied().unwrap_or(0)
        })
    }

    fn periph_write(&mut self, addr: u32, v: u32) {
        let block = addr & !(map::BLOCK - 1);
        let off = addr - block;
        let handled = match block {
            map::GPIO_BASE => self.gpio.write(off, v).map(|()| self.pins_dirty = true),
            map::IO_MUX_BASE => self.gpio.io_mux_write(off, v).map(|()| self.pins_dirty = true),
            map::SYSTIMER_BASE => self.systimer.write(off, v).map(|_| ()),
            map::UART0_BASE if off == uart::reg::FIFO => {
                self.uart_tx.push(v as u8);
                Some(())
            }
            _ => None,
        };
        if handled.is_none() {
            *self.unmodelled.entry(addr).or_default() += 1;
            self.regs.insert(addr, v);
        }
    }
}

fn get(bytes: &[u8], size: Size) -> u32 {
    match size {
        Size::Byte => u32::from(bytes[0]),
        Size::Half => u32::from(u16::from_le_bytes([bytes[0], bytes[1]])),
        Size::Word => u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
    }
}

impl Bus for SocBus {
    fn read(&mut self, addr: u32, size: Size, kind: AccessKind) -> Result<u32, BusError> {
        let n = size.bytes();
        if within(map::PERIPHERALS, addr, n) {
            if kind == AccessKind::Fetch {
                return Err(BusError { addr });
            }
            self.sync = true;
            let word = self.periph_read(addr & !3);
            let shift = 8 * (addr & 3);
            return Ok(match size {
                Size::Byte => (word >> shift) & 0xff,
                Size::Half => (word >> shift) & 0xffff,
                Size::Word => word,
            });
        }
        let (m, off) = self.locate(addr, n).ok_or(BusError { addr })?;
        if kind == AccessKind::Fetch && matches!(m, Mem::RomD | Mem::FlashD) {
            return Err(BusError { addr });
        }
        if kind == AccessKind::Fetch && m == Mem::Sram && within(map::DRAM, addr, n) {
            return Err(BusError { addr });
        }
        Ok(get(&self.mem(m)[off..], size))
    }

    fn write(&mut self, addr: u32, size: Size, value: u32, _kind: AccessKind) -> Result<(), BusError> {
        let n = size.bytes();
        if within(map::PERIPHERALS, addr, n) {
            self.sync = true;
            let shift = 8 * (addr & 3);
            self.periph_write(addr & !3, value << shift);
            return Ok(());
        }
        let (m, off) = self.locate(addr, n).ok_or(BusError { addr })?;
        if !matches!(m, Mem::Sram | Mem::Rtc) {
            return Err(BusError { addr });
        }
        self.mem_mut(m)[off..off + n as usize].copy_from_slice(&value.to_le_bytes()[..n as usize]);
        Ok(())
    }

    fn take_sync_request(&mut self) -> bool {
        std::mem::take(&mut self.sync)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iram_and_dram_alias_the_same_sram() {
        let mut b = SocBus::new();
        b.write(0x3fc8_0000, Size::Word, 0xdead_beef, AccessKind::Store)
            .unwrap();
        assert_eq!(b.read(0x4038_0000, Size::Word, AccessKind::Fetch), Ok(0xdead_beef));
        assert_eq!(b.read(0x4038_0002, Size::Half, AccessKind::Load), Ok(0xdead));
        assert_eq!(b.read(0x3fc8_0000, Size::Byte, AccessKind::Load), Ok(0xef));
        assert!(
            b.read(0x3fc8_0000, Size::Word, AccessKind::Fetch).is_err(),
            "DRAM is not executable"
        );
        // Top of SRAM.
        b.write(0x3fcd_fffc, Size::Word, 7, AccessKind::Store).unwrap();
        assert!(b.write(0x3fce_0000, Size::Word, 7, AccessKind::Store).is_err());
    }

    #[test]
    fn flash_and_rom_are_read_only_to_the_hart() {
        let mut b = SocBus::new();
        b.load(0x4200_0020, &[0x13, 0, 0, 0]).unwrap();
        b.load(0x3c00_0020, &[1, 2, 3, 4]).unwrap();
        assert_eq!(b.read(0x4200_0020, Size::Word, AccessKind::Fetch), Ok(0x13));
        assert_eq!(b.read(0x3c00_0020, Size::Word, AccessKind::Load), Ok(0x0403_0201));
        assert!(b.write(0x4200_0020, Size::Word, 0, AccessKind::Store).is_err());
        assert!(b.write(0x4000_0000, Size::Word, 0, AccessKind::Store).is_err());
        assert!(b.read(0x3c00_0020, Size::Word, AccessKind::Fetch).is_err());
        // ROM1 through the data bus.
        b.load(0x4004_0000, &[9, 0, 0, 0]).unwrap();
        assert_eq!(b.read(0x3ff0_0000, Size::Word, AccessKind::Load), Ok(9));
        assert!(b.read(0x2000_0000, Size::Word, AccessKind::Load).is_err(), "unmapped");
    }

    #[test]
    fn peripheral_access_requests_sync_and_reaches_gpio() {
        let mut b = SocBus::new();
        assert!(!b.take_sync_request());
        b.write(0x6000_4024, Size::Word, 1 << 2, AccessKind::Store).unwrap();
        b.write(0x6000_4008, Size::Word, 1 << 2, AccessKind::Store).unwrap();
        assert!(b.take_sync_request());
        assert!(!b.take_sync_request(), "taken once");
        assert!(b.take_pins_dirty());
        assert_eq!(b.gpio.pin_out(2).level, Some(true));
        assert_eq!(b.read(0x6000_4004, Size::Word, AccessKind::Load), Ok(1 << 2));
        assert!(b.read(0x6000_4004, Size::Word, AccessKind::Fetch).is_err());
    }

    #[test]
    fn uart_fifo_and_unmodelled_registers() {
        let mut b = SocBus::new();
        for c in b"hi" {
            b.write(0x6000_0000, Size::Word, u32::from(*c), AccessKind::Store)
                .unwrap();
        }
        assert_eq!(b.take_uart_tx(), b"hi");
        // RTC_CNTL watchdog config: not modelled, reads back.
        b.write(0x6000_8090, Size::Word, 0x50d8_3aa1, AccessKind::Store)
            .unwrap();
        assert_eq!(b.read(0x6000_8090, Size::Word, AccessKind::Load), Ok(0x50d8_3aa1));
        assert_eq!(b.unmodelled().get(&0x6000_8090), Some(&2));
    }
}

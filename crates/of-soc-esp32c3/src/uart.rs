//! UART0 transmit (TRM chapter 26), enough for logging.
//!
//! Bytes written to `UART_FIFO` are captured instantly (the FIFO never
//! fills, `UART_STATUS.TXFIFO_CNT` reads 0). Baud-rate timing and the
//! receive path are not modelled yet.

/// Register offsets from [`crate::map::UART0_BASE`].
pub mod reg {
    /// TX/RX FIFO.
    pub const FIFO: u32 = 0x00;
    /// Status: `RXFIFO_CNT` bits 9:0, `TXFIFO_CNT` bits 25:16.
    pub const STATUS: u32 = 0x1c;
}

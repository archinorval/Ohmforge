//! Circuit graph, nets and the SPICE netlist reader.
//!
//! [`spice`] parses the ngspice-compatible netlist subset used by the analog
//! solver and the `ohmforge run` command. Later this crate also partitions
//! nets into digital-only nets (event-driven kernel) and mixed nets
//! (promoted to the MNA analog solver).

pub mod spice;

/// Index of a net. Net 0 is ground.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NetId(pub u32);

impl NetId {
    /// The ground reference net.
    pub const GROUND: Self = Self(0);
}

/// How a net is solved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetDomain {
    /// Only digital endpoints: solved by the event-driven logic kernel.
    Digital,
    /// At least one analog endpoint: solved by the MNA solver.
    Analog,
}

/// Placeholder netlist container.
#[derive(Debug, Default)]
pub struct Netlist {
    /// Number of nets including ground.
    pub net_count: u32,
}

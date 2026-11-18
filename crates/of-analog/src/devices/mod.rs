//! Device models.

pub mod bjt;
pub mod diode;
pub mod limit;
pub mod mosfet;
pub mod passive;
pub mod sources;

use std::collections::BTreeMap;

pub use bjt::{Bjt, BjtModel};
pub use diode::{Diode, DiodeModel};
pub use mosfet::{MosModel, Mosfet};
pub use passive::{Capacitor, Inductor, Resistor};
pub use sources::{Control, ControlledSource, CurrentSource, SourceValue, VoltageSource};

/// First of `names` present in `p`, else `default`.
pub(crate) fn param(p: &BTreeMap<String, f64>, names: &[&str], default: f64) -> f64 {
    names.iter().find_map(|n| p.get(*n).copied()).unwrap_or(default)
}

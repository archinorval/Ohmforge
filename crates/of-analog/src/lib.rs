//! SPICE-grade analog solver based on Modified Nodal Analysis (MNA).
//!
//! * [`matrix`]: sparse MNA system; faer sparse LU with the symbolic
//!   factorisation computed once and reused for every Newton iteration.
//! * [`circuit`]: the [`Circuit`] container and the [`Device`] trait that
//!   every model implements (setup, stamping, breakpoints).
//! * [`devices`]: R, C (with ESR/ESL), L (with series resistance), coupled
//!   inductors (K), independent V/I sources (DC, AC, PULSE, SIN, EXP, PWL),
//!   linear controlled sources (E, G, F, H), voltage/current-controlled
//!   switches (S, W), diode and LED, Gummel-Poon BJT and level-1/level-3
//!   MOSFETs, all with SPICE3 temperature scaling.
//! * [`analysis`]: DC operating point (Newton-Raphson with junction
//!   limiting, then damped Newton, gmin stepping and source stepping),
//!   single and nested DC sweeps, small-signal AC and transient analysis
//!   (backward Euler start, trapezoidal or Gear-2, LTE-controlled steps,
//!   exact breakpoints).
//! * [`engine`]: [`AnalogEngine`], the transient solver as an `of-core`
//!   engine (time slices, pin drivers, threshold watches).
//! * [`netlist`]: builds a circuit from a parsed SPICE deck.
//!
//! ```
//! use of_analog::{analysis, netlist};
//! let deck = of_netlist::spice::parse_str("divider\nV1 in 0 10\nR1 in out 1k\nR2 out 0 1k\n.op\n").unwrap();
//! let mut built = netlist::build(&deck).unwrap();
//! let op = analysis::operating_point(&mut built.circuit, &built.options).unwrap();
//! assert!((op.get("v(out)").unwrap() - 5.0).abs() < 1e-9);
//! ```

pub mod analysis;
pub mod circuit;
pub mod devices;
pub mod engine;
pub mod integrate;
pub mod matrix;
pub mod netlist;
pub mod results;
pub mod waveform;

pub use analysis::{AnalysisError, Options};
pub use circuit::{Circuit, CircuitError, Device, GROUND, Var};
pub use engine::{AnalogEngine, Edge, EngineConfig, SetParam};
pub use results::{OpPoint, Sweep, Waveforms};

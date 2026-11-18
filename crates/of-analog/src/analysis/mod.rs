//! Analyses: DC operating point, DC sweeps (single and nested), small-signal
//! AC and transient.

mod ac;
mod dc;
mod newton;
pub(crate) mod tran;

pub use ac::ac_analysis;
pub use dc::{DcAxis, OpMethod, dc_sweep, dc_sweep_nested, operating_point};
pub use tran::{TranRequest, transient};

use crate::integrate::Method;

/// Solver options (SPICE `.options` names and defaults).
#[derive(Debug, Clone, PartialEq)]
pub struct Options {
    /// Relative tolerance.
    pub reltol: f64,
    /// Absolute current tolerance (A).
    pub abstol: f64,
    /// Absolute voltage tolerance (V).
    pub vntol: f64,
    /// Conductance in parallel with every pn junction (S).
    pub gmin: f64,
    /// Newton iteration limit for DC operating points.
    pub itl1: usize,
    /// Newton iteration limit for DC sweep points.
    pub itl2: usize,
    /// Newton iteration limit per transient time point.
    pub itl4: usize,
    /// Truncation-error tolerance factor (larger allows bigger steps).
    pub trtol: f64,
    /// Integration method.
    pub method: Method,
    /// Largest Newton step for node voltages in the damped-Newton fallback (V).
    pub damping_step: f64,
    /// Number of gmin-stepping decades.
    pub gmin_steps: u32,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            reltol: 1e-3,
            abstol: 1e-12,
            vntol: 1e-6,
            gmin: 1e-12,
            itl1: 100,
            itl2: 50,
            itl4: 10,
            trtol: 7.0,
            method: Method::Trapezoidal,
            damping_step: 0.5,
            gmin_steps: 10,
        }
    }
}

impl Options {
    /// Apply `.options` values from a netlist (unknown keys are ignored).
    pub fn apply(&mut self, opts: &std::collections::BTreeMap<String, f64>) {
        for (k, &v) in opts {
            match k.as_str() {
                "reltol" => self.reltol = v,
                "abstol" => self.abstol = v,
                "vntol" => self.vntol = v,
                "gmin" => self.gmin = v,
                "itl1" => self.itl1 = v as usize,
                "itl2" => self.itl2 = v as usize,
                "itl4" => self.itl4 = v as usize,
                "trtol" => self.trtol = v,
                "method" => self.method = if v == 2.0 { Method::Gear2 } else { Method::Trapezoidal },
                _ => {}
            }
        }
    }
}

/// Why an analysis failed.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum AnalysisError {
    /// Circuit setup failed.
    #[error(transparent)]
    Circuit(#[from] crate::circuit::CircuitError),
    /// The matrix is singular.
    #[error("singular matrix: check '{0}' (floating node, or a loop of voltage sources/inductors)")]
    Singular(String),
    /// Newton-Raphson did not converge, even with gmin and source stepping.
    #[error("no DC convergence ({0})")]
    NoConvergence(String),
    /// The transient time step became too small.
    #[error("time step too small at t = {time:e} s ({reason})")]
    TimestepTooSmall {
        /// Time where the step collapsed.
        time: f64,
        /// Last failure.
        reason: String,
    },
    /// A named source does not exist (DC sweep).
    #[error("no independent source named '{0}'")]
    UnknownSource(String),
    /// Bad analysis parameters.
    #[error("invalid analysis: {0}")]
    Invalid(String),
}

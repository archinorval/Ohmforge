//! Newton-Raphson iteration on the assembled MNA system.

use crate::circuit::{Circuit, LoadParams, VarKind};
use crate::integrate::{Integrator, States};
use crate::matrix::LinearError;

use super::{AnalysisError, Options};

/// Per-call settings.
#[derive(Debug, Clone, Copy)]
pub(crate) struct NrParams {
    pub load: LoadParams,
    pub max_iter: usize,
    /// Limit node-voltage updates to this size (damped Newton), if set.
    pub damping: Option<f64>,
}

/// Result of a Newton solve.
#[derive(Debug)]
pub(crate) enum NrOutcome {
    Converged(usize),
    NoConvergence,
    Singular(usize),
}

pub(crate) fn singular_error(ckt: &Circuit, var: Option<usize>) -> AnalysisError {
    let name = var
        .and_then(|v| ckt.vars().get(v))
        .map_or_else(|| "unknown".to_string(), |i| i.name.clone());
    AnalysisError::Singular(name)
}

/// Iterate from `x` (variable numbering, `x[0] = 0`) until converged.
pub(crate) fn newton(
    ckt: &mut Circuit,
    x: &mut [f64],
    states: &mut States,
    integ: &Integrator,
    opts: &Options,
    p: NrParams,
) -> NrOutcome {
    let n = x.len();
    let mut rhs = vec![0.0; n];
    let mut lp = p.load;
    lp.gmin = opts.gmin;
    let mut init = lp.init_junctions;
    for iter in 0..p.max_iter.max(1) {
        lp.init_junctions = init;
        let limited = ckt.load(x, &mut rhs, &lp, integ, states);
        let was_init = init;
        init = false;
        let system = ckt.system.as_mut().expect("finalized");
        if let Err(LinearError::Singular(v)) = system.solve(&mut rhs) {
            return NrOutcome::Singular(v.unwrap_or(0));
        }
        if let Some(dv) = p.damping {
            let max = (1..n)
                .filter(|&i| ckt.vars[i].kind == VarKind::Voltage)
                .map(|i| (rhs[i] - x[i]).abs())
                .fold(0.0, f64::max);
            if max > dv {
                let a = dv / max;
                for i in 1..n {
                    rhs[i] = x[i] + a * (rhs[i] - x[i]);
                }
            }
        }
        let mut converged = !limited && !was_init && iter > 0;
        if converged {
            for i in 1..n {
                let (new, old) = (rhs[i], x[i]);
                let abs = if ckt.vars[i].kind == VarKind::Voltage {
                    opts.vntol
                } else {
                    opts.abstol
                };
                if (new - old).abs() > opts.reltol * new.abs().max(old.abs()) + abs {
                    converged = false;
                    break;
                }
            }
        }
        x.copy_from_slice(&rhs);
        x[0] = 0.0;
        if converged {
            return NrOutcome::Converged(iter + 1);
        }
    }
    NrOutcome::NoConvergence
}

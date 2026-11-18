//! Transient analysis with LTE-controlled adaptive steps and breakpoints.

use std::collections::{BTreeMap, VecDeque};

use crate::circuit::{Circuit, LoadParams, Mode, TranSpec};
use crate::integrate::{Integrator, States};
use crate::results::Waveforms;

use super::dc::solve_op;
use super::newton::{NrOutcome, NrParams, newton, singular_error};
use super::{AnalysisError, Options};

/// Transient request (SPICE `.tran tstep tstop [tstart [tmax]] [uic]`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TranRequest {
    /// Output step; also bounds the internal step unless `tmax` is set.
    pub tstep: f64,
    /// Stop time.
    pub tstop: f64,
    /// Results before this time are discarded.
    pub tstart: f64,
    /// Largest internal step (0 = `min(tstep, (tstop - tstart) / 50)`).
    pub tmax: f64,
    /// Skip the operating point; start from `.ic` and device `ic=` values.
    pub uic: bool,
    /// Node initial conditions (`.ic v(node)=...`) used with `uic`.
    pub ic: BTreeMap<String, f64>,
}

fn load_params(mode: Mode, time: f64, tran: TranSpec) -> LoadParams {
    LoadParams {
        mode,
        init_junctions: false,
        src_factor: 1.0,
        gmin: 0.0,
        diag_gmin: 0.0,
        time,
        tran,
        tran_op: false,
    }
}

/// An accepted time point: solution and device states.
struct Point {
    t: f64,
    x: Vec<f64>,
    q: Vec<f64>,
}

/// Polynomial extrapolation through the newest `k + 1` history points to
/// time `t` (the Newton starting guess).
fn predict(hist: &VecDeque<Point>, k: usize, t: f64) -> Vec<f64> {
    let pts: Vec<&Point> = hist.iter().rev().take(k + 1).collect();
    let mut out = vec![0.0; pts[0].x.len()];
    for (i, pi) in pts.iter().enumerate() {
        let mut w = 1.0;
        for (j, pj) in pts.iter().enumerate() {
            if i != j {
                w *= (t - pj.t) / (pi.t - pj.t);
            }
        }
        for (o, v) in out.iter_mut().zip(&pi.x) {
            *o += w * v;
        }
    }
    out
}

/// Charge tolerance floor (SPICE `chgtol`).
const CHGTOL: f64 = 1e-14;

/// Largest step allowed by the truncation error of every integrated charge,
/// from the `(order + 1)`-th divided difference of the charge history
/// (the SPICE3 `CKTterr` estimate, in current units).
fn truncation_step(integ: &Integrator, t_new: f64, states: &States, hist: &VecDeque<Point>, opts: &Options) -> f64 {
    let order = integ.order as usize;
    let pts: Vec<(f64, &[f64])> = std::iter::once((t_new, states.now.as_slice()))
        .chain(hist.iter().rev().take(order + 1).map(|p| (p.t, p.q.as_slice())))
        .collect();
    if pts.len() < order + 2 {
        return f64::INFINITY;
    }
    let factor = integ.error_constant();
    let mut best = f64::INFINITY;
    let mut dd = vec![0.0; order + 2];
    for (k, _) in states.charges.iter().enumerate().filter(|(_, c)| **c) {
        for (d, p) in dd.iter_mut().zip(&pts) {
            *d = p.1[k];
        }
        for level in 1..=order + 1 {
            for i in 0..=order + 1 - level {
                dd[i] = (dd[i] - dd[i + 1]) / (pts[i].0 - pts[i + level].0);
            }
        }
        let (q0, q1) = (pts[0].1[k], pts[1].1[k]);
        let (i0, i1) = (pts[0].1[k + 1], pts[1].1[k + 1]);
        let volt_tol = opts.abstol + opts.reltol * i0.abs().max(i1.abs());
        let charge_tol = opts.reltol * q0.abs().max(q1.abs()).max(CHGTOL) / integ.h;
        let tol = volt_tol.max(charge_tol);
        let del = opts.trtol * tol / opts.abstol.max(factor * dd[0].abs());
        let step = if order == 2 { del.sqrt() } else { del };
        best = best.min(step);
    }
    best
}

/// Run a transient analysis.
pub fn transient(ckt: &mut Circuit, opts: &Options, req: &TranRequest) -> Result<Waveforms, AnalysisError> {
    ckt.finalize()?;
    let (tstep, tstop) = (req.tstep, req.tstop);
    if !(tstep > 0.0 && tstop > 0.0 && req.tstart >= 0.0 && req.tstart < tstop) {
        return Err(AnalysisError::Invalid(format!(
            ".tran needs 0 < tstep, 0 <= tstart < tstop (got {tstep}, {tstop}, {})",
            req.tstart
        )));
    }
    let tran = TranSpec { tstep, tstop };
    let n = ckt.unknowns() + 1;
    let mut states = States::new(ckt.n_states);
    let mut x = vec![0.0; n];
    if req.uic {
        for (node, v) in &req.ic {
            if let Some(var) = ckt.find_node(node) {
                x[var] = *v;
            }
        }
        for d in ckt.devices() {
            d.initial_condition(&mut x);
        }
    } else {
        solve_op(ckt, &mut x, &mut states, opts, Some(tran), false)?;
    }
    // Initial charges, then seed the history.
    let mut rhs = vec![0.0; n];
    let mut lp = load_params(Mode::TranInit, 0.0, tran);
    lp.gmin = opts.gmin;
    ckt.load(&x, &mut rhs, &lp, &Integrator::default(), &mut states);
    states.seed();

    let span = tstop - req.tstart;
    let max_step = if req.tmax > 0.0 {
        req.tmax
    } else {
        tstep.min(span / 50.0)
    };
    let h_min = 1e-11 * max_step;
    let mut h = (tstop / 100.0).min(tstep) / 10.0;
    let mut h_prev = h;
    let mut order: u8 = 1;
    let mut t = 0.0;
    let mut hist: VecDeque<Point> = VecDeque::from([Point {
        t: 0.0,
        x: x.clone(),
        q: states.now.clone(),
    }]);
    let mut out = Waveforms::new(ckt, "time".into());
    if req.tstart <= 0.0 {
        out.push(0.0, &x);
    }
    let next_bp = |ckt: &Circuit, t: f64| -> f64 {
        ckt.devices()
            .filter_map(|d| d.next_breakpoint(t, &tran))
            .filter(|&b| b > t + h_min)
            .fold(tstop, f64::min)
    };
    let mut bp = next_bp(ckt, t);
    let mut x_new = x.clone();

    while t < tstop - h_min {
        h = h.min(max_step);
        let mut hit = false;
        if t + h >= bp - h_min {
            h = bp - t;
            hit = true;
        } else if t + 2.0 * h > bp {
            h = (bp - t) / 2.0;
        }
        let integ = Integrator::new(opts.method, order, h, h_prev);
        let t_new = if hit { bp } else { t + h };
        let k_pred = (order as usize).min(hist.len() - 1);
        let x_pred = predict(&hist, k_pred, t_new);
        x_new.copy_from_slice(&x_pred);
        x_new[0] = 0.0;
        let p = NrParams {
            load: load_params(Mode::Tran, t_new, tran),
            max_iter: opts.itl4,
            damping: None,
        };
        match newton(ckt, &mut x_new, &mut states, &integ, opts, p) {
            NrOutcome::Converged(k) => out.stats.newton_iterations += k,
            NrOutcome::Singular(v) => return Err(singular_error(ckt, Some(v))),
            NrOutcome::NoConvergence => {
                out.stats.newton_iterations += opts.itl4;
                out.stats.rejected_newton += 1;
                h /= 8.0;
                order = 1;
                if h < h_min {
                    return Err(AnalysisError::TimestepTooSmall {
                        time: t,
                        reason: "Newton did not converge".into(),
                    });
                }
                continue;
            }
        }

        // Charge-based truncation error (SPICE CKTterr), skipped on the
        // first step after a breakpoint where there is no usable history.
        let mut h_lte = f64::INFINITY;
        if hist.len() > order as usize {
            h_lte = truncation_step(&integ, t_new, &states, &hist, opts);
            if h_lte < 0.9 * h {
                out.stats.rejected_lte += 1;
                h = h_lte.max(h / 8.0);
                order = 1.max(order.min(hist.len() as u8 - 1));
                if h < h_min {
                    return Err(AnalysisError::TimestepTooSmall {
                        time: t,
                        reason: "truncation error above tolerance".into(),
                    });
                }
                continue;
            }
        }

        // Accept.
        t = t_new;
        states.rotate();
        x.copy_from_slice(&x_new);
        out.stats.accepted += 1;
        if t >= req.tstart {
            out.push(t, &x);
        }
        h_prev = h;
        let mut h_next = (2.0 * h).min(h_lte);
        if hit {
            out.stats.breakpoints += 1;
            hist.clear();
            bp = next_bp(ckt, t);
            // Restart gently after a corner (as SPICE does).
            h_next = (0.1 * h.min(bp - t)).max(10.0 * h_min);
        }
        hist.push_back(Point {
            t,
            x: x.clone(),
            q: states.now.clone(),
        });
        while hist.len() > 4 {
            hist.pop_front();
        }
        // Second order needs three history points for its error estimate.
        order = if hist.len() >= 3 { 2 } else { 1 };
        h = h_next;
    }
    out.stats.factorisations = ckt.system.as_ref().map_or(0, |s| s.stats().factorisations);
    Ok(out)
}

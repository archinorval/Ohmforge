//! Junction diode: Shockley equation with emission coefficient, series
//! resistance, reverse breakdown, depletion and diffusion capacitance, and
//! SPICE junction-voltage limiting.

use std::collections::BTreeMap;

use super::limit::{depletion, junction, pnjlim, thermal_voltage, vcrit};
use super::param;
use crate::circuit::{Device, LoadCtx, Mode, Setup, Var};
use crate::matrix::Slot;

/// Diode model parameters (SPICE names).
#[derive(Debug, Clone, PartialEq)]
pub struct DiodeModel {
    /// Saturation current (A).
    pub is: f64,
    /// Emission coefficient.
    pub n: f64,
    /// Series resistance (Ω).
    pub rs: f64,
    /// Zero-bias junction capacitance (F).
    pub cjo: f64,
    /// Junction potential (V).
    pub vj: f64,
    /// Grading coefficient.
    pub m: f64,
    /// Forward-bias depletion capacitance coefficient.
    pub fc: f64,
    /// Transit time (s).
    pub tt: f64,
    /// Reverse breakdown voltage (V), if any.
    pub bv: Option<f64>,
    /// Current at breakdown voltage (A).
    pub ibv: f64,
}

impl Default for DiodeModel {
    fn default() -> Self {
        Self {
            is: 1e-14,
            n: 1.0,
            rs: 0.0,
            cjo: 0.0,
            vj: 1.0,
            m: 0.5,
            fc: 0.5,
            tt: 0.0,
            bv: None,
            ibv: 1e-3,
        }
    }
}

impl DiodeModel {
    /// Build from `.model` parameters; unknown names are returned.
    pub fn from_params(p: &BTreeMap<String, f64>) -> (Self, Vec<String>) {
        let d = Self::default();
        let mut unknown = Vec::new();
        for k in p.keys() {
            if !matches!(
                k.as_str(),
                "is" | "n"
                    | "rs"
                    | "cjo"
                    | "cj0"
                    | "cj"
                    | "vj"
                    | "pb"
                    | "m"
                    | "mj"
                    | "fc"
                    | "tt"
                    | "bv"
                    | "ibv"
                    | "level"
            ) {
                unknown.push(k.clone());
            }
        }
        let m = Self {
            is: param(p, &["is"], d.is),
            n: param(p, &["n"], d.n),
            rs: param(p, &["rs"], d.rs),
            cjo: param(p, &["cjo", "cj0", "cj"], d.cjo),
            vj: param(p, &["vj", "pb"], d.vj),
            m: param(p, &["m", "mj"], d.m),
            fc: param(p, &["fc"], d.fc),
            tt: param(p, &["tt"], d.tt),
            bv: p.get("bv").copied(),
            ibv: param(p, &["ibv"], d.ibv),
        };
        (m, unknown)
    }
}

/// A diode instance.
#[derive(Debug, Clone)]
pub struct Diode {
    name: String,
    a: Var,
    k: Var,
    ai: Var,
    model: DiodeModel,
    area: f64,
    off: bool,
    vte: f64,
    vcrit: f64,
    xbv: f64,
    vd_old: f64,
    s_rs: [Slot; 4],
    s_j: [Slot; 4],
    state: usize,
}

impl Diode {
    /// Diode from anode `a` to cathode `k`.
    pub fn new(name: &str, a: Var, k: Var, model: DiodeModel, area: f64, off: bool) -> Self {
        let vt = thermal_voltage();
        let vte = model.n * vt;
        let is = model.is * area;
        let xbv = model
            .bv
            .map_or(f64::INFINITY, |bv| breakdown_knee(bv, model.ibv * area, is, vt));
        Self {
            name: name.into(),
            a,
            k,
            ai: a,
            vcrit: vcrit(vte, is),
            vte,
            xbv,
            model,
            area,
            off,
            vd_old: 0.0,
            s_rs: [Slot::NONE; 4],
            s_j: [Slot::NONE; 4],
            state: 0,
        }
    }

    /// DC current and conductance at junction voltage `vd` (gmin included).
    pub fn dc(&self, vd: f64, gmin: f64) -> (f64, f64) {
        let is = self.model.is * self.area;
        let (i, g) = if vd < -self.xbv {
            let e = (-(self.xbv + vd) / self.vte).min(80.0).exp();
            (-is * e, is * e / self.vte)
        } else {
            junction(vd, is, self.vte)
        };
        (i + gmin * vd, g + gmin)
    }
}

/// SPICE3 breakdown-knee adjustment so that the current at `-bv` is `ibv`.
fn breakdown_knee(bv: f64, ibv: f64, is: f64, vt: f64) -> f64 {
    if ibv < is * bv / vt {
        return bv;
    }
    let cbv = ibv;
    let tol = 1e-3 * cbv;
    let mut xbv = bv - vt * (1.0 + cbv / is).ln();
    for _ in 0..25 {
        xbv = bv - vt * (cbv / is + 1.0 - xbv / vt).ln();
        let xcbv = is * ((bv - xbv) / vt).exp() - is + is * xbv / vt;
        if (xcbv - cbv).abs() <= tol {
            break;
        }
    }
    xbv
}

impl Device for Diode {
    fn name(&self) -> &str {
        &self.name
    }
    fn is_nonlinear(&self) -> bool {
        true
    }
    fn setup(&mut self, s: &mut Setup<'_>) {
        if self.model.rs > 0.0 {
            self.ai = s.internal_node(&self.name, "a");
            self.s_rs = s.slots2(self.a, self.ai);
        }
        self.s_j = s.slots2(self.ai, self.k);
        self.state = s.states(2);
    }
    fn load(&mut self, cx: &mut LoadCtx<'_>) {
        let mut vd = cx.v(self.ai) - cx.v(self.k);
        if cx.init_junctions {
            vd = if self.off { 0.0 } else { self.vcrit };
        } else if self.xbv.is_finite() && vd < (-self.xbv + 10.0 * self.vte).min(0.0) {
            // Limit in the breakdown region by mirroring about -BV (SPICE3).
            let (v, lim) = pnjlim(-(vd + self.xbv), -(self.vd_old + self.xbv), self.vte, self.vcrit);
            vd = -(v + self.xbv);
            cx.limited |= lim;
        } else {
            let (v, lim) = pnjlim(vd, self.vd_old, self.vte, self.vcrit);
            vd = v;
            cx.limited |= lim;
        }
        self.vd_old = vd;
        let (mut id, mut gd) = self.dc(vd, cx.gmin);
        if cx.mode != Mode::Dc {
            let m = &self.model;
            let (qj, cj) = depletion(vd, m.cjo * self.area, m.vj, m.m, m.fc);
            let (idd, gdd) = self.dc(vd, 0.0);
            cx.states.now[self.state] = m.tt * idd + qj;
            if cx.mode == Mode::Tran {
                let (geq, iq) = cx.integrate(self.state, m.tt * gdd + cj);
                id += iq;
                gd += geq;
            } else {
                cx.states.now[self.state + 1] = 0.0;
            }
        }
        if self.model.rs > 0.0 {
            cx.conductance(&self.s_rs, self.area / self.model.rs);
        }
        cx.conductance(&self.s_j, gd);
        cx.current(self.ai, self.k, id - gd * vd);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shockley_and_reverse_regions() {
        let d = Diode::new("d1", 1, 0, DiodeModel::default(), 1.0, false);
        let vt = thermal_voltage();
        let (i, g) = d.dc(0.6, 0.0);
        let exact = 1e-14 * ((0.6 / vt).exp() - 1.0);
        assert!((i - exact).abs() < 1e-12 * exact);
        assert!((g - 1e-14 * (0.6 / vt).exp() / vt).abs() < 1e-9 * g);
        let (ir, _) = d.dc(-5.0, 0.0);
        assert!((ir + 1e-14).abs() < 1e-16);
    }

    #[test]
    fn breakdown_matches_ibv() {
        let model = DiodeModel {
            bv: Some(5.1),
            ibv: 1e-3,
            ..DiodeModel::default()
        };
        let d = Diode::new("dz", 1, 0, model, 1.0, false);
        let (i, _) = d.dc(-5.1, 0.0);
        assert!((i + 1e-3).abs() < 2e-5, "{i}");
    }
}

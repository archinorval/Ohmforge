//! MOSFET: Shichman-Hodges (SPICE level 1) with body effect, channel-length
//! modulation, drain/source series resistance, bulk-junction diodes and
//! gate overlap capacitances, with SPICE FET voltage limiting.
//!
//! Not yet modelled: Meyer intrinsic gate capacitances (TOX-derived),
//! bulk depletion capacitances, levels 2/3 and BSIM.

use std::collections::BTreeMap;

use super::limit::{fetlim, limvds, pnjlim, thermal_voltage, vcrit};
use super::param;
use crate::circuit::{Device, LoadCtx, Mode, Setup, Var};
use crate::matrix::Slot;

/// Level-1 parameters (SPICE names).
#[derive(Debug, Clone, PartialEq)]
#[allow(missing_docs)]
pub struct MosModel {
    /// +1 NMOS, -1 PMOS.
    pub polarity: f64,
    pub vto: f64,
    pub kp: f64,
    pub gamma: f64,
    pub phi: f64,
    pub lambda: f64,
    pub rd: f64,
    pub rs: f64,
    pub is: f64,
    pub cgso: f64,
    pub cgdo: f64,
    pub cgbo: f64,
    pub ld: f64,
}

impl MosModel {
    /// SPICE level-1 defaults.
    pub fn new(polarity: f64) -> Self {
        Self {
            polarity,
            vto: 0.0,
            kp: 2e-5,
            gamma: 0.0,
            phi: 0.6,
            lambda: 0.0,
            rd: 0.0,
            rs: 0.0,
            is: 1e-14,
            cgso: 0.0,
            cgdo: 0.0,
            cgbo: 0.0,
            ld: 0.0,
        }
    }

    /// Build from `.model` parameters; returns names this model ignores.
    /// If `KP` is absent but `TOX` is given, `KP = UO * eps_ox / TOX`.
    pub fn from_params(polarity: f64, p: &BTreeMap<String, f64>) -> (Self, Vec<String>) {
        let d = Self::new(polarity);
        const KNOWN: &[&str] = &[
            "level", "vto", "vt0", "kp", "gamma", "phi", "lambda", "rd", "rs", "is", "cgso", "cgdo", "cgbo", "ld",
            "tox", "uo", "u0",
        ];
        let unknown = p.keys().filter(|k| !KNOWN.contains(&k.as_str())).cloned().collect();
        let kp = match (p.get("kp"), p.get("tox")) {
            (Some(&kp), _) => kp,
            (None, Some(&tox)) => param(p, &["uo", "u0"], 600.0) * 1e-4 * 3.9 * 8.854_214_871e-12 / tox,
            (None, None) => d.kp,
        };
        let m = Self {
            polarity,
            vto: param(p, &["vto", "vt0"], d.vto),
            kp,
            gamma: param(p, &["gamma"], d.gamma),
            phi: param(p, &["phi"], d.phi),
            lambda: param(p, &["lambda"], d.lambda),
            rd: param(p, &["rd"], d.rd),
            rs: param(p, &["rs"], d.rs),
            is: param(p, &["is"], d.is),
            cgso: param(p, &["cgso"], d.cgso),
            cgdo: param(p, &["cgdo"], d.cgdo),
            cgbo: param(p, &["cgbo"], d.cgbo),
            ld: param(p, &["ld"], d.ld),
        };
        (m, unknown)
    }
}

/// Drain current and derivatives in normalised (NMOS, forward) terms.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct MosOp {
    /// Drain current (positive into the drain for NMOS forward mode).
    pub id: f64,
    /// dId/dVgs.
    pub gm: f64,
    /// dId/dVds.
    pub gds: f64,
    /// dId/dVbs.
    pub gmbs: f64,
    /// Threshold voltage including body effect.
    pub von: f64,
}

/// A MOSFET instance (drain, gate, source, bulk).
#[derive(Debug, Clone)]
pub struct Mosfet {
    name: String,
    d: Var,
    g: Var,
    s: Var,
    b: Var,
    di: Var,
    si: Var,
    m: MosModel,
    beta: f64,
    is: f64,
    cgs: f64,
    cgd: f64,
    cgb: f64,
    gdr: f64,
    gsr: f64,
    off: bool,
    vt: f64,
    vcrit: f64,
    old: (f64, f64, f64),
    von: f64,
    slots: [Slot; 22],
    state: usize,
}

impl Mosfet {
    /// MOSFET with width `w`, length `l` (metres) and multiplier `mult`.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        name: &str,
        d: Var,
        g: Var,
        s: Var,
        b: Var,
        model: MosModel,
        w: f64,
        l: f64,
        mult: f64,
        off: bool,
    ) -> Self {
        let leff = l - 2.0 * model.ld;
        let vt = thermal_voltage();
        let is = model.is * mult;
        Self {
            name: name.into(),
            d,
            g,
            s,
            b,
            di: d,
            si: s,
            beta: model.kp * w / leff * mult,
            is,
            cgs: model.cgso * w * mult,
            cgd: model.cgdo * w * mult,
            cgb: model.cgbo * leff * mult,
            gdr: if model.rd > 0.0 { mult / model.rd } else { 0.0 },
            gsr: if model.rs > 0.0 { mult / model.rs } else { 0.0 },
            off,
            vt,
            vcrit: vcrit(vt, is),
            old: (0.0, 0.0, 0.0),
            von: model.polarity * model.vto,
            m: model,
            slots: [Slot::NONE; 22],
            state: 0,
        }
    }

    /// Channel current in normalised terms for `vgs`, `vds` (>= 0), `vbs`.
    pub fn channel(&self, vgs: f64, vds: f64, vbs: f64) -> MosOp {
        let m = &self.m;
        let sarg = if vbs <= 0.0 {
            (m.phi - vbs).sqrt()
        } else {
            let s = m.phi.sqrt();
            (s - vbs / (s + s)).max(0.0)
        };
        let von = m.polarity * m.vto - m.gamma * m.phi.sqrt() + m.gamma * sarg;
        let vgst = vgs - von;
        let arg = if sarg <= 0.0 { 0.0 } else { m.gamma / (sarg + sarg) };
        if vgst <= 0.0 {
            return MosOp {
                von,
                ..MosOp::default()
            };
        }
        let betap = self.beta * (1.0 + m.lambda * vds);
        if vgst <= vds {
            let id = betap * vgst * vgst * 0.5;
            let gm = betap * vgst;
            MosOp {
                id,
                gm,
                gds: m.lambda * self.beta * vgst * vgst * 0.5,
                gmbs: gm * arg,
                von,
            }
        } else {
            let id = betap * vds * (vgst - 0.5 * vds);
            let gm = betap * vds;
            let gds = betap * (vgst - vds) + m.lambda * self.beta * vds * (vgst - 0.5 * vds);
            MosOp {
                id,
                gm,
                gds,
                gmbs: gm * arg,
                von,
            }
        }
    }

    fn bulk_diode(&self, v: f64, gmin: f64) -> (f64, f64) {
        if v <= -3.0 * self.vt {
            (gmin * v - self.is, gmin)
        } else {
            let e = (v / self.vt).min(80.0).exp();
            (self.is * (e - 1.0) + gmin * v, self.is * e / self.vt + gmin)
        }
    }
}

const DD: usize = 0;
const GG: usize = 1;
const SS: usize = 2;
const BB: usize = 3;
const DPDP: usize = 4;
const SPSP: usize = 5;
const DDP: usize = 6;
const GB: usize = 7;
const GDP: usize = 8;
const GSP: usize = 9;
const SSP: usize = 10;
const BDP: usize = 11;
const BSP: usize = 12;
const DPSP: usize = 13;
const DPD: usize = 14;
const BG: usize = 15;
const DPG: usize = 16;
const SPG: usize = 17;
const SPS: usize = 18;
const DPB: usize = 19;
const SPB: usize = 20;
const SPDP: usize = 21;

impl Device for Mosfet {
    fn name(&self) -> &str {
        &self.name
    }
    fn is_nonlinear(&self) -> bool {
        true
    }
    fn setup(&mut self, s: &mut Setup<'_>) {
        if self.gdr > 0.0 {
            self.di = s.internal_node(&self.name, "d");
        }
        if self.gsr > 0.0 {
            self.si = s.internal_node(&self.name, "s");
        }
        let (d, g, src, b, dp, sp) = (self.d, self.g, self.s, self.b, self.di, self.si);
        let pairs = [
            (d, d),
            (g, g),
            (src, src),
            (b, b),
            (dp, dp),
            (sp, sp),
            (d, dp),
            (g, b),
            (g, dp),
            (g, sp),
            (src, sp),
            (b, dp),
            (b, sp),
            (dp, sp),
            (dp, d),
            (b, g),
            (dp, g),
            (sp, g),
            (sp, src),
            (dp, b),
            (sp, b),
            (sp, dp),
        ];
        for (i, (r, c)) in pairs.into_iter().enumerate() {
            self.slots[i] = s.slot(r, c);
        }
        self.state = s.states(6);
    }
    fn load(&mut self, cx: &mut LoadCtx<'_>) {
        let t = self.m.polarity;
        let (mut vgs, mut vds, mut vbs);
        if cx.init_junctions {
            if self.off {
                (vgs, vds, vbs) = (0.0, 0.0, 0.0);
            } else {
                (vgs, vds, vbs) = (t * self.m.vto, 0.0, -1.0);
            }
        } else {
            vbs = t * (cx.v(self.b) - cx.v(self.si));
            vgs = t * (cx.v(self.g) - cx.v(self.si));
            vds = t * (cx.v(self.di) - cx.v(self.si));
            let (vgs_o, vds_o, vbs_o) = self.old;
            let vgd = vgs - vds;
            let vgdo = vgs_o - vds_o;
            if vds_o >= 0.0 {
                vgs = fetlim(vgs, vgs_o, self.von);
                vds = vgs - vgd;
                vds = limvds(vds, vds_o);
            } else {
                let vgd = fetlim(vgd, vgdo, self.von);
                vds = vgs - vgd;
                vds = -limvds(-vds, -vds_o);
                vgs = vgd + vds;
            }
            let lim;
            if vds >= 0.0 {
                (vbs, lim) = pnjlim(vbs, vbs_o, self.vt, self.vcrit);
            } else {
                let (vbd, l) = pnjlim(vbs - vds, vbs_o - vds_o, self.vt, self.vcrit);
                vbs = vbd + vds;
                lim = l;
            }
            cx.limited |= lim;
        }
        self.old = (vgs, vds, vbs);
        let vbd = vbs - vds;
        let vgd = vgs - vds;
        let (cbs, gbs) = self.bulk_diode(vbs, cx.gmin);
        let (cbd, gbd) = self.bulk_diode(vbd, cx.gmin);
        let forward = vds >= 0.0;
        let op = if forward {
            self.channel(vgs, vds, vbs)
        } else {
            self.channel(vgd, -vds, vbd)
        };
        self.von = op.von;
        let (gm, gds, gmbs) = (op.gm, op.gds, op.gmbs);
        let (xnrm, xrev) = if forward { (1.0, 0.0) } else { (0.0, 1.0) };
        let cdreq = if forward {
            t * (op.id - gds * vds - gm * vgs - gmbs * vbs)
        } else {
            -t * (op.id - gds * (-vds) - gm * vgd - gmbs * vbd)
        };
        let ceqbs = t * (cbs - gbs * vbs);
        let ceqbd = t * (cbd - gbd * vbd);

        // Linear overlap capacitances (real voltages, polarity-independent).
        let (mut gcgs, mut gcgd, mut gcgb) = (0.0, 0.0, 0.0);
        let (mut ceqgs, mut ceqgd, mut ceqgb) = (0.0, 0.0, 0.0);
        if cx.mode != Mode::Dc {
            let st = self.state;
            let vg = cx.v(self.g);
            let caps = [
                (self.cgs, vg - cx.v(self.si)),
                (self.cgd, vg - cx.v(self.di)),
                (self.cgb, vg - cx.v(self.b)),
            ];
            let mut res = [(0.0, 0.0); 3];
            for (k, (c, v)) in caps.into_iter().enumerate() {
                cx.states.now[st + 2 * k] = c * v;
                if cx.mode == Mode::Tran {
                    let (geq, i) = cx.integrate(st + 2 * k, c);
                    res[k] = (geq, i - geq * v);
                } else {
                    cx.states.now[st + 2 * k + 1] = 0.0;
                }
            }
            (gcgs, ceqgs) = res[0];
            (gcgd, ceqgd) = res[1];
            (gcgb, ceqgb) = res[2];
        }

        cx.rhs(self.g, -(ceqgs + ceqgb + ceqgd));
        cx.rhs(self.b, -(ceqbs + ceqbd) + ceqgb);
        cx.rhs(self.di, ceqbd - cdreq + ceqgd);
        cx.rhs(self.si, cdreq + ceqbs + ceqgs);
        let (gdr, gsr) = (self.gdr, self.gsr);
        let stamps = [
            (DD, gdr),
            (GG, gcgd + gcgs + gcgb),
            (SS, gsr),
            (BB, gbd + gbs + gcgb),
            (DPDP, gdr + gds + gbd + xrev * (gm + gmbs) + gcgd),
            (SPSP, gsr + gds + gbs + xnrm * (gm + gmbs) + gcgs),
            (DDP, -gdr),
            (GB, -gcgb),
            (GDP, -gcgd),
            (GSP, -gcgs),
            (SSP, -gsr),
            (BDP, -gbd),
            (BSP, -gbs),
            (DPSP, -gds - xnrm * (gm + gmbs)),
            (DPD, -gdr),
            (BG, -gcgb),
            (DPG, (xnrm - xrev) * gm - gcgd),
            (SPG, -(xnrm - xrev) * gm - gcgs),
            (SPS, -gsr),
            (DPB, -gbd + (xnrm - xrev) * gmbs),
            (SPB, -gbs - (xnrm - xrev) * gmbs),
            (SPDP, -gds - xrev * (gm + gmbs)),
        ];
        for (k, v) in stamps {
            cx.add(self.slots[k], v);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nmos() -> Mosfet {
        let mut m = MosModel::new(1.0);
        m.vto = 0.7;
        m.kp = 100e-6;
        m.lambda = 0.02;
        m.gamma = 0.4;
        Mosfet::new("m1", 1, 2, 0, 0, m, 10e-6, 1e-6, 1.0, false)
    }

    #[test]
    fn square_law_regions() {
        let q = nmos();
        let beta = 100e-6 * 10.0;
        let sat = q.channel(1.7, 3.0, 0.0);
        assert!((sat.id - 0.5 * beta * 1.0 * (1.0 + 0.02 * 3.0)).abs() < 1e-12);
        let lin = q.channel(1.7, 0.2, 0.0);
        assert!((lin.id - beta * (1.0 + 0.02 * 0.2) * 0.2 * (1.0 - 0.1)).abs() < 1e-12);
        assert_eq!(q.channel(0.5, 1.0, 0.0).id, 0.0);
        // Body effect raises the threshold.
        assert!(q.channel(1.7, 3.0, -2.0).von > 0.7 + 0.3);
    }

    #[test]
    fn derivatives_match_finite_differences() {
        let q = nmos();
        let dv = 1e-7;
        for (vgs, vds, vbs) in [(1.5, 3.0, -0.5), (2.0, 0.3, -1.0)] {
            let c = q.channel(vgs, vds, vbs);
            let gm = (q.channel(vgs + dv, vds, vbs).id - q.channel(vgs - dv, vds, vbs).id) / (2.0 * dv);
            let gds = (q.channel(vgs, vds + dv, vbs).id - q.channel(vgs, vds - dv, vbs).id) / (2.0 * dv);
            let gmbs = (q.channel(vgs, vds, vbs + dv).id - q.channel(vgs, vds, vbs - dv).id) / (2.0 * dv);
            assert!((gm - c.gm).abs() < 1e-6 * c.gm.abs().max(1e-6));
            assert!((gds - c.gds).abs() < 1e-6 * c.gds.abs().max(1e-6));
            assert!((gmbs - c.gmbs).abs() < 1e-5 * c.gmbs.abs().max(1e-6));
        }
    }
}

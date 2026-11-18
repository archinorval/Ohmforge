//! Bipolar transistor: Gummel-Poon subset of the SPICE level-1 BJT
//! (forward/reverse transport with Early effect and high injection,
//! base-emitter and base-collector leakage, terminal resistances, depletion
//! and diffusion charges).

use std::collections::BTreeMap;

use super::limit::{depletion, junction, pnjlim, thermal_voltage, vcrit};
use super::param;
use crate::circuit::{Device, LoadCtx, Mode, Setup, Var};
use crate::matrix::Slot;

/// Gummel-Poon parameters (SPICE names; zero means "infinite" for VAF,
/// VAR, IKF, IKR as in SPICE).
#[derive(Debug, Clone, PartialEq)]
#[allow(missing_docs)]
pub struct BjtModel {
    /// +1 for NPN, -1 for PNP.
    pub polarity: f64,
    pub is: f64,
    pub bf: f64,
    pub nf: f64,
    pub vaf: f64,
    pub ikf: f64,
    pub ise: f64,
    pub ne: f64,
    pub br: f64,
    pub nr: f64,
    pub var: f64,
    pub ikr: f64,
    pub isc: f64,
    pub nc: f64,
    pub rb: f64,
    pub re: f64,
    pub rc: f64,
    pub cje: f64,
    pub vje: f64,
    pub mje: f64,
    pub tf: f64,
    pub cjc: f64,
    pub vjc: f64,
    pub mjc: f64,
    pub tr: f64,
    pub fc: f64,
}

impl BjtModel {
    /// SPICE defaults for an NPN (`polarity` = 1) or PNP (-1).
    pub fn new(polarity: f64) -> Self {
        Self {
            polarity,
            is: 1e-16,
            bf: 100.0,
            nf: 1.0,
            vaf: 0.0,
            ikf: 0.0,
            ise: 0.0,
            ne: 1.5,
            br: 1.0,
            nr: 1.0,
            var: 0.0,
            ikr: 0.0,
            isc: 0.0,
            nc: 2.0,
            rb: 0.0,
            re: 0.0,
            rc: 0.0,
            cje: 0.0,
            vje: 0.75,
            mje: 0.33,
            tf: 0.0,
            cjc: 0.0,
            vjc: 0.75,
            mjc: 0.33,
            tr: 0.0,
            fc: 0.5,
        }
    }

    /// Build from `.model` parameters; returns names this subset ignores.
    pub fn from_params(polarity: f64, p: &BTreeMap<String, f64>) -> (Self, Vec<String>) {
        let d = Self::new(polarity);
        const KNOWN: &[&str] = &[
            "is", "bf", "nf", "vaf", "va", "ikf", "ik", "ise", "c2", "ne", "br", "nr", "var", "vb", "ikr", "isc", "c4",
            "nc", "rb", "re", "rc", "cje", "vje", "pe", "mje", "me", "tf", "cjc", "vjc", "pc", "mjc", "mc", "tr", "fc",
            "level",
        ];
        let unknown = p.keys().filter(|k| !KNOWN.contains(&k.as_str())).cloned().collect();
        let m = Self {
            polarity,
            is: param(p, &["is"], d.is),
            bf: param(p, &["bf"], d.bf),
            nf: param(p, &["nf"], d.nf),
            vaf: param(p, &["vaf", "va"], d.vaf),
            ikf: param(p, &["ikf", "ik"], d.ikf),
            ise: param(p, &["ise", "c2"], d.ise),
            ne: param(p, &["ne"], d.ne),
            br: param(p, &["br"], d.br),
            nr: param(p, &["nr"], d.nr),
            var: param(p, &["var", "vb"], d.var),
            ikr: param(p, &["ikr"], d.ikr),
            isc: param(p, &["isc", "c4"], d.isc),
            nc: param(p, &["nc"], d.nc),
            rb: param(p, &["rb"], d.rb),
            re: param(p, &["re"], d.re),
            rc: param(p, &["rc"], d.rc),
            cje: param(p, &["cje"], d.cje),
            vje: param(p, &["vje", "pe"], d.vje),
            mje: param(p, &["mje", "me"], d.mje),
            tf: param(p, &["tf"], d.tf),
            cjc: param(p, &["cjc"], d.cjc),
            vjc: param(p, &["vjc", "pc"], d.vjc),
            mjc: param(p, &["mjc", "mc"], d.mjc),
            tr: param(p, &["tr"], d.tr),
            fc: param(p, &["fc"], d.fc),
        };
        (m, unknown)
    }
}

/// Small-signal quantities at one bias point (normalised to NPN polarity).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct BjtOp {
    /// Collector current.
    pub ic: f64,
    /// Base current.
    pub ib: f64,
    /// Transconductance dIc/dVbe.
    pub gm: f64,
    /// Input conductance dIb/dVbe.
    pub gpi: f64,
    /// Feedback conductance dIb/dVbc.
    pub gmu: f64,
    /// Output conductance.
    pub go: f64,
    /// Base charge factor qb.
    pub qb: f64,
    dqbdve: f64,
    dqbdvc: f64,
    cbe: f64,
    gbe: f64,
    cbc: f64,
    gbc: f64,
}

/// A BJT instance (collector, base, emitter).
#[derive(Debug, Clone)]
pub struct Bjt {
    name: String,
    c: Var,
    b: Var,
    e: Var,
    ci: Var,
    bi: Var,
    ei: Var,
    m: BjtModel,
    area: f64,
    off: bool,
    vt: f64,
    vcrit: f64,
    vbe_old: f64,
    vbc_old: f64,
    slots: [Slot; 21],
    state: usize,
}

const CC: usize = 0;
const BB: usize = 1;
const EE: usize = 2;
const CPCP: usize = 3;
const BPBP: usize = 4;
const EPEP: usize = 5;
const CCP: usize = 6;
const BBP: usize = 7;
const EEP: usize = 8;
const CPC: usize = 9;
const CPBP: usize = 10;
const CPEP: usize = 11;
const BPB: usize = 12;
const BPCP: usize = 13;
const BPEP: usize = 14;
const EPE: usize = 15;
const EPCP: usize = 16;
const EPBP: usize = 17;

impl Bjt {
    /// BJT with collector `c`, base `b`, emitter `e`.
    pub fn new(name: &str, c: Var, b: Var, e: Var, model: BjtModel, area: f64, off: bool) -> Self {
        let vt = thermal_voltage();
        Self {
            name: name.into(),
            c,
            b,
            e,
            ci: c,
            bi: b,
            ei: e,
            vcrit: vcrit(vt, model.is * area),
            vt,
            m: model,
            area,
            off,
            vbe_old: 0.0,
            vbc_old: 0.0,
            slots: [Slot::NONE; 21],
            state: 0,
        }
    }

    /// DC Gummel-Poon evaluation at internal junction voltages (NPN sense).
    pub fn dc(&self, vbe: f64, vbc: f64, gmin: f64) -> BjtOp {
        let m = &self.m;
        let a = self.area;
        let vt = self.vt;
        let (cbe0, gbe0) = junction(vbe, m.is * a, m.nf * vt);
        let (cbe, gbe) = (cbe0 + gmin * vbe, gbe0 + gmin);
        let (cben, gben) = if m.ise > 0.0 {
            let (i, g) = junction(vbe, m.ise * a, m.ne * vt);
            (i + gmin * vbe, g + gmin)
        } else {
            (0.0, 0.0)
        };
        let (cbc0, gbc0) = junction(vbc, m.is * a, m.nr * vt);
        let (cbc, gbc) = (cbc0 + gmin * vbc, gbc0 + gmin);
        let (cbcn, gbcn) = if m.isc > 0.0 {
            let (i, g) = junction(vbc, m.isc * a, m.nc * vt);
            (i + gmin * vbc, g + gmin)
        } else {
            (0.0, 0.0)
        };
        let inv = |v: f64| if v > 0.0 { 1.0 / v } else { 0.0 };
        let (vafi, vari) = (inv(m.vaf), inv(m.var));
        let (oik, oikr) = (inv(m.ikf * a), inv(m.ikr * a));
        let q1 = 1.0 / (1.0 - vafi * vbc - vari * vbe);
        let (qb, dqbdve, dqbdvc) = if oik == 0.0 && oikr == 0.0 {
            (q1, q1 * q1 * vari, q1 * q1 * vafi)
        } else {
            let q2 = oik * cbe + oikr * cbc;
            let arg = (1.0 + 4.0 * q2).max(0.0);
            let sqarg = if arg != 0.0 { arg.sqrt() } else { 1.0 };
            let qb = q1 * (1.0 + sqarg) / 2.0;
            (
                qb,
                q1 * (qb * vari + oik * gbe / sqarg),
                q1 * (qb * vafi + oikr * gbc / sqarg),
            )
        };
        let ic = (cbe - cbc) / qb - cbc / m.br - cbcn;
        let ib = cbe / m.bf + cben + cbc / m.br + cbcn;
        let go = (gbc + (cbe - cbc) * dqbdvc / qb) / qb;
        let gm = (gbe - (cbe - cbc) * dqbdve / qb) / qb - go;
        let gpi = gbe / m.bf + gben;
        let gmu = gbc / m.br + gbcn;
        BjtOp {
            ic,
            ib,
            gm,
            gpi,
            gmu,
            go,
            qb,
            dqbdve,
            dqbdvc,
            cbe,
            gbe,
            cbc,
            gbc,
        }
    }
}

impl Device for Bjt {
    fn name(&self) -> &str {
        &self.name
    }
    fn is_nonlinear(&self) -> bool {
        true
    }
    fn setup(&mut self, s: &mut Setup<'_>) {
        if self.m.rc > 0.0 {
            self.ci = s.internal_node(&self.name, "c");
        }
        if self.m.rb > 0.0 {
            self.bi = s.internal_node(&self.name, "b");
        }
        if self.m.re > 0.0 {
            self.ei = s.internal_node(&self.name, "e");
        }
        let (c, b, e, cp, bp, ep) = (self.c, self.b, self.e, self.ci, self.bi, self.ei);
        let pairs = [
            (c, c),
            (b, b),
            (e, e),
            (cp, cp),
            (bp, bp),
            (ep, ep),
            (c, cp),
            (b, bp),
            (e, ep),
            (cp, c),
            (cp, bp),
            (cp, ep),
            (bp, b),
            (bp, cp),
            (bp, ep),
            (ep, e),
            (ep, cp),
            (ep, bp),
        ];
        for (i, (r, col)) in pairs.into_iter().enumerate() {
            self.slots[i] = s.slot(r, col);
        }
        self.state = s.states(4);
    }
    fn load(&mut self, cx: &mut LoadCtx<'_>) {
        let t = self.m.polarity;
        let (mut vbe, mut vbc);
        if cx.init_junctions {
            vbe = if self.off { 0.0 } else { self.vcrit };
            vbc = 0.0;
        } else {
            vbe = t * (cx.v(self.bi) - cx.v(self.ei));
            vbc = t * (cx.v(self.bi) - cx.v(self.ci));
            let (v1, l1) = pnjlim(vbe, self.vbe_old, self.vt, self.vcrit);
            let (v2, l2) = pnjlim(vbc, self.vbc_old, self.vt, self.vcrit);
            vbe = v1;
            vbc = v2;
            cx.limited |= l1 || l2;
        }
        self.vbe_old = vbe;
        self.vbc_old = vbc;
        let op = self.dc(vbe, vbc, cx.gmin);
        let (mut cc, mut cb) = (op.ic, op.ib);
        let (mut gpi, mut gmu, gm, go) = (op.gpi, op.gmu, op.gm, op.go);
        let mut geqcb = 0.0;

        if cx.mode != Mode::Dc {
            let m = &self.m;
            let a = self.area;
            // Diffusion charge (XTF = 0 subset) plus depletion charges.
            let (mut qbe, mut capbe) = depletion(vbe, m.cje * a, m.vje, m.mje, m.fc);
            if m.tf > 0.0 && vbe > 0.0 {
                let cbe_q = op.cbe / op.qb;
                let gbe_q = (op.gbe - cbe_q * op.dqbdve) / op.qb;
                geqcb = m.tf * (-cbe_q * op.dqbdvc) / op.qb;
                qbe += m.tf * cbe_q;
                capbe += m.tf * gbe_q;
            }
            let (qj, cj) = depletion(vbc, m.cjc * a, m.vjc, m.mjc, m.fc);
            let qbc = m.tr * op.cbc + qj;
            let capbc = m.tr * op.gbc + cj;
            let s = self.state;
            cx.states.now[s] = qbe;
            cx.states.now[s + 2] = qbc;
            if cx.mode == Mode::Tran {
                let (gbe_eq, cqbe) = cx.integrate(s, capbe);
                let (gbc_eq, cqbc) = cx.integrate(s + 2, capbc);
                geqcb *= cx.integ.ag0;
                gpi += gbe_eq;
                cb += cqbe;
                gmu += gbc_eq;
                cb += cqbc;
                cc -= cqbc;
            } else {
                cx.states.now[s + 1] = 0.0;
                cx.states.now[s + 3] = 0.0;
            }
        }

        let gx = if self.m.rb > 0.0 { self.area / self.m.rb } else { 0.0 };
        let gcpr = if self.m.rc > 0.0 { self.area / self.m.rc } else { 0.0 };
        let gepr = if self.m.re > 0.0 { self.area / self.m.re } else { 0.0 };
        let ceqbe = t * (cc + cb - vbe * (gm + go + gpi) + vbc * (go - geqcb));
        let ceqbc = t * (-cc + vbe * (gm + go) - vbc * (gmu + go));
        cx.rhs(self.ci, ceqbc);
        cx.rhs(self.bi, -ceqbe - ceqbc);
        cx.rhs(self.ei, ceqbe);
        let s = &self.slots;
        let stamps = [
            (CC, gcpr),
            (BB, gx),
            (EE, gepr),
            (CPCP, gmu + go + gcpr),
            (BPBP, gx + gpi + gmu + geqcb),
            (EPEP, gpi + gepr + gm + go),
            (CCP, -gcpr),
            (BBP, -gx),
            (EEP, -gepr),
            (CPC, -gcpr),
            (CPBP, -gmu + gm),
            (CPEP, -gm - go),
            (BPB, -gx),
            (BPCP, -gmu - geqcb),
            (BPEP, -gpi),
            (EPE, -gepr),
            (EPCP, -go + geqcb),
            (EPBP, -gpi - gm - geqcb),
        ];
        for (k, v) in stamps {
            cx.add(s[k], v);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forward_active_beta_and_early_effect() {
        let mut m = BjtModel::new(1.0);
        m.bf = 150.0;
        let q = Bjt::new("q1", 1, 2, 0, m.clone(), 1.0, false);
        let op = q.dc(0.65, -5.0, 0.0);
        assert!((op.ic / op.ib - 150.0).abs() < 1e-6, "{}", op.ic / op.ib);
        m.vaf = 50.0;
        let q = Bjt::new("q1", 1, 2, 0, m, 1.0, false);
        let a = q.dc(0.65, -1.0, 0.0);
        let b = q.dc(0.65, -6.0, 0.0);
        // Ic grows with Vce through 1 / (1 - vbc/VAF).
        // Ic is proportional to 1/qb = 1 - vbc/VAF when VAR, IKF are absent.
        assert!((b.ic / a.ic - (1.0 + 6.0 / 50.0) / (1.0 + 1.0 / 50.0)).abs() < 1e-9);
        // Small-signal check: dIc/dVbe = gm + go (SPICE convention).
        let dv = 1e-7;
        let up = q.dc(0.65 + dv, -3.0, 0.0);
        let dn = q.dc(0.65 - dv, -3.0, 0.0);
        let c = q.dc(0.65, -3.0, 0.0);
        assert!(((up.ic - dn.ic) / (2.0 * dv) - (c.gm + c.go)).abs() < 1e-5 * c.gm);
    }

    #[test]
    fn high_injection_derivatives_are_consistent() {
        let mut m = BjtModel::new(1.0);
        m.ikf = 10e-3;
        m.vaf = 80.0;
        m.ise = 1e-14;
        let q = Bjt::new("q1", 1, 2, 0, m, 1.0, false);
        let (vbe, vbc) = (0.78, -2.0);
        let c = q.dc(vbe, vbc, 0.0);
        let dv = 1e-7;
        let dic_dvbe = (q.dc(vbe + dv, vbc, 0.0).ic - q.dc(vbe - dv, vbc, 0.0).ic) / (2.0 * dv);
        let dic_dvbc = (q.dc(vbe, vbc + dv, 0.0).ic - q.dc(vbe, vbc - dv, 0.0).ic) / (2.0 * dv);
        let dib_dvbe = (q.dc(vbe + dv, vbc, 0.0).ib - q.dc(vbe - dv, vbc, 0.0).ib) / (2.0 * dv);
        // In (vbe, vbc) coordinates: dIc/dVbe = gm + go, dIc/dVbc = -(go + gmu), dIb/dVbe = gpi.
        assert!((dic_dvbe - (c.gm + c.go)).abs() < 1e-5 * dic_dvbe.abs());
        assert!((dic_dvbc + c.go + c.gmu).abs() < 1e-5 * dic_dvbe.abs());
        assert!((dib_dvbe - c.gpi).abs() < 1e-5 * c.gpi);
    }
}

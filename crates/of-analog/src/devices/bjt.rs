//! Bipolar transistor: Gummel-Poon subset of the SPICE level-1 BJT
//! (forward/reverse transport with Early effect and high injection,
//! base-emitter and base-collector leakage, current-dependent base
//! resistance, terminal resistances, depletion and bias-dependent diffusion
//! charges, split base-collector capacitance, substrate capacitance, excess
//! phase and SPICE3 temperature scaling).

use std::collections::BTreeMap;

use super::limit::{REFTEMP, depletion, junction, junction_temp, pnjlim, vcrit, vt_at};
use super::param;
use crate::circuit::{Device, LoadCtx, Mode, Setup, Var};
use crate::matrix::Slot;

/// Gummel-Poon parameters (SPICE names; zero means "infinite" for VAF,
/// VAR, IKF, IKR, ITF and VTF as in SPICE).
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
    /// Current where the base resistance falls halfway to RBM (0 = off).
    pub irb: f64,
    /// Minimum base resistance at high current (defaults to RB).
    pub rbm: Option<f64>,
    pub re: f64,
    pub rc: f64,
    pub cje: f64,
    pub vje: f64,
    pub mje: f64,
    pub tf: f64,
    /// Transit-time bias coefficient.
    pub xtf: f64,
    /// Transit-time dependence on Vbc (V).
    pub vtf: f64,
    /// Transit-time high-current parameter (A).
    pub itf: f64,
    /// Excess phase at 1/(2 pi TF) (degrees).
    pub ptf: f64,
    pub cjc: f64,
    pub vjc: f64,
    pub mjc: f64,
    /// Fraction of the B-C capacitance connected to the internal base.
    pub xcjc: f64,
    pub tr: f64,
    /// Collector-substrate zero-bias capacitance (F).
    pub cjs: f64,
    pub vjs: f64,
    pub mjs: f64,
    pub fc: f64,
    /// Band gap for IS temperature scaling (eV).
    pub eg: f64,
    /// IS temperature exponent.
    pub xti: f64,
    /// Beta temperature exponent.
    pub xtb: f64,
    /// Nominal (measurement) temperature (K).
    pub tnom: f64,
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
            irb: 0.0,
            rbm: None,
            re: 0.0,
            rc: 0.0,
            cje: 0.0,
            vje: 0.75,
            mje: 0.33,
            tf: 0.0,
            xtf: 0.0,
            vtf: 0.0,
            itf: 0.0,
            ptf: 0.0,
            cjc: 0.0,
            vjc: 0.75,
            mjc: 0.33,
            xcjc: 1.0,
            tr: 0.0,
            cjs: 0.0,
            vjs: 0.75,
            mjs: 0.0,
            fc: 0.5,
            eg: 1.11,
            xti: 3.0,
            xtb: 0.0,
            tnom: REFTEMP,
        }
    }

    /// Build from `.model` parameters; returns names this subset ignores.
    /// `tnom` is given in °C, as in SPICE.
    pub fn from_params(polarity: f64, p: &BTreeMap<String, f64>) -> (Self, Vec<String>) {
        let d = Self::new(polarity);
        const KNOWN: &[&str] = &[
            "is", "bf", "nf", "vaf", "va", "ikf", "ik", "ise", "c2", "ne", "br", "nr", "var", "vb", "ikr", "isc", "c4",
            "nc", "rb", "irb", "rbm", "re", "rc", "cje", "vje", "pe", "mje", "me", "tf", "xtf", "vtf", "itf", "ptf",
            "cjc", "vjc", "pc", "mjc", "mc", "xcjc", "tr", "cjs", "ccs", "vjs", "ps", "mjs", "ms", "fc", "eg", "xti",
            "xtb", "tnom", "level", "subs",
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
            irb: param(p, &["irb"], d.irb),
            rbm: p.get("rbm").copied(),
            re: param(p, &["re"], d.re),
            rc: param(p, &["rc"], d.rc),
            cje: param(p, &["cje"], d.cje),
            vje: param(p, &["vje", "pe"], d.vje),
            mje: param(p, &["mje", "me"], d.mje),
            tf: param(p, &["tf"], d.tf),
            xtf: param(p, &["xtf"], d.xtf),
            vtf: param(p, &["vtf"], d.vtf),
            itf: param(p, &["itf"], d.itf),
            ptf: param(p, &["ptf"], d.ptf),
            cjc: param(p, &["cjc"], d.cjc),
            vjc: param(p, &["vjc", "pc"], d.vjc),
            mjc: param(p, &["mjc", "mc"], d.mjc),
            xcjc: param(p, &["xcjc"], d.xcjc),
            tr: param(p, &["tr"], d.tr),
            cjs: param(p, &["cjs", "ccs"], d.cjs),
            vjs: param(p, &["vjs", "ps"], d.vjs),
            mjs: param(p, &["mjs", "ms"], d.mjs),
            fc: param(p, &["fc"], d.fc),
            eg: param(p, &["eg"], d.eg),
            xti: param(p, &["xti"], d.xti),
            xtb: param(p, &["xtb"], d.xtb),
            tnom: p.get("tnom").map_or(d.tnom, |c| c + 273.15),
        };
        (m, unknown)
    }
}

/// Temperature-adjusted parameters.
#[derive(Debug, Clone, Copy, PartialEq)]
struct BjtTemp {
    vt: f64,
    is: f64,
    bf: f64,
    br: f64,
    ise: f64,
    isc: f64,
    vje: f64,
    cje: f64,
    vjc: f64,
    cjc: f64,
    vjs: f64,
    cjs: f64,
}

impl BjtTemp {
    fn new(m: &BjtModel, temp: f64) -> Self {
        let tnom = m.tnom;
        let vt = vt_at(temp);
        let ratlog = (temp / tnom).ln();
        let ratio1 = temp / tnom - 1.0;
        let factlog = ratio1 * m.eg / vt + m.xti * ratlog;
        let factor = factlog.exp();
        let bfactor = (ratlog * m.xtb).exp();
        let (vje, cje) = junction_temp(m.vje, m.cje, m.mje, tnom, temp);
        let (vjc, cjc) = junction_temp(m.vjc, m.cjc, m.mjc, tnom, temp);
        let (vjs, cjs) = junction_temp(m.vjs, m.cjs, m.mjs, tnom, temp);
        Self {
            vt,
            is: m.is * factor,
            bf: m.bf * bfactor,
            br: m.br * bfactor,
            ise: m.ise * (factlog / m.ne).exp() / bfactor,
            isc: m.isc * (factlog / m.nc).exp() / bfactor,
            vje,
            cje,
            vjc,
            cjc,
            vjs,
            cjs,
        }
    }
}

/// Small-signal quantities at one bias point (normalised to NPN polarity).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct BjtOp {
    /// Collector current.
    pub ic: f64,
    /// Base current.
    pub ib: f64,
    /// Transconductance (SPICE convention: dIc/dVbe = gm + go).
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
    cbcn: f64,
}

/// Matrix entries of one BJT.
#[derive(Debug, Clone, Copy)]
struct Slots {
    rc: [Slot; 4],
    rb: [Slot; 4],
    re: [Slot; 4],
    cpcp: Slot,
    bpbp: Slot,
    epep: Slot,
    cpbp: Slot,
    cpep: Slot,
    bpcp: Slot,
    bpep: Slot,
    epcp: Slot,
    epbp: Slot,
    sub: [Slot; 4],
    bx: [Slot; 4],
}

/// A BJT instance (collector, base, emitter, optional substrate).
#[derive(Debug, Clone)]
pub struct Bjt {
    name: String,
    c: Var,
    b: Var,
    e: Var,
    sub: Var,
    ci: Var,
    bi: Var,
    ei: Var,
    m: BjtModel,
    t: BjtTemp,
    area: f64,
    off: bool,
    vcrit: f64,
    vbe_old: f64,
    vbc_old: f64,
    slots: Option<Slots>,
    state: usize,
}

// State layout: qbe, cqbe, qbc, cqbc, qbx, cqbx, qcs, cqcs, cexbc.
const QBE: usize = 0;
const QBC: usize = 2;
const QBX: usize = 4;
const QCS: usize = 6;
const CEXBC: usize = 8;

impl Bjt {
    /// BJT with collector `c`, base `b`, emitter `e` (substrate grounded),
    /// at the nominal temperature.
    pub fn new(name: &str, c: Var, b: Var, e: Var, model: BjtModel, area: f64, off: bool) -> Self {
        let t = BjtTemp::new(&model, model.tnom);
        Self {
            name: name.into(),
            c,
            b,
            e,
            sub: crate::circuit::GROUND,
            ci: c,
            bi: b,
            ei: e,
            vcrit: vcrit(t.vt, t.is * area),
            t,
            m: model,
            area,
            off,
            vbe_old: 0.0,
            vbc_old: 0.0,
            slots: None,
            state: 0,
        }
    }

    /// Connect the substrate terminal.
    pub fn with_substrate(mut self, sub: Var) -> Self {
        self.sub = sub;
        self
    }

    /// Evaluate at device temperature `temp` (K).
    pub fn with_temp(mut self, temp: f64) -> Self {
        self.t = BjtTemp::new(&self.m, temp);
        self.vcrit = vcrit(self.t.vt, self.t.is * self.area);
        self
    }

    /// DC Gummel-Poon evaluation at internal junction voltages (NPN sense).
    /// `gmin` is added across the leakage diodes, as in SPICE.
    pub fn dc(&self, vbe: f64, vbc: f64, gmin: f64) -> BjtOp {
        let m = &self.m;
        let t = &self.t;
        let a = self.area;
        let vt = t.vt;
        let (cbe, gbe) = junction(vbe, t.is * a, m.nf * vt);
        let (mut cben, mut gben) = if t.ise > 0.0 {
            junction(vbe, t.ise * a, m.ne * vt)
        } else {
            (0.0, 0.0)
        };
        cben += gmin * vbe;
        gben += gmin;
        let (cbc, gbc) = junction(vbc, t.is * a, m.nr * vt);
        let (mut cbcn, mut gbcn) = if t.isc > 0.0 {
            junction(vbc, t.isc * a, m.nc * vt)
        } else {
            (0.0, 0.0)
        };
        cbcn += gmin * vbc;
        gbcn += gmin;
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
        let ic = (cbe - cbc) / qb - cbc / t.br - cbcn;
        let ib = cbe / t.bf + cben + cbc / t.br + cbcn;
        let go = (gbc + (cbe - cbc) * dqbdvc / qb) / qb;
        let gm = (gbe - (cbe - cbc) * dqbdve / qb) / qb - go;
        let gpi = gbe / t.bf + gben;
        let gmu = gbc / t.br + gbcn;
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
            cbcn,
        }
    }

    /// Base resistance conductance at the current bias (RB/RBM/IRB).
    fn base_conductance(&self, qb: f64, ib: f64) -> f64 {
        let m = &self.m;
        if m.rb <= 0.0 {
            return 0.0;
        }
        let rbm = m.rbm.unwrap_or(m.rb).min(m.rb);
        let rbpr = rbm / self.area;
        let rbpi = m.rb / self.area - rbpr;
        let mut rx = rbpr + rbpi / qb;
        let xjrb = m.irb * self.area;
        if xjrb > 0.0 {
            let arg1 = (ib / xjrb).max(1e-9);
            let arg2 = (-1.0 + (1.0 + 14.59025 * arg1).sqrt()) / 2.4317 / arg1.sqrt();
            let tn = arg2.tan();
            rx = rbpr + 3.0 * rbpi * (tn - arg2) / arg2 / tn / tn;
        }
        if rx > 0.0 { 1.0 / rx } else { 0.0 }
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
        self.slots = Some(Slots {
            rc: s.slots2(c, cp),
            rb: s.slots2(b, bp),
            re: s.slots2(e, ep),
            cpcp: s.slot(cp, cp),
            bpbp: s.slot(bp, bp),
            epep: s.slot(ep, ep),
            cpbp: s.slot(cp, bp),
            cpep: s.slot(cp, ep),
            bpcp: s.slot(bp, cp),
            bpep: s.slot(bp, ep),
            epcp: s.slot(ep, cp),
            epbp: s.slot(ep, bp),
            sub: s.slots2(self.sub, cp),
            bx: s.slots2(b, cp),
        });
        self.state = s.states(9);
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
            if !cx.small_signal {
                let (v1, l1) = pnjlim(vbe, self.vbe_old, self.t.vt, self.vcrit);
                let (v2, l2) = pnjlim(vbc, self.vbc_old, self.t.vt, self.vcrit);
                vbe = v1;
                vbc = v2;
                cx.limited |= l1 || l2;
            }
        }
        self.vbe_old = vbe;
        self.vbc_old = vbc;
        let vbx = t * (cx.v(self.b) - cx.v(self.ci));
        let vcs = t * (cx.v(self.sub) - cx.v(self.ci));
        let op = self.dc(vbe, vbc, cx.gmin);
        let m = self.m.clone();
        let a = self.area;
        let st = self.state;

        // Excess phase (Weil's approximation, transient only).
        let td = m.ptf.to_radians() * m.tf;
        let (mut cex, mut gex, mut cc_ex) = (op.cbe, op.gbe, 0.0);
        if td > 0.0 {
            match cx.mode {
                Mode::TranInit => cx.states.now[st + CEXBC] = op.cbe / op.qb,
                Mode::Tran if !cx.small_signal => {
                    let h = cx.integ.h;
                    let arg1 = h / td;
                    let arg2 = 3.0 * arg1;
                    let arg1 = arg2 * arg1;
                    let denom = 1.0 + arg1 + arg2;
                    let arg3 = arg1 / denom;
                    let r = if cx.integ.h_prev > 0.0 {
                        h / cx.integ.h_prev
                    } else {
                        1.0
                    };
                    cc_ex = (cx.states.prev[st + CEXBC] * (1.0 + r + arg2) - cx.states.prev2[st + CEXBC] * r) / denom;
                    cex = op.cbe * arg3;
                    gex = op.gbe * arg3;
                    cx.states.now[st + CEXBC] = cc_ex + cex / op.qb;
                }
                _ => {}
            }
        }
        let tm = &self.t;
        let mut cc = cc_ex + (cex - op.cbc) / op.qb - op.cbc / tm.br - op.cbcn;
        let mut cb = op.ib;
        let go = (op.gbc + (cex - op.cbc) * op.dqbdvc / op.qb) / op.qb;
        let gm = (gex - (cex - op.cbc) * op.dqbdve / op.qb) / op.qb - go;
        let (mut gpi, mut gmu) = (op.gpi, op.gmu);
        let gx = self.base_conductance(op.qb, op.ib);
        let mut geqcb = 0.0;
        let (mut gbx, mut cbx_eq, mut gcs, mut ccs_eq) = (0.0, 0.0, 0.0, 0.0);

        if cx.mode != Mode::Dc {
            // Transit time with XTF/VTF/ITF modulation, then depletion charges.
            let (mut cbe_q, mut gbe_q) = (op.cbe, op.gbe);
            if m.tf > 0.0 && vbe > 0.0 {
                let (mut argtf, mut arg2, mut arg3) = (0.0, 0.0, 0.0);
                if m.xtf != 0.0 {
                    let ovtf = if m.vtf > 0.0 { 1.0 / (1.44 * m.vtf) } else { 0.0 };
                    argtf = m.xtf;
                    if ovtf != 0.0 {
                        argtf *= (vbc * ovtf).exp();
                    }
                    arg2 = argtf;
                    let xjtf = m.itf * a;
                    if xjtf != 0.0 {
                        let temp = op.cbe / (op.cbe + xjtf);
                        argtf *= temp * temp;
                        arg2 = argtf * (3.0 - temp - temp);
                    }
                    arg3 = op.cbe * argtf * ovtf;
                }
                cbe_q = op.cbe * (1.0 + argtf) / op.qb;
                gbe_q = (op.gbe * (1.0 + arg2) - cbe_q * op.dqbdve) / op.qb;
                geqcb = m.tf * (arg3 - cbe_q * op.dqbdvc) / op.qb;
            }
            let (qje, cje) = depletion(vbe, tm.cje * a, tm.vje, m.mje, m.fc);
            let ctot = tm.cjc * a;
            let (qjc, cjc) = depletion(vbc, ctot * m.xcjc, tm.vjc, m.mjc, m.fc);
            let (qjx, cjx) = depletion(vbx, ctot * (1.0 - m.xcjc), tm.vjc, m.mjc, m.fc);
            let (qjs, cjs) = substrate_charge(vcs, tm.cjs * a, tm.vjs, m.mjs);
            let qbe = m.tf * cbe_q + qje;
            let capbe = m.tf * gbe_q + cje;
            let qbc = m.tr * op.cbc + qjc;
            let capbc = m.tr * op.gbc + cjc;
            let now = &mut cx.states.now;
            now[st + QBE] = qbe;
            now[st + QBC] = qbc;
            now[st + QBX] = qjx;
            now[st + QCS] = qjs;
            if cx.mode == Mode::Tran {
                let (gbe_eq, cqbe) = cx.integrate(st + QBE, capbe);
                let (gbc_eq, cqbc) = cx.integrate(st + QBC, capbc);
                geqcb *= cx.integ.ag0;
                gpi += gbe_eq;
                cb += cqbe + cqbc;
                gmu += gbc_eq;
                cc -= cqbc;
                if ctot * (1.0 - m.xcjc) > 0.0 {
                    let (g, i) = cx.integrate(st + QBX, cjx);
                    gbx = g;
                    cbx_eq = t * (i - vbx * g);
                }
                if tm.cjs > 0.0 {
                    let (g, i) = cx.integrate(st + QCS, cjs);
                    gcs = g;
                    ccs_eq = t * (i - vcs * g);
                }
            } else {
                for k in [QBE, QBC, QBX, QCS] {
                    cx.states.now[st + k + 1] = 0.0;
                }
                geqcb = 0.0;
            }
        }

        let gcpr = if m.rc > 0.0 { a / m.rc } else { 0.0 };
        let gepr = if m.re > 0.0 { a / m.re } else { 0.0 };
        let ceqbe = t * (cc + cb - vbe * (gm + go + gpi) + vbc * (go - geqcb));
        let ceqbc = t * (-cc + vbe * (gm + go) - vbc * (gmu + go));
        cx.rhs(self.ci, ceqbc + cbx_eq + ccs_eq);
        cx.rhs(self.bi, -ceqbe - ceqbc);
        cx.rhs(self.ei, ceqbe);
        cx.rhs(self.b, -cbx_eq);
        cx.rhs(self.sub, -ccs_eq);
        let s = self.slots.expect("setup");
        cx.conductance(&s.rc, gcpr);
        cx.conductance(&s.rb, gx);
        cx.conductance(&s.re, gepr);
        cx.conductance(&s.bx, gbx);
        cx.conductance(&s.sub, gcs);
        cx.add(s.cpcp, gmu + go);
        cx.add(s.bpbp, gpi + gmu + geqcb);
        cx.add(s.epep, gpi + gm + go);
        cx.add(s.cpbp, -gmu + gm);
        cx.add(s.cpep, -gm - go);
        cx.add(s.bpcp, -gmu - geqcb);
        cx.add(s.bpep, -gpi);
        cx.add(s.epcp, -go + geqcb);
        cx.add(s.epbp, -gpi - gm - geqcb);
    }
}

/// Collector-substrate charge, SPICE3 form (linearised in forward bias).
fn substrate_charge(v: f64, cjs: f64, vjs: f64, mjs: f64) -> (f64, f64) {
    if cjs == 0.0 {
        return (0.0, 0.0);
    }
    if v < 0.0 {
        let arg = 1.0 - v / vjs;
        let sarg = (-mjs * arg.ln()).exp();
        (vjs * cjs * (1.0 - arg * sarg) / (1.0 - mjs), cjs * sarg)
    } else {
        (v * cjs * (1.0 + mjs * v / (2.0 * vjs)), cjs * (1.0 + mjs * v / vjs))
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

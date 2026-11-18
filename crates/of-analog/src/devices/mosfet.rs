//! MOSFET: Shichman-Hodges (SPICE level 1) and the semi-empirical SPICE
//! level 3 model, with body effect, drain/source series resistance, bulk
//! junction diodes and depletion capacitances (bottom and sidewall), Meyer
//! intrinsic gate capacitances from TOX, gate overlap capacitances, SPICE3
//! temperature scaling and SPICE FET voltage limiting.
//!
//! Level 3 derivatives come from forward-mode dual numbers, so they are
//! exact for the implemented current equations.

use std::collections::BTreeMap;
use std::ops::{Add, Div, Mul, Neg, Sub};

use super::limit::{CHARGE, REFTEMP, depletion, egfet, fetlim, junction_potential_temp, limvds, pnjlim, vcrit, vt_at};
use super::param;
use crate::circuit::{Device, LoadCtx, Mode, Setup, Var};
use crate::matrix::Slot;

/// Permittivity of free space (F/m), as used by SPICE.
const EPS0: f64 = 8.854_214_871e-12;
/// Oxide permittivity.
const EPSOX: f64 = 3.9 * EPS0;
/// Silicon permittivity.
const EPSSIL: f64 = 11.7 * EPS0;

/// Level-1/level-3 parameters (SPICE names).
#[derive(Debug, Clone, PartialEq)]
#[allow(missing_docs)]
pub struct MosModel {
    /// +1 NMOS, -1 PMOS.
    pub polarity: f64,
    /// 1 or 3.
    pub level: u8,
    pub vto: f64,
    pub kp: f64,
    pub gamma: f64,
    pub phi: f64,
    pub lambda: f64,
    pub rd: f64,
    pub rs: f64,
    pub is: f64,
    /// Bulk junction saturation current density (A/m²); used with AD/AS.
    pub js: f64,
    pub cgso: f64,
    pub cgdo: f64,
    pub cgbo: f64,
    pub ld: f64,
    /// Oxide capacitance per area (F/m²); 0 disables Meyer capacitances.
    pub cox: f64,
    /// Surface mobility (cm²/V·s).
    pub u0: f64,
    /// Zero-bias bulk-drain/bulk-source capacitances (F), overriding CJ.
    pub cbd: Option<f64>,
    pub cbs: Option<f64>,
    /// Bottom junction capacitance per area (F/m²) and grading.
    pub cj: f64,
    pub mj: f64,
    /// Sidewall junction capacitance per metre (F/m) and grading.
    pub cjsw: f64,
    pub mjsw: f64,
    /// Bulk junction potential (V).
    pub pb: f64,
    pub fc: f64,
    /// Nominal temperature (K).
    pub tnom: f64,
    // Level 3.
    pub eta: f64,
    pub theta: f64,
    pub kappa: f64,
    pub vmax: f64,
    pub xj: f64,
    pub delta: f64,
    pub nfs: f64,
    /// 2·εsi/(q·NSUB) (m²/V), 0 when NSUB is not given.
    pub alpha: f64,
}

impl MosModel {
    /// SPICE level-1 defaults.
    pub fn new(polarity: f64) -> Self {
        Self {
            polarity,
            level: 1,
            vto: 0.0,
            kp: 2e-5,
            gamma: 0.0,
            phi: 0.6,
            lambda: 0.0,
            rd: 0.0,
            rs: 0.0,
            is: 1e-14,
            js: 0.0,
            cgso: 0.0,
            cgdo: 0.0,
            cgbo: 0.0,
            ld: 0.0,
            cox: 0.0,
            u0: 600.0,
            cbd: None,
            cbs: None,
            cj: 0.0,
            mj: 0.5,
            cjsw: 0.0,
            mjsw: 0.5,
            pb: 0.8,
            fc: 0.5,
            tnom: REFTEMP,
            eta: 0.0,
            theta: 0.0,
            kappa: 0.2,
            vmax: 0.0,
            xj: 0.0,
            delta: 0.0,
            nfs: 0.0,
            alpha: 0.0,
        }
    }

    /// Build from `.model` parameters (level 1 or 3); returns names this
    /// model ignores. Missing KP, PHI, GAMMA and VTO are derived from TOX,
    /// UO, NSUB, NSS and TPG as in SPICE; `tnom` is in °C.
    pub fn from_params(polarity: f64, p: &BTreeMap<String, f64>) -> (Self, Vec<String>) {
        let level = if p.get("level").copied().unwrap_or(1.0) == 3.0 {
            3
        } else {
            1
        };
        let mut m = Self::new(polarity);
        m.level = level;
        if level == 3 {
            m.mjsw = 0.33;
        }
        const KNOWN: &[&str] = &[
            "level", "vto", "vt0", "kp", "gamma", "phi", "lambda", "rd", "rs", "is", "js", "cgso", "cgdo", "cgbo",
            "ld", "tox", "uo", "u0", "cbd", "cbs", "cj", "mj", "cjsw", "mjsw", "pb", "fc", "tnom", "nsub", "nss",
            "tpg", "kf", "af", "rsh",
        ];
        const KNOWN3: &[&str] = &["eta", "theta", "kappa", "vmax", "xj", "delta", "nfs"];
        let unknown = p
            .keys()
            .filter(|k| !KNOWN.contains(&k.as_str()) && !(level == 3 && KNOWN3.contains(&k.as_str())))
            .cloned()
            .collect();
        m.tnom = p.get("tnom").map_or(REFTEMP, |c| c + 273.15);
        m.u0 = param(p, &["uo", "u0"], 600.0);
        let tox = p.get("tox").copied().or(if level == 3 { Some(1e-7) } else { None });
        m.cox = tox.map_or(0.0, |t| EPSOX / t);
        m.kp = match p.get("kp") {
            Some(&kp) => kp,
            None if m.cox > 0.0 => m.u0 * m.cox * 1e-4,
            None => m.kp,
        };
        m.phi = param(p, &["phi"], m.phi);
        m.gamma = param(p, &["gamma"], m.gamma);
        m.vto = param(p, &["vto", "vt0"], m.vto);
        if let (Some(&nsub), true) = (p.get("nsub"), m.cox > 0.0) {
            let vtnom = vt_at(m.tnom);
            let eg1 = egfet(m.tnom);
            if nsub * 1e6 > 1.45e16 {
                if !p.contains_key("phi") {
                    m.phi = (2.0 * vtnom * (nsub * 1e6 / 1.45e16).ln()).max(0.1);
                }
                let fermis = polarity * 0.5 * m.phi;
                let tpg = p.get("tpg").copied().unwrap_or(1.0);
                let wkfng = if tpg != 0.0 {
                    3.25 + 0.5 * eg1 - polarity * tpg * 0.5 * eg1
                } else {
                    3.2
                };
                let wkfngs = wkfng - (3.25 + 0.5 * eg1 + fermis);
                if !p.contains_key("gamma") {
                    m.gamma = (2.0 * EPSSIL * CHARGE * nsub * 1e6).sqrt() / m.cox;
                }
                if !p.contains_key("vto") && !p.contains_key("vt0") {
                    let nss = p.get("nss").copied().unwrap_or(0.0);
                    let vfb = wkfngs - nss * 1e4 * CHARGE / m.cox;
                    m.vto = vfb + polarity * (m.gamma * m.phi.sqrt() + m.phi);
                }
            }
            m.alpha = 2.0 * EPSSIL / (CHARGE * nsub * 1e6);
        }
        m.lambda = param(p, &["lambda"], 0.0);
        m.rd = param(p, &["rd"], 0.0);
        m.rs = param(p, &["rs"], 0.0);
        m.is = param(p, &["is"], m.is);
        m.js = param(p, &["js"], 0.0);
        m.cgso = param(p, &["cgso"], 0.0);
        m.cgdo = param(p, &["cgdo"], 0.0);
        m.cgbo = param(p, &["cgbo"], 0.0);
        m.ld = param(p, &["ld"], 0.0);
        m.cbd = p.get("cbd").copied();
        m.cbs = p.get("cbs").copied();
        m.cj = param(p, &["cj"], 0.0);
        m.mj = param(p, &["mj"], m.mj);
        m.cjsw = param(p, &["cjsw"], 0.0);
        m.mjsw = param(p, &["mjsw"], m.mjsw);
        m.pb = param(p, &["pb"], m.pb);
        m.fc = param(p, &["fc"], m.fc);
        if level == 3 {
            m.eta = param(p, &["eta"], 0.0);
            m.theta = param(p, &["theta"], 0.0);
            m.kappa = param(p, &["kappa"], 0.2);
            m.vmax = param(p, &["vmax"], 0.0);
            m.xj = param(p, &["xj"], 0.0);
            m.delta = param(p, &["delta"], 0.0);
            m.nfs = param(p, &["nfs"], 0.0);
        }
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
    /// Turn-on voltage including body effect.
    pub von: f64,
    /// Saturation voltage.
    pub vdsat: f64,
}

/// Instance geometry for the bulk junctions.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct MosGeometry {
    /// Drain and source diffusion areas (m²).
    pub ad: f64,
    #[allow(missing_docs)]
    pub as_: f64,
    /// Drain and source diffusion perimeters (m).
    pub pd: f64,
    #[allow(missing_docs)]
    pub ps: f64,
}

/// Temperature- and geometry-dependent instance constants.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct Derived {
    vt: f64,
    phi: f64,
    /// type·tVbi.
    vbi: f64,
    vto: f64,
    beta: f64,
    u0: f64,
    is_d: f64,
    is_s: f64,
    vcrit_d: f64,
    vcrit_s: f64,
    pb: f64,
    czbd: f64,
    czbdsw: f64,
    czbs: f64,
    czbssw: f64,
    /// Total oxide capacitance Cox·Leff·W·M.
    oxide_cap: f64,
    cgs: f64,
    cgd: f64,
    cgb: f64,
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
    w: f64,
    l: f64,
    mult: f64,
    geom: MosGeometry,
    temp: f64,
    t: Derived,
    gdr: f64,
    gsr: f64,
    off: bool,
    old: (f64, f64, f64),
    von: f64,
    slots: [Slot; 22],
    state: usize,
}

// State layout (normalised polarity).
const QGS: usize = 0;
const QGD: usize = 2;
const QGB: usize = 4;
const QBD: usize = 6;
const QBS: usize = 8;
const VGS: usize = 10;
const VDS: usize = 11;
const VBS: usize = 12;
const CAPGS: usize = 13;
const CAPGD: usize = 14;
const CAPGB: usize = 15;
const N_STATES: usize = 16;

impl Mosfet {
    /// MOSFET with width `w`, length `l` (metres) and multiplier `mult`, at
    /// the nominal temperature and with no diffusion areas.
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
        let mut q = Self {
            name: name.into(),
            d,
            g,
            s,
            b,
            di: d,
            si: s,
            gdr: if model.rd > 0.0 { mult / model.rd } else { 0.0 },
            gsr: if model.rs > 0.0 { mult / model.rs } else { 0.0 },
            temp: model.tnom,
            m: model,
            w,
            l,
            mult,
            geom: MosGeometry::default(),
            t: Derived::default(),
            off,
            old: (0.0, 0.0, 0.0),
            von: 0.0,
            slots: [Slot::NONE; 22],
            state: 0,
        };
        q.derive();
        q.von = q.t.vto * q.m.polarity;
        q
    }

    /// Set the drain/source diffusion areas and perimeters.
    pub fn with_geometry(mut self, geom: MosGeometry) -> Self {
        self.geom = geom;
        self.derive();
        self
    }

    /// Evaluate at device temperature `temp` (K).
    pub fn with_temp(mut self, temp: f64) -> Self {
        self.temp = temp;
        self.derive();
        self.von = self.t.vto * self.m.polarity;
        self
    }

    fn derive(&mut self) {
        let m = &self.m;
        let (temp, tnom) = (self.temp, m.tnom);
        let vt = vt_at(temp);
        let vtnom = vt_at(tnom);
        let ratio = temp / tnom;
        let ratio4 = ratio * ratio.sqrt();
        let (eg, eg1) = (egfet(temp), egfet(tnom));
        let (_, phi) = junction_potential_temp(m.phi, tnom, temp);
        let ty = m.polarity;
        let tvbi = m.vto - ty * m.gamma * m.phi.sqrt() + 0.5 * (eg1 - eg) + ty * 0.5 * (phi - m.phi);
        let tvto = tvbi + ty * m.gamma * phi.sqrt();
        let sat = (-eg / vt + eg1 / vtnom).exp();
        let (is_d, is_s) = if m.js == 0.0 || self.geom.ad == 0.0 || self.geom.as_ == 0.0 {
            (m.is * sat * self.mult, m.is * sat * self.mult)
        } else {
            (
                m.js * sat * self.mult * self.geom.ad,
                m.js * sat * self.mult * self.geom.as_,
            )
        };
        // Bulk junction potential and capacitances.
        let (pbo, pb) = junction_potential_temp(m.pb, tnom, temp);
        let gmaold = (m.pb - pbo) / pbo;
        let gmanew = (pb - pbo) / pbo;
        let f =
            |mj: f64| (1.0 + mj * (4e-4 * (temp - REFTEMP) - gmanew)) / (1.0 + mj * (4e-4 * (tnom - REFTEMP) - gmaold));
        let (fj, fsw) = (f(m.mj), f(m.mjsw));
        let mult = self.mult;
        let czbd = match m.cbd {
            Some(c) => c * fj * mult,
            None => m.cj * fj * self.geom.ad * mult,
        };
        let czbs = match m.cbs {
            Some(c) => c * fj * mult,
            None => m.cj * fj * self.geom.as_ * mult,
        };
        let leff = self.leff();
        self.t = Derived {
            vt,
            phi,
            vbi: ty * tvbi,
            vto: tvto,
            beta: m.kp / ratio4 * self.w / leff * mult,
            u0: m.u0 / ratio4,
            is_d,
            is_s,
            vcrit_d: vcrit(vt, is_d),
            vcrit_s: vcrit(vt, is_s),
            pb,
            czbd,
            czbdsw: m.cjsw * fsw * self.geom.pd * mult,
            czbs,
            czbssw: m.cjsw * fsw * self.geom.ps * mult,
            oxide_cap: m.cox * leff * self.w * mult,
            cgs: m.cgso * self.w * mult,
            cgd: m.cgdo * self.w * mult,
            cgb: m.cgbo * leff * mult,
        };
    }

    fn leff(&self) -> f64 {
        self.l - 2.0 * self.m.ld
    }

    /// Channel current in normalised terms for `vgs`, `vds` (>= 0), `vbs`.
    pub fn channel(&self, vgs: f64, vds: f64, vbs: f64) -> MosOp {
        if self.m.level == 3 {
            self.level3(vgs, vds, vbs)
        } else {
            self.level1(vgs, vds, vbs)
        }
    }

    fn level1(&self, vgs: f64, vds: f64, vbs: f64) -> MosOp {
        let m = &self.m;
        let phi = self.t.phi;
        let sarg = if vbs <= 0.0 {
            (phi - vbs).sqrt()
        } else {
            let s = phi.sqrt();
            (s - vbs / (s + s)).max(0.0)
        };
        let von = self.t.vbi + m.gamma * sarg;
        let vgst = vgs - von;
        let vdsat = vgst.max(0.0);
        let arg = if sarg <= 0.0 { 0.0 } else { m.gamma / (sarg + sarg) };
        if vgst <= 0.0 {
            return MosOp {
                von,
                ..MosOp::default()
            };
        }
        let beta = self.t.beta;
        let betap = beta * (1.0 + m.lambda * vds);
        if vgst <= vds {
            let id = betap * vgst * vgst * 0.5;
            let gm = betap * vgst;
            MosOp {
                id,
                gm,
                gds: m.lambda * beta * vgst * vgst * 0.5,
                gmbs: gm * arg,
                von,
                vdsat,
            }
        } else {
            let id = betap * vds * (vgst - 0.5 * vds);
            let gm = betap * vds;
            let gds = betap * (vgst - vds) + m.lambda * beta * vds * (vgst - 0.5 * vds);
            MosOp {
                id,
                gm,
                gds,
                gmbs: gm * arg,
                von,
                vdsat,
            }
        }
    }

    /// SPICE level-3 drain current (Dang/Vladimirescu semi-empirical model).
    fn level3(&self, vgs: f64, vds: f64, vbs: f64) -> MosOp {
        const COEFF0: f64 = 0.063_135_3;
        const COEFF1: f64 = 0.801_329_2;
        const COEFF2: f64 = -0.011_107_77;
        let m = &self.m;
        let t = &self.t;
        let vgs = D::var(vgs, 0);
        let vds = D::var(vds, 1);
        let vbs = D::var(vbs, 2);
        let leff = self.leff();
        let weff = self.w;
        let eta = m.eta * 8.15e-22 / (m.cox * leff * leff * leff);
        let (phibs, sqphbs) = if vbs.v <= 0.0 {
            let p = -vbs + t.phi;
            (p, p.sqrt())
        } else {
            let sqphis = t.phi.sqrt();
            let s = sqphis / (vbs / (t.phi + t.phi) + 1.0);
            (s * s, s)
        };
        let coeff_dep = m.alpha.sqrt();
        let fshort = if m.xj != 0.0 && coeff_dep != 0.0 {
            let wps = sqphbs * coeff_dep;
            let oneoxj = 1.0 / m.xj;
            let xjonxl = m.xj / leff;
            let djonxj = m.ld * oneoxj;
            let wponxj = wps * oneoxj;
            let wconxj = wponxj * COEFF1 + wponxj * wponxj * COEFF2 + COEFF0;
            let arga = wconxj + djonxj;
            let argc = wponxj / (wponxj + 1.0);
            let argb = (-(argc * argc) + 1.0).sqrt();
            -((arga * argb - djonxj) * xjonxl) + 1.0
        } else {
            D::c(1.0)
        };
        let narrow = m.delta * 0.5 * std::f64::consts::PI * EPSSIL / m.cox;
        let gammas = fshort * m.gamma;
        let fbodys = gammas * 0.5 / (sqphbs + sqphbs);
        let fbody = fbodys + narrow / weff;
        let onfbdy = 1.0 / (fbody + 1.0);
        let qbonco = gammas * sqphbs + phibs * (narrow / weff);
        let vth = -(vds * eta) + t.vbi + qbonco;
        let mut von = vth;
        let mut xn = D::c(1.0);
        let off = |von: D, vdsat: f64| MosOp {
            von: von.v,
            vdsat,
            ..MosOp::default()
        };
        if m.nfs != 0.0 {
            let csonco = CHARGE * m.nfs * 1e4 / m.cox;
            let cdonco = qbonco / (phibs + phibs);
            xn = cdonco + (1.0 + csonco);
            von = vth + xn * t.vt;
        } else if vgs.v - von.v <= 0.0 {
            return off(von, 0.0);
        }
        let vgsx = if vgs.v > von.v { vgs } else { von };
        let onfg = (vgsx - vth) * m.theta + 1.0;
        let fgate = 1.0 / onfg;
        let us = fgate * (t.u0 * 1e-4);
        let mut vdsat = (vgsx - vth) * onfbdy;
        let mut vdsc = D::c(0.0);
        if m.vmax > 0.0 {
            vdsc = leff * m.vmax / us;
            let arga = (vgsx - vth) * onfbdy;
            let argb = (arga * arga + vdsc * vdsc).sqrt();
            vdsat = arga + vdsc - argb;
        }
        let vdsx = if vds.v < vdsat.v { vds } else { vdsat };
        if vdsx.v == 0.0 {
            return off(von, vdsat.v);
        }
        let cdo = vgsx - vth - (fbody + 1.0) * vdsx * 0.5;
        let cdnorm = cdo * vdsx;
        let mut cdrain = cdnorm * fgate * t.beta;
        let mut fdrain = D::c(0.0);
        if m.vmax > 0.0 {
            fdrain = 1.0 / (vdsx / vdsc + 1.0);
            cdrain = cdrain * fdrain;
        }
        // Channel-length modulation.
        let alpha = m.alpha;
        let delxl = if vds.v <= vdsat.v {
            if m.vmax > 0.0 || alpha == 0.0 {
                None
            } else {
                let arga = vds / vdsat;
                let dl = (vdsat * (m.kappa * alpha / 8.0)).sqrt();
                let a2 = arga * arga;
                Some(dl * (a2 * a2))
            }
        } else if m.vmax <= 0.0 {
            Some(((vds - vdsat) * (m.kappa * alpha)).sqrt())
        } else if alpha == 0.0 {
            None
        } else {
            let cdsat = cdrain;
            let mut gdsat = cdsat * (-fdrain + 1.0) / vdsc;
            if gdsat.v < 1e-12 {
                gdsat = D::c(1e-12);
            }
            // ngspice multiplies by KAPPA here (the uncorrected SPICE3 form,
            // without it, is ngspice's `badmos3` option).
            let emax = cdsat / leff / gdsat * m.kappa;
            let arga = emax * (0.5 * alpha);
            let argc = m.kappa * alpha;
            let argb = (arga * arga + (vds - vdsat) * argc).sqrt();
            Some(argb - arga)
        };
        if let Some(mut dl) = delxl {
            if dl.v > 0.5 * leff {
                dl = -(leff * leff * 0.25 / dl) + leff;
            }
            let xlfact = 1.0 / (-(dl / leff) + 1.0);
            cdrain = cdrain * xlfact;
        }
        // Weak inversion.
        if vgs.v < von.v {
            let wfact = ((vgs - von) / (xn * t.vt)).exp();
            cdrain = cdrain * wfact;
        }
        MosOp {
            id: cdrain.v,
            gm: cdrain.d[0],
            gds: cdrain.d[1],
            gmbs: cdrain.d[2],
            von: von.v,
            vdsat: vdsat.v,
        }
    }

    fn bulk_diode(&self, v: f64, is: f64, gmin: f64) -> (f64, f64) {
        let vt = self.t.vt;
        if v <= -3.0 * vt {
            (gmin * v - is, gmin)
        } else {
            let e = (v / vt).min(80.0).exp();
            (is * (e - 1.0) + gmin * v, is * e / vt + gmin)
        }
    }

    /// Bulk junction charge and capacitance (bottom plus sidewall).
    fn bulk_charge(&self, v: f64, cz: f64, czsw: f64) -> (f64, f64) {
        let (q1, c1) = depletion(v, cz, self.t.pb, self.m.mj, self.m.fc);
        let (q2, c2) = depletion(v, czsw, self.t.pb, self.m.mjsw, self.m.fc);
        (q1 + q2, c1 + c2)
    }
}

/// Meyer gate capacitances (half values, as SPICE's DEVqmeyer) for the
/// device in its forward orientation.
fn qmeyer(vgs: f64, vgd: f64, von: f64, vdsat: f64, phi: f64, cox: f64) -> (f64, f64, f64) {
    const MAGIC_VDS: f64 = 0.025;
    let vgst = vgs - von;
    let vdsat = vdsat.max(MAGIC_VDS);
    let vds = vgs - vgd;
    let split = |cap: f64| {
        if vds >= vdsat {
            (cap, 0.0)
        } else {
            let vddif = 2.0 * vdsat - vds;
            let vddif1 = vdsat - vds;
            let vddif2 = vddif * vddif;
            (
                cap * (1.0 - vddif1 * vddif1 / vddif2),
                cap * (1.0 - vdsat * vdsat / vddif2),
            )
        }
    };
    if vgst <= -phi {
        (0.0, 0.0, cox / 2.0)
    } else if vgst <= -phi / 2.0 {
        (0.0, 0.0, -vgst * cox / (2.0 * phi))
    } else if vgst <= 0.0 {
        let (cgs, cgd) = split(vgst * cox / (1.5 * phi) + cox / 3.0);
        (cgs, cgd, -vgst * cox / (2.0 * phi))
    } else {
        let (cgs, cgd) = split(cox / 3.0);
        (cgs, cgd, 0.0)
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
        self.state = s.states(N_STATES);
    }
    fn load(&mut self, cx: &mut LoadCtx<'_>) {
        let t = self.m.polarity;
        let vt = self.t.vt;
        let (mut vgs, mut vds, mut vbs);
        if cx.init_junctions {
            if self.off {
                (vgs, vds, vbs) = (0.0, 0.0, 0.0);
            } else {
                (vgs, vds, vbs) = (t * self.t.vto, 0.0, -1.0);
            }
        } else {
            vbs = t * (cx.v(self.b) - cx.v(self.si));
            vgs = t * (cx.v(self.g) - cx.v(self.si));
            vds = t * (cx.v(self.di) - cx.v(self.si));
            if !cx.small_signal {
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
                    (vbs, lim) = pnjlim(vbs, vbs_o, vt, self.t.vcrit_s);
                } else {
                    let (vbd, l) = pnjlim(vbs - vds, vbs_o - vds_o, vt, self.t.vcrit_d);
                    vbs = vbd + vds;
                    lim = l;
                }
                cx.limited |= lim;
            }
        }
        self.old = (vgs, vds, vbs);
        let vbd = vbs - vds;
        let vgd = vgs - vds;
        let vgb = vgs - vbs;
        let (mut cbs, mut gbs) = self.bulk_diode(vbs, self.t.is_s, cx.gmin);
        let (mut cbd, mut gbd) = self.bulk_diode(vbd, self.t.is_d, cx.gmin);
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

        let (mut gcgs, mut gcgd, mut gcgb) = (0.0, 0.0, 0.0);
        let (mut ceqgs, mut ceqgd, mut ceqgb) = (0.0, 0.0, 0.0);
        if cx.mode != Mode::Dc {
            let st = self.state;
            let d = self.t;
            // Bulk junction depletion charges.
            let (qbd, capbd) = self.bulk_charge(vbd, d.czbd, d.czbdsw);
            let (qbs, capbs) = self.bulk_charge(vbs, d.czbs, d.czbssw);
            // Meyer capacitances (half values) in the device orientation.
            let (hgs, hgd, hgb) = if d.oxide_cap > 0.0 {
                if forward {
                    qmeyer(vgs, vgd, op.von, op.vdsat, d.phi, d.oxide_cap)
                } else {
                    let (a, b, c) = qmeyer(vgd, vgs, op.von, op.vdsat, d.phi, d.oxide_cap);
                    (b, a, c)
                }
            } else {
                (0.0, 0.0, 0.0)
            };
            let now = &mut cx.states.now;
            now[st + VGS] = vgs;
            now[st + VDS] = vds;
            now[st + VBS] = vbs;
            now[st + CAPGS] = hgs;
            now[st + CAPGD] = hgd;
            now[st + CAPGB] = hgb;
            now[st + QBD] = qbd;
            now[st + QBS] = qbs;
            let op_like = cx.mode == Mode::TranInit || cx.small_signal;
            let prev = &cx.states.prev;
            let (capgs, capgd, capgb) = if op_like {
                (2.0 * hgs + d.cgs, 2.0 * hgd + d.cgd, 2.0 * hgb + d.cgb)
            } else {
                (
                    hgs + prev[st + CAPGS] + d.cgs,
                    hgd + prev[st + CAPGD] + d.cgd,
                    hgb + prev[st + CAPGB] + d.cgb,
                )
            };
            let (qgs, qgd, qgb) = if op_like {
                (vgs * capgs, vgd * capgd, vgb * capgb)
            } else {
                let vgs1 = prev[st + VGS];
                let vgd1 = vgs1 - prev[st + VDS];
                let vgb1 = vgs1 - prev[st + VBS];
                (
                    (vgs - vgs1) * capgs + prev[st + QGS],
                    (vgd - vgd1) * capgd + prev[st + QGD],
                    (vgb - vgb1) * capgb + prev[st + QGB],
                )
            };
            let now = &mut cx.states.now;
            now[st + QGS] = qgs;
            now[st + QGD] = qgd;
            now[st + QGB] = qgb;
            if cx.mode == Mode::Tran {
                let (g, i) = cx.integrate(st + QBD, capbd);
                gbd += g;
                cbd += i;
                let (g, i) = cx.integrate(st + QBS, capbs);
                gbs += g;
                cbs += i;
                let mut gate = |k: usize, cap: f64, v: f64| {
                    if cap == 0.0 {
                        cx.states.now[st + k + 1] = 0.0;
                        return (0.0, 0.0);
                    }
                    let (g, i) = cx.integrate(st + k, cap);
                    (g, t * (i - g * v))
                };
                (gcgs, ceqgs) = gate(QGS, capgs, vgs);
                (gcgd, ceqgd) = gate(QGD, capgd, vgd);
                (gcgb, ceqgb) = gate(QGB, capgb, vgb);
            } else {
                for k in [QGS, QGD, QGB, QBD, QBS] {
                    cx.states.now[st + k + 1] = 0.0;
                }
            }
        }
        let ceqbs = t * (cbs - gbs * vbs);
        let ceqbd = t * (cbd - gbd * vbd);

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

/// Forward-mode dual number with three partial derivatives
/// (with respect to vgs, vds and vbs).
#[derive(Debug, Clone, Copy, PartialEq)]
struct D {
    v: f64,
    d: [f64; 3],
}

impl D {
    fn c(v: f64) -> Self {
        Self { v, d: [0.0; 3] }
    }
    fn var(v: f64, i: usize) -> Self {
        let mut d = [0.0; 3];
        d[i] = 1.0;
        Self { v, d }
    }
    fn map(self, f: f64, df: f64) -> Self {
        Self {
            v: f,
            d: self.d.map(|x| x * df),
        }
    }
    fn sqrt(self) -> Self {
        let r = self.v.sqrt();
        self.map(r, if r > 0.0 { 0.5 / r } else { 0.0 })
    }
    fn exp(self) -> Self {
        let e = self.v.exp();
        self.map(e, e)
    }
}

impl Add for D {
    type Output = D;
    fn add(self, o: D) -> D {
        D {
            v: self.v + o.v,
            d: [self.d[0] + o.d[0], self.d[1] + o.d[1], self.d[2] + o.d[2]],
        }
    }
}
impl Sub for D {
    type Output = D;
    fn sub(self, o: D) -> D {
        self + (-o)
    }
}
impl Neg for D {
    type Output = D;
    fn neg(self) -> D {
        D {
            v: -self.v,
            d: self.d.map(|x| -x),
        }
    }
}
// Product and quotient rules legitimately mix + and - with * and /.
#[allow(clippy::suspicious_arithmetic_impl)]
impl Mul for D {
    type Output = D;
    fn mul(self, o: D) -> D {
        D {
            v: self.v * o.v,
            d: [0, 1, 2].map(|i| self.d[i] * o.v + o.d[i] * self.v),
        }
    }
}
#[allow(clippy::suspicious_arithmetic_impl)]
impl Div for D {
    type Output = D;
    fn div(self, o: D) -> D {
        let inv = 1.0 / o.v;
        D {
            v: self.v * inv,
            d: [0, 1, 2].map(|i| (self.d[i] - self.v * inv * o.d[i]) * inv),
        }
    }
}
impl Add<f64> for D {
    type Output = D;
    fn add(self, o: f64) -> D {
        D {
            v: self.v + o,
            d: self.d,
        }
    }
}
impl Sub<f64> for D {
    type Output = D;
    fn sub(self, o: f64) -> D {
        D {
            v: self.v - o,
            d: self.d,
        }
    }
}
impl Mul<f64> for D {
    type Output = D;
    fn mul(self, o: f64) -> D {
        D {
            v: self.v * o,
            d: self.d.map(|x| x * o),
        }
    }
}
impl Div<f64> for D {
    type Output = D;
    fn div(self, o: f64) -> D {
        self * (1.0 / o)
    }
}
impl Add<D> for f64 {
    type Output = D;
    fn add(self, o: D) -> D {
        o + self
    }
}
impl Mul<D> for f64 {
    type Output = D;
    fn mul(self, o: D) -> D {
        o * self
    }
}
impl Div<D> for f64 {
    type Output = D;
    fn div(self, o: D) -> D {
        D::c(self) / o
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

    fn nmos3() -> Mosfet {
        let p: BTreeMap<String, f64> = [
            ("level", 3.0),
            ("vto", 0.8),
            ("kp", 60e-6),
            ("gamma", 0.5),
            ("phi", 0.7),
            ("tox", 20e-9),
            ("nsub", 1e16),
            ("theta", 0.06),
            ("eta", 0.1),
            ("kappa", 0.3),
            ("vmax", 1.5e5),
            ("xj", 0.2e-6),
            ("nfs", 1e11),
            ("delta", 0.5),
            ("ld", 0.1e-6),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect();
        let (m, unknown) = MosModel::from_params(1.0, &p);
        assert!(unknown.is_empty(), "{unknown:?}");
        Mosfet::new("m3", 1, 2, 0, 0, m, 10e-6, 2e-6, 1.0, false)
    }

    #[test]
    fn level3_derivatives_match_finite_differences() {
        let q = nmos3();
        let dv = 1e-6;
        for (vgs, vds, vbs) in [(1.5, 3.0, -0.5), (2.5, 0.3, -1.0), (0.6, 1.0, 0.0), (3.0, 1.2, 0.2)] {
            let c = q.channel(vgs, vds, vbs);
            assert!(c.id > 0.0);
            let fd = |a: (f64, f64, f64), b: (f64, f64, f64)| {
                (q.channel(a.0, a.1, a.2).id - q.channel(b.0, b.1, b.2).id) / (2.0 * dv)
            };
            let gm = fd((vgs + dv, vds, vbs), (vgs - dv, vds, vbs));
            let gds = fd((vgs, vds + dv, vbs), (vgs, vds - dv, vbs));
            let gmbs = fd((vgs, vds, vbs + dv), (vgs, vds, vbs - dv));
            let tol = 1e-5 * c.id / 0.01 + 1e-12;
            assert!((gm - c.gm).abs() < tol.max(1e-4 * c.gm.abs()), "gm {gm} {}", c.gm);
            assert!((gds - c.gds).abs() < tol.max(1e-4 * c.gds.abs()), "gds {gds} {}", c.gds);
            assert!(
                (gmbs - c.gmbs).abs() < tol.max(1e-4 * c.gmbs.abs()),
                "gmbs {gmbs} {}",
                c.gmbs
            );
        }
    }

    #[test]
    fn meyer_capacitances_sum_to_cox_in_strong_inversion() {
        let cox = 1e-14;
        // Saturation: Cgs = 2/3 Cox (halves returned), Cgd = 0.
        let (cgs, cgd, cgb) = qmeyer(3.0, 0.0, 0.7, 2.3, 0.6, cox);
        assert!((2.0 * cgs - 2.0 / 3.0 * cox).abs() < 1e-20 && cgd == 0.0 && cgb == 0.0);
        // Linear at vds = 0: Cgs = Cgd = Cox/2.
        let (cgs, cgd, _) = qmeyer(3.0, 3.0, 0.7, 2.3, 0.6, cox);
        assert!((2.0 * cgs - cox / 2.0).abs() < 1e-20 && (2.0 * cgd - cox / 2.0).abs() < 1e-20);
        // Accumulation: all to the bulk.
        let (cgs, cgd, cgb) = qmeyer(-2.0, -2.0, 0.7, 0.0, 0.6, cox);
        assert!(cgs == 0.0 && cgd == 0.0 && (2.0 * cgb - cox).abs() < 1e-20);
    }
}

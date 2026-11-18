//! Independent and controlled sources.

use crate::circuit::{Device, LoadCtx, Mode, Setup, TranSpec, Var};
use crate::matrix::Slot;
use crate::waveform::Wave;

/// Value specification of an independent source.
#[derive(Debug, Clone, PartialEq)]
pub struct SourceValue {
    /// DC value used for operating points (falls back to the waveform at t = 0).
    pub dc: Option<f64>,
    /// Transient waveform (falls back to `dc`).
    pub wave: Option<Wave>,
}

impl SourceValue {
    /// A constant source.
    pub fn dc(v: f64) -> Self {
        Self {
            dc: Some(v),
            wave: None,
        }
    }

    /// A source with a transient waveform (DC value from the waveform at 0).
    pub fn wave(w: Wave) -> Self {
        Self {
            dc: None,
            wave: Some(w),
        }
    }

    fn at(&self, cx: &LoadCtx<'_>) -> f64 {
        let v = match (cx.mode, &self.wave, self.dc) {
            (Mode::Dc, Some(w), _) if cx.tran_op => w.value(0.0, &cx.tran),
            (Mode::Dc, _, Some(dc)) => dc,
            (Mode::Dc, Some(w), None) => w.value(0.0, &cx.tran),
            (_, Some(w), _) => w.value(cx.time, &cx.tran),
            (_, None, dc) => dc.unwrap_or(0.0),
        };
        v * cx.src_factor
    }

    fn next_breakpoint(&self, t: f64, tran: &TranSpec) -> Option<f64> {
        self.wave.as_ref().and_then(|w| w.next_breakpoint(t, tran))
    }
}

/// Independent voltage source; branch current `i(name)` flows from `p`
/// through the source to `n` (SPICE convention).
#[derive(Debug, Clone)]
pub struct VoltageSource {
    name: String,
    p: Var,
    n: Var,
    /// Value.
    pub value: SourceValue,
    /// Small-signal magnitude and phase (degrees) for `.ac`.
    pub ac: Option<(f64, f64)>,
    b: Var,
    s: [Slot; 4],
}

impl VoltageSource {
    /// Source with `p` positive.
    pub fn new(name: &str, p: Var, n: Var, value: SourceValue) -> Self {
        Self {
            name: name.into(),
            p,
            n,
            value,
            ac: None,
            b: 0,
            s: [Slot::NONE; 4],
        }
    }

    /// Set the `.ac` excitation (magnitude, phase in degrees).
    pub fn with_ac(mut self, ac: Option<(f64, f64)>) -> Self {
        self.ac = ac;
        self
    }
}

impl Device for VoltageSource {
    fn name(&self) -> &str {
        &self.name
    }
    fn setup_priority(&self) -> u8 {
        0
    }
    fn setup(&mut self, s: &mut Setup<'_>) {
        self.b = s.branch(&self.name);
        self.s = [
            s.slot(self.p, self.b),
            s.slot(self.n, self.b),
            s.slot(self.b, self.p),
            s.slot(self.b, self.n),
        ];
    }
    fn branch(&self) -> Option<Var> {
        Some(self.b)
    }
    fn load(&mut self, cx: &mut LoadCtx<'_>) {
        cx.add(self.s[0], 1.0);
        cx.add(self.s[1], -1.0);
        cx.add(self.s[2], 1.0);
        cx.add(self.s[3], -1.0);
        let v = self.value.at(cx);
        cx.rhs(self.b, v);
    }
    fn next_breakpoint(&self, t: f64, tran: &TranSpec) -> Option<f64> {
        self.value.next_breakpoint(t, tran)
    }
    fn set_dc(&mut self, value: f64) -> bool {
        self.value.dc = Some(value);
        true
    }
    fn ac_stimulus(&self, re: &mut [f64], im: &mut [f64]) {
        if let Some((mag, ph)) = self.ac {
            let (s, c) = ph.to_radians().sin_cos();
            re[self.b] += mag * c;
            im[self.b] += mag * s;
        }
    }
}

/// Independent current source: positive current flows from `p` through
/// the source to `n` (out of node `p`, into node `n`).
#[derive(Debug, Clone)]
pub struct CurrentSource {
    name: String,
    p: Var,
    n: Var,
    /// Value.
    pub value: SourceValue,
    /// Small-signal magnitude and phase (degrees) for `.ac`.
    pub ac: Option<(f64, f64)>,
}

impl CurrentSource {
    /// Current source from `p` to `n`.
    pub fn new(name: &str, p: Var, n: Var, value: SourceValue) -> Self {
        Self {
            name: name.into(),
            p,
            n,
            value,
            ac: None,
        }
    }

    /// Set the `.ac` excitation (magnitude, phase in degrees).
    pub fn with_ac(mut self, ac: Option<(f64, f64)>) -> Self {
        self.ac = ac;
        self
    }
}

impl Device for CurrentSource {
    fn name(&self) -> &str {
        &self.name
    }
    fn setup(&mut self, _s: &mut Setup<'_>) {}
    fn load(&mut self, cx: &mut LoadCtx<'_>) {
        let i = self.value.at(cx);
        cx.current(self.p, self.n, i);
    }
    fn next_breakpoint(&self, t: f64, tran: &TranSpec) -> Option<f64> {
        self.value.next_breakpoint(t, tran)
    }
    fn set_dc(&mut self, value: f64) -> bool {
        self.value.dc = Some(value);
        true
    }
    fn ac_stimulus(&self, re: &mut [f64], im: &mut [f64]) {
        if let Some((mag, ph)) = self.ac {
            let (s, c) = ph.to_radians().sin_cos();
            re[self.p] -= mag * c;
            im[self.p] -= mag * s;
            re[self.n] += mag * c;
            im[self.n] += mag * s;
        }
    }
}

/// Which linear controlled source.
#[derive(Debug, Clone, PartialEq)]
pub enum Control {
    /// E: `v(p,n) = gain * v(cp,cn)`.
    Vcvs {
        /// Controlling positive node.
        cp: Var,
        /// Controlling negative node.
        cn: Var,
    },
    /// G: current `gain * v(cp,cn)` from `p` to `n`.
    Vccs {
        /// Controlling positive node.
        cp: Var,
        /// Controlling negative node.
        cn: Var,
    },
    /// F: current `gain * i(source)` from `p` to `n`.
    Cccs {
        /// Controlling voltage source name.
        source: String,
    },
    /// H: `v(p,n) = gain * i(source)`.
    Ccvs {
        /// Controlling voltage source name.
        source: String,
    },
}

/// Linear controlled source (E, G, F, H).
#[derive(Debug, Clone)]
pub struct ControlledSource {
    name: String,
    p: Var,
    n: Var,
    /// Gain, transconductance or transresistance.
    pub gain: f64,
    /// Kind and controlling quantity.
    pub control: Control,
    b: Var,
    s: Vec<(Slot, f64)>,
    /// Set if the controlling source could not be found at setup.
    pub missing_control: bool,
}

impl ControlledSource {
    /// New controlled source.
    pub fn new(name: &str, p: Var, n: Var, gain: f64, control: Control) -> Self {
        Self {
            name: name.into(),
            p,
            n,
            gain,
            control,
            b: 0,
            s: Vec::new(),
            missing_control: false,
        }
    }
}

impl Device for ControlledSource {
    fn name(&self) -> &str {
        &self.name
    }
    fn setup_priority(&self) -> u8 {
        match self.control {
            Control::Vcvs { .. } => 0,
            _ => 2,
        }
    }
    fn setup(&mut self, s: &mut Setup<'_>) {
        let (p, n, g) = (self.p, self.n, self.gain);
        let mut entries = Vec::new();
        let ctrl_branch = |s: &Setup<'_>, src: &str| s.branch_of(src);
        match &self.control {
            Control::Vcvs { cp, cn } => {
                self.b = s.branch(&self.name);
                let b = self.b;
                entries.extend([
                    (s.slot(p, b), 1.0),
                    (s.slot(n, b), -1.0),
                    (s.slot(b, p), 1.0),
                    (s.slot(b, n), -1.0),
                ]);
                entries.extend([(s.slot(b, *cp), -g), (s.slot(b, *cn), g)]);
            }
            Control::Vccs { cp, cn } => {
                entries.extend([
                    (s.slot(p, *cp), g),
                    (s.slot(p, *cn), -g),
                    (s.slot(n, *cp), -g),
                    (s.slot(n, *cn), g),
                ]);
            }
            Control::Cccs { source } => match ctrl_branch(s, source) {
                Some(bc) => entries.extend([(s.slot(p, bc), g), (s.slot(n, bc), -g)]),
                None => self.missing_control = true,
            },
            Control::Ccvs { source } => {
                self.b = s.branch(&self.name);
                let b = self.b;
                entries.extend([
                    (s.slot(p, b), 1.0),
                    (s.slot(n, b), -1.0),
                    (s.slot(b, p), 1.0),
                    (s.slot(b, n), -1.0),
                ]);
                match ctrl_branch(s, source) {
                    Some(bc) => entries.push((s.slot(b, bc), -g)),
                    None => self.missing_control = true,
                }
            }
        }
        self.s = entries;
    }
    fn branch(&self) -> Option<Var> {
        matches!(self.control, Control::Vcvs { .. } | Control::Ccvs { .. }).then_some(self.b)
    }
    fn load(&mut self, cx: &mut LoadCtx<'_>) {
        for &(slot, v) in &self.s {
            cx.add(slot, v);
        }
    }
}

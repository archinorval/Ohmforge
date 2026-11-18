//! Resistor, capacitor and inductor.

use crate::circuit::{Device, GROUND, LoadCtx, Mode, Setup, Var};
use crate::matrix::Slot;

/// Linear resistor.
#[derive(Debug, Clone)]
pub struct Resistor {
    name: String,
    p: Var,
    n: Var,
    /// Resistance in ohms.
    pub ohms: f64,
    s: [Slot; 4],
}

impl Resistor {
    /// Resistor of `ohms` between `p` and `n`.
    pub fn new(name: &str, p: Var, n: Var, ohms: f64) -> Self {
        Self {
            name: name.into(),
            p,
            n,
            ohms,
            s: [Slot::NONE; 4],
        }
    }
}

impl Device for Resistor {
    fn name(&self) -> &str {
        &self.name
    }
    fn setup(&mut self, s: &mut Setup<'_>) {
        self.s = s.slots2(self.p, self.n);
    }
    fn load(&mut self, cx: &mut LoadCtx<'_>) {
        cx.conductance(&self.s, 1.0 / self.ohms);
    }
}

/// Linear capacitor with optional initial voltage (used with `uic`).
#[derive(Debug, Clone)]
pub struct Capacitor {
    name: String,
    p: Var,
    n: Var,
    /// Capacitance in farads.
    pub farads: f64,
    /// Initial voltage for `uic` runs.
    pub ic: Option<f64>,
    s: [Slot; 4],
    state: usize,
}

impl Capacitor {
    /// Capacitor of `farads` between `p` and `n`.
    pub fn new(name: &str, p: Var, n: Var, farads: f64, ic: Option<f64>) -> Self {
        Self {
            name: name.into(),
            p,
            n,
            farads,
            ic,
            s: [Slot::NONE; 4],
            state: 0,
        }
    }
}

impl Device for Capacitor {
    fn name(&self) -> &str {
        &self.name
    }
    fn setup(&mut self, s: &mut Setup<'_>) {
        self.s = s.slots2(self.p, self.n);
        self.state = s.states(2);
    }
    fn initial_condition(&self, x: &mut [f64]) {
        if let Some(ic) = self.ic {
            if self.n == GROUND {
                x[self.p] = ic;
            } else if self.p == GROUND {
                x[self.n] = -ic;
            } else {
                x[self.p] = x[self.n] + ic;
            }
        }
    }
    fn load(&mut self, cx: &mut LoadCtx<'_>) {
        let v = cx.v(self.p) - cx.v(self.n);
        match cx.mode {
            Mode::Dc => cx.states.now[self.state] = self.farads * v,
            Mode::TranInit => {
                let v0 = self.ic.unwrap_or(v);
                cx.states.now[self.state] = self.farads * v0;
                cx.states.now[self.state + 1] = 0.0;
            }
            Mode::Tran => {
                cx.states.now[self.state] = self.farads * v;
                let (geq, i) = cx.integrate(self.state, self.farads);
                cx.conductance(&self.s, geq);
                cx.current(self.p, self.n, i - geq * v);
            }
        }
    }
}

/// Linear inductor; its current is an MNA unknown `i(name)`.
#[derive(Debug, Clone)]
pub struct Inductor {
    name: String,
    p: Var,
    n: Var,
    /// Inductance in henries.
    pub henries: f64,
    /// Initial current for `uic` runs.
    pub ic: Option<f64>,
    b: Var,
    pb: Slot,
    nb: Slot,
    bp: Slot,
    bn: Slot,
    bb: Slot,
    state: usize,
}

impl Inductor {
    /// Inductor of `henries` from `p` to `n`.
    pub fn new(name: &str, p: Var, n: Var, henries: f64, ic: Option<f64>) -> Self {
        Self {
            name: name.into(),
            p,
            n,
            henries,
            ic,
            b: 0,
            pb: Slot::NONE,
            nb: Slot::NONE,
            bp: Slot::NONE,
            bn: Slot::NONE,
            bb: Slot::NONE,
            state: 0,
        }
    }
}

impl Device for Inductor {
    fn name(&self) -> &str {
        &self.name
    }
    fn setup_priority(&self) -> u8 {
        0
    }
    fn setup(&mut self, s: &mut Setup<'_>) {
        self.b = s.branch(&self.name);
        self.pb = s.slot(self.p, self.b);
        self.nb = s.slot(self.n, self.b);
        self.bp = s.slot(self.b, self.p);
        self.bn = s.slot(self.b, self.n);
        self.bb = s.slot(self.b, self.b);
        self.state = s.states(2);
    }
    fn branch(&self) -> Option<Var> {
        Some(self.b)
    }
    fn initial_condition(&self, x: &mut [f64]) {
        if let Some(i) = self.ic {
            x[self.b] = i;
        }
    }
    fn load(&mut self, cx: &mut LoadCtx<'_>) {
        cx.add(self.pb, 1.0);
        cx.add(self.nb, -1.0);
        cx.add(self.bp, 1.0);
        cx.add(self.bn, -1.0);
        let i = cx.v(self.b);
        match cx.mode {
            Mode::Dc => cx.states.now[self.state] = self.henries * i,
            Mode::TranInit => {
                cx.states.now[self.state] = self.henries * i;
                cx.states.now[self.state + 1] = 0.0;
            }
            Mode::Tran => {
                cx.states.now[self.state] = self.henries * i;
                let (req, v) = cx.integrate(self.state, self.henries);
                // v_p - v_n - req * i = v - req * i
                cx.add(self.bb, -req);
                cx.rhs(self.b, v - req * i);
            }
        }
    }
}

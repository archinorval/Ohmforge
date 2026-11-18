//! Numerical integration of device charges: backward Euler (first step and
//! after breakpoints), trapezoidal (default) and variable-step Gear-2 (BDF2).

/// Integration method for transient analysis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Method {
    /// Trapezoidal rule (second order, SPICE default).
    #[default]
    Trapezoidal,
    /// Gear second order (BDF2): damps trapezoidal ringing.
    Gear2,
}

/// Charge/current history. Each device state `k` is a charge (or flux) and
/// `k + 1` its time derivative (current or voltage).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct States {
    /// Values at the time point being solved.
    pub now: Vec<f64>,
    /// Last accepted time point.
    pub prev: Vec<f64>,
    /// The accepted point before that.
    pub prev2: Vec<f64>,
    /// `charges[k]` is set once state `k` has been integrated (it then
    /// takes part in the truncation-error estimate).
    pub charges: Vec<bool>,
}

impl States {
    /// `n` zeroed slots.
    pub fn new(n: usize) -> Self {
        Self {
            now: vec![0.0; n],
            prev: vec![0.0; n],
            prev2: vec![0.0; n],
            charges: vec![false; n],
        }
    }

    /// Initialise history from the current values (start of a transient).
    pub fn seed(&mut self) {
        self.prev.clone_from(&self.now);
        self.prev2.clone_from(&self.now);
    }

    /// Accept the current time point.
    pub fn rotate(&mut self) {
        std::mem::swap(&mut self.prev2, &mut self.prev);
        self.prev.clone_from(&self.now);
    }
}

/// Coefficients for one time step: `dq/dt ~ ag0 q_n + history`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Integrator {
    /// Method in use.
    pub method: Method,
    /// 1 = backward Euler, 2 = trapezoidal or Gear-2.
    pub order: u8,
    /// Current step.
    pub h: f64,
    /// Previous accepted step (equal to `h` on the first step).
    pub h_prev: f64,
    /// Coefficient of `q_n`.
    pub ag0: f64,
    ag1: f64,
    ag2: f64,
}

impl Default for Integrator {
    fn default() -> Self {
        Self {
            method: Method::Trapezoidal,
            order: 1,
            h: 1.0,
            h_prev: 1.0,
            ag0: 0.0,
            ag1: 0.0,
            ag2: 0.0,
        }
    }
}

impl Integrator {
    /// Coefficients for step `h` (previous step `h_prev`) at `order`.
    pub fn new(method: Method, order: u8, h: f64, h_prev: f64) -> Self {
        let (ag0, ag1, ag2) = match (order, method) {
            (1, _) => (1.0 / h, -1.0 / h, 0.0),
            (_, Method::Trapezoidal) => (2.0 / h, -2.0 / h, 0.0),
            (_, Method::Gear2) => {
                let r = h / h_prev;
                (
                    (1.0 + 2.0 * r) / ((1.0 + r) * h),
                    -(1.0 + r) / h,
                    r * r / ((1.0 + r) * h),
                )
            }
        };
        Self {
            method,
            order,
            h,
            h_prev,
            ag0,
            ag1,
            ag2,
        }
    }

    /// Pseudo-integrator for small-signal linearisation: every capacitor
    /// stamps `ag0 * C`, so loading with `ag0 = 0` and `ag0 = 1` separates
    /// the conductance and capacitance matrices.
    pub fn small_signal(ag0: f64) -> Self {
        Self {
            method: Method::Trapezoidal,
            order: 1,
            h: f64::INFINITY,
            h_prev: f64::INFINITY,
            ag0,
            ag1: 0.0,
            ag2: 0.0,
        }
    }

    /// Derivative of state `k` at the current point.
    pub fn derivative(&self, s: &States, k: usize) -> f64 {
        match (self.order, self.method) {
            (1, _) | (_, Method::Gear2) => self.ag0 * s.now[k] + self.ag1 * s.prev[k] + self.ag2 * s.prev2[k],
            (_, Method::Trapezoidal) => self.ag0 * s.now[k] + self.ag1 * s.prev[k] - s.prev[k + 1],
        }
    }

    /// Local truncation error constant of the corrector (`C h^(p+1) x^(p+1)`).
    pub fn error_constant(&self) -> f64 {
        match (self.order, self.method) {
            (1, _) => 0.5,
            (_, Method::Trapezoidal) => 1.0 / 12.0,
            (_, Method::Gear2) => 2.0 / 9.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// q(t) = t^2: every second-order method differentiates it exactly.
    #[test]
    fn second_order_methods_are_exact_on_quadratics() {
        let (t0, t1, t2) = (0.0f64, 0.3f64, 0.8f64);
        let q = |t: f64| t * t;
        let mut s = States::new(2);
        s.prev2[0] = q(t0);
        s.prev[0] = q(t1);
        s.prev[1] = 2.0 * t1; // exact derivative history for trapezoidal
        s.now[0] = q(t2);
        let gear = Integrator::new(Method::Gear2, 2, t2 - t1, t1 - t0);
        assert!((gear.derivative(&s, 0) - 2.0 * t2).abs() < 1e-12);
        let trap = Integrator::new(Method::Trapezoidal, 2, t2 - t1, t1 - t0);
        assert!((trap.derivative(&s, 0) - 2.0 * t2).abs() < 1e-12);
        let be = Integrator::new(Method::Trapezoidal, 1, t2 - t1, t1 - t0);
        assert!((be.derivative(&s, 0) - (q(t2) - q(t1)) / (t2 - t1)).abs() < 1e-12);
    }
}

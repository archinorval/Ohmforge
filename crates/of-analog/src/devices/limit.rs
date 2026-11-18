//! Newton-Raphson voltage limiting and junction helpers, following the
//! published SPICE3 algorithms (Nagel, 1975; Quarles, 1989).

/// Boltzmann constant (J/K), as used by ngspice.
pub const BOLTZMANN: f64 = 1.380_648_52e-23;
/// Elementary charge (C), as used by ngspice.
pub const CHARGE: f64 = 1.602_176_620_8e-19;
/// Nominal temperature: 27 °C.
pub const TNOM: f64 = 300.15;

/// Thermal voltage kT/q at the nominal temperature (about 25.86 mV).
pub fn thermal_voltage() -> f64 {
    BOLTZMANN * TNOM / CHARGE
}

/// Critical voltage of a junction: where the exponential's curvature makes
/// limiting worthwhile.
pub fn vcrit(vt: f64, is: f64) -> f64 {
    vt * (vt / (std::f64::consts::SQRT_2 * is)).ln()
}

/// Limit a pn-junction voltage step. Returns the limited voltage and
/// whether limiting happened.
pub fn pnjlim(vnew: f64, vold: f64, vt: f64, vcrit: f64) -> (f64, bool) {
    if vnew > vcrit && (vnew - vold).abs() > vt + vt {
        let v = if vold > 0.0 {
            let arg = 1.0 + (vnew - vold) / vt;
            if arg > 0.0 { vold + vt * arg.ln() } else { vcrit }
        } else {
            vt * (vnew / vt).ln()
        };
        (v, true)
    } else {
        (vnew, false)
    }
}

/// Limit a FET gate-source step around the threshold `vto`.
pub fn fetlim(vnew: f64, vold: f64, vto: f64) -> f64 {
    let vtsthi = (2.0 * (vold - vto)).abs() + 2.0;
    let vtstlo = vtsthi / 2.0 + 2.0;
    let vtox = vto + 3.5;
    let delv = vnew - vold;
    let mut v = vnew;
    if vold >= vto {
        if vold >= vtox {
            if delv <= 0.0 {
                if vnew >= vtox {
                    if -delv > vtstlo {
                        v = vold - vtstlo;
                    }
                } else {
                    v = v.max(vto + 2.0);
                }
            } else if delv >= vtsthi {
                v = vold + vtsthi;
            }
        } else if delv <= 0.0 {
            v = v.max(vto - 0.5);
        } else {
            v = v.min(vto + 4.0);
        }
    } else if delv <= 0.0 {
        if -delv > vtsthi {
            v = vold - vtsthi;
        }
    } else {
        let vtemp = vto + 0.5;
        if vnew <= vtemp {
            if delv > vtstlo {
                v = vold + vtstlo;
            }
        } else {
            v = vtemp;
        }
    }
    v
}

/// Limit a FET drain-source step.
pub fn limvds(vnew: f64, vold: f64) -> f64 {
    if vold >= 3.5 {
        if vnew > vold {
            vnew.min(3.0 * vold + 2.0)
        } else if vnew < 3.5 {
            vnew.max(2.0)
        } else {
            vnew
        }
    } else if vnew > vold {
        vnew.min(4.0)
    } else {
        vnew.max(-0.5)
    }
}

/// Diode-like exponential current with SPICE's cubic reverse region:
/// returns `(i, g)` for saturation current `is` and emission voltage `vte`.
pub fn junction(v: f64, is: f64, vte: f64) -> (f64, f64) {
    if v >= -3.0 * vte {
        let e = (v / vte).min(80.0).exp();
        (is * (e - 1.0), is * e / vte)
    } else {
        let arg = 3.0 * vte / (v * std::f64::consts::E);
        let arg = arg * arg * arg;
        (-is * (1.0 + arg), is * 3.0 * arg / v)
    }
}

/// Depletion charge and capacitance with the forward-bias linearisation
/// above `fc * vj`. Returns `(q, c)`.
pub fn depletion(v: f64, cj0: f64, vj: f64, m: f64, fc: f64) -> (f64, f64) {
    if cj0 == 0.0 {
        return (0.0, 0.0);
    }
    if v < fc * vj {
        let arg = 1.0 - v / vj;
        let sarg = arg.powf(-m);
        let q = vj * cj0 * (1.0 - arg * sarg) / (1.0 - m);
        (q, cj0 * sarg)
    } else {
        let f1 = vj * (1.0 - (1.0 - fc).powf(1.0 - m)) / (1.0 - m);
        let f2 = (1.0 - fc).powf(1.0 + m);
        let f3 = 1.0 - fc * (1.0 + m);
        let czof2 = cj0 / f2;
        let q = cj0 * f1 + czof2 * (f3 * (v - fc * vj) + (m / (2.0 * vj)) * (v * v - fc * vj * fc * vj));
        (q, czof2 * (f3 + m * v / vj))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thermal_voltage_at_27c() {
        assert!((thermal_voltage() - 0.025_864_917).abs() < 1e-8);
    }

    #[test]
    fn pnjlim_compresses_large_forward_steps() {
        let vt = 0.025;
        let vc = vcrit(vt, 1e-14);
        let (v, lim) = pnjlim(5.0, 0.6, vt, vc);
        assert!(lim && v > 0.6 && v < 0.8, "{v}");
        assert_eq!(pnjlim(0.61, 0.6, vt, vc), (0.61, false));
        assert_eq!(pnjlim(-5.0, 0.6, vt, vc), (-5.0, false));
    }

    #[test]
    fn fet_limits() {
        assert_eq!(limvds(10.0, 0.0), 4.0);
        assert_eq!(limvds(-10.0, 0.0), -0.5);
        assert_eq!(limvds(100.0, 5.0), 17.0);
        assert!(fetlim(10.0, 0.0, 1.0) <= 1.5);
    }

    #[test]
    fn depletion_charge_is_integral_of_capacitance() {
        let (cj0, vj, m, fc) = (1e-12, 0.8, 0.5, 0.5);
        // Numerically differentiate q across the fc*vj boundary.
        for v in [-3.0, -0.5, 0.0, 0.39, 0.41, 0.7] {
            let dv = 1e-6;
            let (q1, _) = depletion(v - dv, cj0, vj, m, fc);
            let (q2, _) = depletion(v + dv, cj0, vj, m, fc);
            let (_, c) = depletion(v, cj0, vj, m, fc);
            assert!(((q2 - q1) / (2.0 * dv) - c).abs() < 1e-6 * c, "v={v}");
        }
        // Continuous at the boundary.
        let b = fc * vj;
        let (qa, ca) = depletion(b - 1e-12, cj0, vj, m, fc);
        let (qb, cb) = depletion(b + 1e-12, cj0, vj, m, fc);
        assert!((qa - qb).abs() < 1e-20 && (ca - cb).abs() < 1e-18);
    }
}

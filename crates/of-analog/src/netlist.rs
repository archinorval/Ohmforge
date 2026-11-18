//! Build a [`Circuit`] from a parsed SPICE [`Deck`].

use of_netlist::spice::{Analysis, Deck, Element};

use crate::analysis::{Options, TranRequest};
use crate::circuit::{Circuit, CircuitError};
use crate::devices::{
    Bjt, BjtModel, Capacitor, Control, ControlledSource, CurrentSource, Diode, DiodeModel, Inductor, MosGeometry,
    MosModel, Mosfet, MutualInductance, Resistor, SourceValue, Switch, SwitchControl, SwitchModel, VoltageSource,
};
use crate::waveform::Wave;

/// A circuit plus the analyses and options requested by the deck.
#[derive(Debug)]
pub struct Built {
    /// The circuit.
    pub circuit: Circuit,
    /// Options from `.options`.
    pub options: Options,
    /// Analyses in deck order.
    pub analyses: Vec<Analysis>,
    /// Non-fatal notes (ignored parameters and cards).
    pub warnings: Vec<String>,
    /// `.ic` node values.
    pub ic: std::collections::BTreeMap<String, f64>,
    /// Circuit temperature (K) from `.temp` or `.options temp`.
    pub temp: f64,
}

impl Built {
    /// The transient request for a `.tran` analysis.
    pub fn tran_request(&self, a: &Analysis) -> Option<TranRequest> {
        match *a {
            Analysis::Tran {
                tstep,
                tstop,
                tstart,
                tmax,
                uic,
            } => Some(TranRequest {
                tstep,
                tstop,
                tstart,
                tmax,
                uic,
                ic: self.ic.clone(),
            }),
            _ => None,
        }
    }

    /// Frequencies of an `.ac` analysis.
    pub fn ac_frequencies(a: &Analysis) -> Option<Vec<f64>> {
        match a {
            Analysis::Ac {
                sweep,
                points,
                fstart,
                fstop,
            } => Some(sweep.frequencies(*points, *fstart, *fstop)),
            _ => None,
        }
    }
}

/// Errors building a circuit from a deck.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BuildError {
    /// An element refers to a model that is missing or of the wrong type.
    #[error("line {line}: {element}: {message}")]
    Element {
        /// Netlist line.
        line: usize,
        /// Element name.
        element: String,
        /// Problem.
        message: String,
    },
    /// Circuit-level problem.
    #[error(transparent)]
    Circuit(#[from] CircuitError),
}

fn bad(e: &Element, message: impl Into<String>) -> BuildError {
    BuildError::Element {
        line: e.line,
        element: e.name.clone(),
        message: message.into(),
    }
}

/// Kelvin offset of the Celsius scale.
const KELVIN: f64 = 273.15;

/// Convert a deck into a circuit.
pub fn build(deck: &Deck) -> Result<Built, BuildError> {
    let mut c = Circuit::new();
    let mut warnings = deck.warnings.clone();
    let mut options = Options::default();
    options.apply(&deck.options);
    let temp_c = deck.temp.or_else(|| deck.options.get("temp").copied()).unwrap_or(27.0);
    let tnom_c = deck.options.get("tnom").copied().unwrap_or(27.0);
    let circuit_temp = temp_c + KELVIN;
    // Model parameters with the circuit TNOM filled in.
    let with_tnom = |m: &of_netlist::spice::Model| {
        let mut p = m.params.clone();
        p.entry("tnom".into()).or_insert(tnom_c);
        p
    };
    for e in &deck.elements {
        let nodes: Vec<usize> = e.nodes.iter().map(|n| c.node(n)).collect();
        let value = || e.value.ok_or_else(|| bad(e, "missing value"));
        let model = |kinds: &[&str]| -> Result<&of_netlist::spice::Model, BuildError> {
            let name = e.model.as_deref().ok_or_else(|| bad(e, "missing model name"))?;
            let m = deck
                .models
                .get(name)
                .ok_or_else(|| bad(e, format!("unknown model '{name}'")))?;
            if !kinds.contains(&m.kind.as_str()) {
                return Err(bad(
                    e,
                    format!("model '{name}' is a {} model, expected {}", m.kind, kinds.join("/")),
                ));
            }
            Ok(m)
        };
        let off = e.flags.iter().any(|f| f == "off");
        // Instance temperature: `temp=` (°C) or circuit temperature + `dtemp=`.
        let inst_temp =
            e.params.get("temp").map_or(circuit_temp, |t| t + KELVIN) + e.params.get("dtemp").copied().unwrap_or(0.0);
        // Linear temperature coefficients of passives, relative to TNOM.
        let tc_factor = || {
            let dt = inst_temp - (tnom_c + KELVIN);
            let tc1 = e.params.get("tc1").copied().unwrap_or(0.0);
            let tc2 = e.params.get("tc2").copied().unwrap_or(0.0);
            1.0 + tc1 * dt + tc2 * dt * dt
        };
        let tol = e.params.get("tol").copied();
        match e.letter() {
            'r' => {
                let r = value()? * tc_factor();
                if r == 0.0 {
                    return Err(bad(e, "zero resistance (use a voltage source for a short)"));
                }
                c.add(Resistor::new(&e.name, nodes[0], nodes[1], r).with_tolerance(tol))?;
            }
            'c' => {
                let esr = e.params.get("esr").copied().unwrap_or(0.0);
                let esl = e.params.get("esl").copied().unwrap_or(0.0);
                c.add(
                    Capacitor::new(
                        &e.name,
                        nodes[0],
                        nodes[1],
                        value()? * tc_factor(),
                        e.params.get("ic").copied(),
                    )
                    .with_parasitics(esr, esl)
                    .with_tolerance(tol),
                )?
            }
            'l' => {
                let rser = e
                    .params
                    .get("rser")
                    .or_else(|| e.params.get("esr"))
                    .copied()
                    .unwrap_or(0.0);
                c.add(
                    Inductor::new(
                        &e.name,
                        nodes[0],
                        nodes[1],
                        value()? * tc_factor(),
                        e.params.get("ic").copied(),
                    )
                    .with_series_resistance(rser)
                    .with_tolerance(tol),
                )?
            }
            'k' => {
                let k = value()?;
                if k.is_nan() || k.abs() > 1.0 {
                    return Err(bad(e, format!("coupling coefficient {k} must be within -1..1")));
                }
                let mut ls = Vec::new();
                for r in &e.refs {
                    let l = deck
                        .elements
                        .iter()
                        .find(|o| o.name == *r && o.letter() == 'l')
                        .ok_or_else(|| bad(e, format!("'{r}' is not an inductor")))?;
                    ls.push(l.value.unwrap_or(0.0));
                }
                let m = k * (ls[0] * ls[1]).sqrt();
                c.add(MutualInductance::new(&e.name, &e.refs[0], &e.refs[1], m))?;
            }
            'v' | 'i' => {
                let spec = e.source.clone().unwrap_or_default();
                let wave = spec
                    .wave
                    .as_ref()
                    .map(Wave::from_spec)
                    .transpose()
                    .map_err(|m| bad(e, m))?;
                let sv = SourceValue {
                    dc: spec.dc.or(if wave.is_none() { Some(0.0) } else { None }),
                    wave,
                };
                let ac = spec.ac.map(|m| (m, spec.ac_phase));
                if e.letter() == 'v' {
                    c.add(VoltageSource::new(&e.name, nodes[0], nodes[1], sv).with_ac(ac))?;
                } else {
                    c.add(CurrentSource::new(&e.name, nodes[0], nodes[1], sv).with_ac(ac))?;
                }
            }
            'e' => c.add(ControlledSource::new(
                &e.name,
                nodes[0],
                nodes[1],
                value()?,
                Control::Vcvs {
                    cp: nodes[2],
                    cn: nodes[3],
                },
            ))?,
            'g' => c.add(ControlledSource::new(
                &e.name,
                nodes[0],
                nodes[1],
                value()?,
                Control::Vccs {
                    cp: nodes[2],
                    cn: nodes[3],
                },
            ))?,
            'f' | 'h' | 'w' => {
                let src = e.control.clone().ok_or_else(|| bad(e, "missing controlling source"))?;
                if !deck
                    .elements
                    .iter()
                    .any(|o| o.name == src && matches!(o.letter(), 'v' | 'l' | 'e' | 'h'))
                {
                    return Err(CircuitError::UnknownControl(e.name.clone(), src).into());
                }
                match e.letter() {
                    'w' => {
                        let m = model(&["csw"])?;
                        let (sm, unknown) = SwitchModel::from_params(true, &m.params);
                        note_unknown(&mut warnings, e, unknown);
                        let on = e.flags.iter().any(|f| f == "on");
                        let ctl = SwitchControl::Current { source: src };
                        c.add(Switch::new(&e.name, nodes[0], nodes[1], ctl, sm, on))?;
                    }
                    'f' => c.add(ControlledSource::new(
                        &e.name,
                        nodes[0],
                        nodes[1],
                        value()?,
                        Control::Cccs { source: src },
                    ))?,
                    _ => c.add(ControlledSource::new(
                        &e.name,
                        nodes[0],
                        nodes[1],
                        value()?,
                        Control::Ccvs { source: src },
                    ))?,
                }
            }
            's' => {
                let m = model(&["sw"])?;
                let (sm, unknown) = SwitchModel::from_params(false, &m.params);
                note_unknown(&mut warnings, e, unknown);
                let on = e.flags.iter().any(|f| f == "on");
                let ctl = SwitchControl::Voltage {
                    cp: nodes[2],
                    cn: nodes[3],
                };
                c.add(Switch::new(&e.name, nodes[0], nodes[1], ctl, sm, on))?;
            }
            'd' => {
                let m = model(&["d", "led"])?;
                let p = with_tnom(m);
                let (dm, unknown) = if m.kind == "led" {
                    DiodeModel::led_from_params(&p)
                } else {
                    DiodeModel::from_params(&p)
                };
                note_unknown(&mut warnings, e, unknown);
                let area = e.params.get("area").copied().unwrap_or(1.0);
                c.add(Diode::new(&e.name, nodes[0], nodes[1], dm, area, off).with_temp(inst_temp))?;
            }
            'q' => {
                let m = model(&["npn", "pnp"])?;
                let pol = if m.kind == "npn" { 1.0 } else { -1.0 };
                let (bm, unknown) = BjtModel::from_params(pol, &with_tnom(m));
                note_unknown(&mut warnings, e, unknown);
                let area = e.params.get("area").copied().unwrap_or(1.0);
                let mut q = Bjt::new(&e.name, nodes[0], nodes[1], nodes[2], bm, area, off).with_temp(inst_temp);
                if let Some(&sub) = nodes.get(3) {
                    q = q.with_substrate(sub);
                }
                c.add(q)?;
            }
            'm' => {
                let m = model(&["nmos", "pmos"])?;
                let level = m.params.get("level").copied().unwrap_or(1.0);
                if level != 1.0 && level != 3.0 {
                    return Err(bad(
                        e,
                        format!("MOSFET level {level} is not supported yet (levels 1 and 3)"),
                    ));
                }
                let pol = if m.kind == "nmos" { 1.0 } else { -1.0 };
                let (mm, unknown) = MosModel::from_params(pol, &with_tnom(m));
                note_unknown(&mut warnings, e, unknown);
                let g = |k: &str| e.params.get(k).copied().unwrap_or(0.0);
                let w = e.params.get("w").copied().unwrap_or(100e-6);
                let l = e.params.get("l").copied().unwrap_or(100e-6);
                let mult = e.params.get("m").copied().unwrap_or(1.0);
                let geom = MosGeometry {
                    ad: g("ad"),
                    as_: g("as"),
                    pd: g("pd"),
                    ps: g("ps"),
                };
                c.add(
                    Mosfet::new(&e.name, nodes[0], nodes[1], nodes[2], nodes[3], mm, w, l, mult, off)
                        .with_geometry(geom)
                        .with_temp(inst_temp),
                )?;
            }
            other => return Err(bad(e, format!("element type '{other}' is not supported"))),
        }
    }
    c.finalize()?;
    Ok(Built {
        circuit: c,
        options,
        analyses: deck.analyses.clone(),
        warnings,
        ic: deck.initial_conditions.clone(),
        temp: circuit_temp,
    })
}

fn note_unknown(warnings: &mut Vec<String>, e: &Element, unknown: Vec<String>) {
    if !unknown.is_empty() {
        warnings.push(format!(
            "line {}: {}: model parameters not supported, ignored: {}",
            e.line,
            e.name,
            unknown.join(", ")
        ));
    }
}

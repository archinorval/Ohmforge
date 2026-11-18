//! Build a [`Circuit`] from a parsed SPICE [`Deck`].

use of_netlist::spice::{Analysis, Deck, Element};

use crate::analysis::{Options, TranRequest};
use crate::circuit::{Circuit, CircuitError};
use crate::devices::{
    Bjt, BjtModel, Capacitor, Control, ControlledSource, CurrentSource, Diode, DiodeModel, Inductor, MosModel, Mosfet,
    Resistor, SourceValue, VoltageSource,
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

/// Convert a deck into a circuit.
pub fn build(deck: &Deck) -> Result<Built, BuildError> {
    let mut c = Circuit::new();
    let mut warnings = deck.warnings.clone();
    let mut options = Options::default();
    options.apply(&deck.options);
    for e in &deck.elements {
        let nodes: Vec<usize> = match e.letter() {
            // The BJT substrate node (4th) has no element in this subset.
            'q' => e.nodes.iter().take(3).map(|n| c.node(n)).collect(),
            _ => e.nodes.iter().map(|n| c.node(n)).collect(),
        };
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
        match e.letter() {
            'r' => {
                let r = value()?;
                if r == 0.0 {
                    return Err(bad(e, "zero resistance (use a voltage source for a short)"));
                }
                c.add(Resistor::new(&e.name, nodes[0], nodes[1], r))?;
            }
            'c' => c.add(Capacitor::new(
                &e.name,
                nodes[0],
                nodes[1],
                value()?,
                e.params.get("ic").copied(),
            ))?,
            'l' => c.add(Inductor::new(
                &e.name,
                nodes[0],
                nodes[1],
                value()?,
                e.params.get("ic").copied(),
            ))?,
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
                if e.letter() == 'v' {
                    c.add(VoltageSource::new(&e.name, nodes[0], nodes[1], sv))?;
                } else {
                    c.add(CurrentSource::new(&e.name, nodes[0], nodes[1], sv))?;
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
            'f' | 'h' => {
                let src = e.control.clone().ok_or_else(|| bad(e, "missing controlling source"))?;
                if !deck
                    .elements
                    .iter()
                    .any(|o| o.name == src && matches!(o.letter(), 'v' | 'l' | 'e' | 'h'))
                {
                    return Err(CircuitError::UnknownControl(e.name.clone(), src).into());
                }
                let ctl = if e.letter() == 'f' {
                    Control::Cccs { source: src }
                } else {
                    Control::Ccvs { source: src }
                };
                c.add(ControlledSource::new(&e.name, nodes[0], nodes[1], value()?, ctl))?;
            }
            'd' => {
                let m = model(&["d"])?;
                let (dm, unknown) = DiodeModel::from_params(&m.params);
                note_unknown(&mut warnings, e, unknown);
                let area = e.params.get("area").copied().unwrap_or(1.0);
                c.add(Diode::new(&e.name, nodes[0], nodes[1], dm, area, off))?;
            }
            'q' => {
                let m = model(&["npn", "pnp"])?;
                let pol = if m.kind == "npn" { 1.0 } else { -1.0 };
                let (bm, unknown) = BjtModel::from_params(pol, &m.params);
                note_unknown(&mut warnings, e, unknown);
                if e.nodes.len() > 3 {
                    warnings.push(format!(
                        "line {}: {}: substrate node ignored (no CJS in this model subset)",
                        e.line, e.name
                    ));
                }
                let area = e.params.get("area").copied().unwrap_or(1.0);
                c.add(Bjt::new(&e.name, nodes[0], nodes[1], nodes[2], bm, area, off))?;
            }
            'm' => {
                let m = model(&["nmos", "pmos"])?;
                let level = m.params.get("level").copied().unwrap_or(1.0);
                if level != 1.0 {
                    return Err(bad(
                        e,
                        format!("MOSFET level {level} is not supported yet (level 1 only)"),
                    ));
                }
                let pol = if m.kind == "nmos" { 1.0 } else { -1.0 };
                let (mm, unknown) = MosModel::from_params(pol, &m.params);
                note_unknown(&mut warnings, e, unknown);
                if m.params.contains_key("tox") {
                    warnings.push(format!(
                        "line {}: {}: TOX given, but Meyer gate capacitances are not modelled yet",
                        e.line, e.name
                    ));
                }
                let w = e.params.get("w").copied().unwrap_or(100e-6);
                let l = e.params.get("l").copied().unwrap_or(100e-6);
                let mult = e.params.get("m").copied().unwrap_or(1.0);
                c.add(Mosfet::new(
                    &e.name, nodes[0], nodes[1], nodes[2], nodes[3], mm, w, l, mult, off,
                ))?;
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

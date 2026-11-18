//! AMS1117-3.3 macromodel, placed through the parts bridge, against the
//! datasheet's electrical characteristics (AMS1117 datasheet, 25 °C).
//! Electrical tests use `rja` ≈ 0 to stand for the datasheet's pulsed
//! (isothermal) measurements; the thermal test uses the part's 90 K/W.

mod common;

use of_analog::netlist::{BuildEnv, add_deck};
use of_analog::{Circuit, Options, analysis};
use of_components::analog::{Board, Placement, build};

/// The regulator from in→out with its stimulus deck (which must drive
/// node `in` and load node `out`) added.
fn reg(stimulus: &str, rja: f64) -> Circuit {
    let lib = common::library();
    let board = Board {
        placements: vec![
            Placement::new("U1", "ams.ams1117-3.3", &[("VIN", "in"), ("VOUT", "out"), ("GND", "0")]).with("rja", rja),
        ],
        ..Board::default()
    };
    let mut b = build(&lib, &board).unwrap();
    let deck = of_netlist::spice::parse_str(&format!("stimulus\n{stimulus}\n.end\n")).unwrap();
    add_deck(&mut b.circuit, &deck, &BuildEnv::default(), &mut Vec::new()).unwrap();
    b.circuit
}

fn op(stimulus: &str, sig: &str) -> f64 {
    let mut c = reg(stimulus, 1e-3);
    analysis::operating_point(&mut c, &Options::default())
        .unwrap()
        .get(sig)
        .unwrap()
}

#[test]
fn output_voltage_within_the_datasheet_band() {
    // 3.267..3.333 V at 10 mA, 5 V in; 3.235..3.365 V over line and load.
    let v = op("VIN in 0 5\nIL out 0 10m", "v(out)");
    assert!((3.267..=3.333).contains(&v), "{v}");
    for (vin, il) in [(4.75, 0.0), (4.75, 0.8), (12.0, 0.0), (12.0, 0.1)] {
        let v = op(&format!("VIN in 0 {vin}\nIL out 0 {il}"), "v(out)");
        assert!((3.235..=3.365).contains(&v), "{vin} V, {il} A: {v}");
    }
}

#[test]
fn line_and_load_regulation() {
    // Line: 4.75..12 V at 0 A, datasheet typ 1 mV / max 10 mV.
    let line = (op("VIN in 0 12", "v(out)") - op("VIN in 0 4.75", "v(out)")).abs();
    assert!(line < 10e-3 && line > 0.1e-3, "line {line}");
    // Load: 0..0.8 A at 5 V in (Vin − Vout = 1.7 V), typ 3 mV / max 15 mV.
    let load = op("VIN in 0 5", "v(out)") - op("VIN in 0 5\nIL out 0 0.8", "v(out)");
    assert!(load > 0.5e-3 && load < 15e-3, "load {load}");
}

#[test]
fn quiescent_current() {
    // Datasheet typ 5 mA, max 11 mA, for Vin up to 15 V.
    for vin in [5.0, 15.0] {
        let iq = -op(&format!("VIN in 0 {vin}"), "i(vin)");
        assert!((3e-3..11e-3).contains(&iq), "{vin} V: {iq}");
    }
}

#[test]
fn dropout_voltage() {
    // Dropout (ΔVout = 1 %) at 800 mA: typ 1.1 V, max 1.3 V.
    let mut c = reg("VIN in 0 5\nIL out 0 0.8", 1e-3);
    let s = analysis::dc_sweep(&mut c, &Options::default(), "vin", 6.0, 3.5, -0.005).unwrap();
    let vout = s.signal("v(out)").unwrap();
    let nominal = vout[0];
    let k = vout.iter().position(|&v| v < 0.99 * nominal).unwrap();
    let dropout = s.x[k] - vout[k];
    assert!((0.9..1.3).contains(&dropout), "dropout {dropout}");
}

#[test]
fn current_limit() {
    // Short-ish load at Vin − Vout = 1.5 V: limit min 0.9 A (typ 1.1 A),
    // and it must actually limit.
    let i = op("VIN in 0 4.8\nVO out 0 3.0", "i(vo)");
    assert!((0.9..1.5).contains(&i), "limit {i}");
    let short = op("VIN in 0 4.8\nVO out 0 0", "i(vo)");
    assert!(short < 1.5 && short > 0.5, "short-circuit {short}");
}

#[test]
fn ripple_rejection() {
    // 120 Hz, Vin = 4.8 V + ripple, Iout = 0.5 A: min 60 dB, typ 75 dB.
    let mut c = reg("VIN in 0 DC 4.8 AC 1\nIL out 0 0.5", 1e-3);
    let ac = analysis::ac_analysis(&mut c, &Options::default(), &[120.0]).unwrap();
    let rr = -ac.db("v(out)").unwrap()[0];
    assert!((60.0..90.0).contains(&rr), "ripple rejection {rr} dB");
}

#[test]
fn thermal_rise_and_shutdown() {
    // 7 V in, 100 mA: 0.37 W → about 33 K over ambient at 90 K/W.
    let mut c = reg("VIN in 0 7\nRL out 0 33", 90.0);
    let o = analysis::operating_point(&mut c, &Options::default()).unwrap();
    let p = (7.0 - o.get("v(out)").unwrap()) * o.get("v(out)").unwrap() / 33.0;
    // v(tj) is the rise over the 25 °C ambient.
    let tj = 25.0 + o.get("v(xu1.tj)").unwrap();
    assert!((tj - (25.0 + 90.0 * p)).abs() < 1.0, "tj {tj} for {p} W");
    // 12 V into 3.3 Ω would be ~8.7 W: the thermal limit holds Tj near the
    // 150 °C shutdown by folding the output back.
    let mut c = reg("VIN in 0 12\nRL out 0 3.3", 90.0);
    let o = analysis::operating_point(&mut c, &Options::default()).unwrap();
    let tj = 25.0 + o.get("v(xu1.tj)").unwrap();
    assert!((145.0..165.0).contains(&tj), "tj {tj}");
    assert!(o.get("v(out)").unwrap() < 3.0, "output folds back");
}

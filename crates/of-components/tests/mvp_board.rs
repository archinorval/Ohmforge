//! MVP analog check through the parts bridge: the board of
//! `of-analog/tests/circuits/mvp_board.cir` (USB 5 V ramp, AMS1117-3.3,
//! LED with series resistor, NPN LED switch driven by a GPIO-like pulse),
//! placed from the parts library, must match the ngspice reference within
//! 1 % of each signal's span.

mod common;

use of_analog::netlist::{BuildEnv, add_deck};
use of_analog::results::interpolate;
use of_analog::{Options, analysis};
use of_components::analog::{Board, Placement, build};

fn mvp_board() -> Board {
    let r = |refdes: &str, a: &str, b: &str, ohms: f64| {
        Placement::new(refdes, "generic.resistor", &[("1", a), ("2", b)]).with("resistance", ohms)
    };
    Board {
        placements: vec![
            r("RUSB", "vbus", "usb", 0.2),
            Placement::new("CIN", "generic.capacitor-mlcc", &[("1", "usb"), ("2", "0")]).with("capacitance", 10e-6),
            Placement::new(
                "U1",
                "ams.ams1117-3.3",
                &[("VIN", "usb"), ("VOUT", "vdd"), ("GND", "0")],
            ),
            Placement::new("COUT", "generic.capacitor-tantalum", &[("+", "vdd"), ("-", "0")]),
            r("R1", "vdd", "led1a", 330.0),
            Placement::new("D1", "generic.led-red-0603", &[("A", "led1a"), ("K", "0")]),
            r("RG", "g", "gp", 25.0),
            r("RB", "gp", "b", 1e3),
            Placement::new("Q1", "generic.npn-small-signal", &[("C", "sw"), ("B", "b"), ("E", "0")]),
            r("R2", "vdd", "led2a", 150.0),
            Placement::new("D2", "generic.led-red-0603", &[("A", "led2a"), ("K", "sw")]),
        ],
        ..Board::default()
    }
}

#[test]
fn mvp_board_from_parts_matches_ngspice() {
    let lib = common::library();
    let mut b = build(&lib, &mvp_board()).unwrap();
    assert!(b.warnings.is_empty(), "{:?}", b.warnings);
    // Stimulus: the USB supply and the GPIO.
    let stim = of_netlist::spice::parse_str(
        "stimulus\nVUSB vbus 0 PWL(0 0 1m 5)\nVGPIO g 0 PULSE(0 3.3 2m 10u 10u 1m 2m)\n.end\n",
    )
    .unwrap();
    let mut warnings = Vec::new();
    add_deck(&mut b.circuit, &stim, &BuildEnv::default(), &mut warnings).unwrap();
    let opts = Options {
        trtol: 1.0,
        reltol: 1e-4,
        ..Options::default()
    };
    let req = analysis::TranRequest {
        tstep: 5e-6,
        tstop: 6e-3,
        tmax: 2e-6,
        ..Default::default()
    };
    let w = analysis::transient(&mut b.circuit, &opts, &req).unwrap();
    let (names, rows) = common::golden("mvp_board");
    let t: Vec<f64> = rows.iter().map(|r| r[0]).collect();
    let mut report = Vec::new();
    for (j, sig) in names.iter().enumerate() {
        let reference: Vec<f64> = rows.iter().map(|r| r[j + 1]).collect();
        let span =
            reference.iter().copied().fold(f64::MIN, f64::max) - reference.iter().copied().fold(f64::MAX, f64::min);
        let mine = w.signal(sig).unwrap_or_else(|| panic!("no {sig}"));
        let worst = t
            .iter()
            .zip(&reference)
            .map(|(&ti, &r)| (interpolate(&w.x, mine, ti) - r).abs())
            .fold(0.0, f64::max);
        let pct = 100.0 * worst / span;
        println!("mvp_board (parts) {sig}: {pct:.4} % of span");
        report.push((sig.clone(), pct));
    }
    let bad: Vec<_> = report.iter().filter(|(_, p)| *p > 1.0).collect();
    assert!(bad.is_empty(), "{bad:?}");
}

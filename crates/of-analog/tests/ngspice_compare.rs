//! Compare of-analog against ngspice reference data.
//!
//! The circuits live in `tests/circuits/*.cir`; `tests/golden/*.csv` were
//! produced by running ngspice as an external program
//! (`scripts/gen-ngspice-golden.sh`; see `tests/golden/README.md`). Every
//! compared signal must agree within 1 % of the reference signal's span
//! (or of its magnitude for operating points). AC magnitudes (`vm(...)`)
//! are compared point by point relative to the reference magnitude (with
//! a floor of 1e-3 of the largest), and AC phases (`vp(...)`, degrees) as
//! a percentage of 180°. Nested DC sweeps are compared row by row.
//!
//! Run with `cargo test -p of-analog --test ngspice_compare -- --nocapture`
//! to print the comparison table.

use std::path::Path;

use of_analog::{analysis, netlist, results::interpolate};
use of_netlist::spice::{Analysis, parse_file};

const LIMIT_PERCENT: f64 = 1.0;

struct Golden {
    names: Vec<String>,
    rows: Vec<Vec<f64>>,
}

fn read_golden(path: &Path) -> Golden {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let mut names = Vec::new();
    let mut rows = Vec::new();
    for line in text.lines() {
        if let Some(cols) = line.strip_prefix("# columns: ") {
            names = cols.split_whitespace().skip(1).map(str::to_owned).collect();
        } else if !line.starts_with('#') && !line.trim().is_empty() {
            rows.push(line.split(',').map(|v| v.trim().parse().unwrap()).collect());
        }
    }
    Golden { names, rows }
}

/// Simulate one circuit; returns (circuit, signal, max error %) rows.
fn compare(name: &str) -> Vec<(String, String, f64)> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    let deck = parse_file(&dir.join("circuits").join(format!("{name}.cir"))).expect("parse");
    let golden = read_golden(&dir.join("golden").join(format!("{name}.csv")));
    let mut b = netlist::build(&deck).expect("build");
    let mut out = Vec::new();
    match b.analyses[0].clone() {
        Analysis::Op => {
            let op = analysis::operating_point(&mut b.circuit, &b.options).expect("op");
            for (j, sig) in golden.names.iter().enumerate() {
                let reference = golden.rows[0][j + 1];
                let mine = op.get(sig).unwrap_or_else(|| panic!("{name}: no {sig}"));
                let err = 100.0 * (mine - reference).abs() / reference.abs().max(1e-12);
                out.push((name.to_owned(), format!("{sig} (op)"), err));
            }
        }
        a @ Analysis::Ac { .. } => {
            let freqs = netlist::Built::ac_frequencies(&a).unwrap();
            let ac = analysis::ac_analysis(&mut b.circuit, &b.options, &freqs).expect("ac");
            assert_eq!(ac.freq.len(), golden.rows.len(), "{name}: frequency count");
            for (j, sig) in golden.names.iter().enumerate() {
                let refs: Vec<f64> = golden.rows.iter().map(|r| r[j + 1]).collect();
                let inner = &format!("v({})", &sig[3..sig.len() - 1]);
                let err = if sig.starts_with("vm(") {
                    let mine = ac.mag(inner).unwrap_or_else(|| panic!("{name}: no {inner}"));
                    let floor = refs.iter().copied().fold(0.0, f64::max) * 1e-3;
                    mine.iter()
                        .zip(&refs)
                        .map(|(m, r)| 100.0 * (m - r).abs() / r.abs().max(floor).max(1e-30))
                        .fold(0.0, f64::max)
                } else {
                    let mine = ac.phase_deg(inner).unwrap_or_else(|| panic!("{name}: no {inner}"));
                    mine.iter()
                        .zip(&refs)
                        .map(|(m, r)| {
                            let d = (m - r).rem_euclid(360.0);
                            100.0 * d.min(360.0 - d) / 180.0
                        })
                        .fold(0.0, f64::max)
                };
                out.push((name.to_owned(), format!("{sig} (ac)"), err));
            }
        }
        a => {
            let (w, nested) = match a {
                Analysis::Dc {
                    source,
                    start,
                    stop,
                    step,
                    outer,
                } => {
                    let inner = analysis::DcAxis {
                        source,
                        start,
                        stop,
                        step,
                    };
                    let outer = outer.map(|o| analysis::DcAxis {
                        source: o.source,
                        start: o.start,
                        stop: o.stop,
                        step: o.step,
                    });
                    let nested = outer.is_some();
                    (
                        analysis::dc_sweep_nested(&mut b.circuit, &b.options, &inner, outer.as_ref()).expect("dc"),
                        nested,
                    )
                }
                Analysis::Tran { .. } => {
                    let req = b.tran_request(&a).unwrap();
                    (
                        analysis::transient(&mut b.circuit, &b.options, &req).expect("tran"),
                        false,
                    )
                }
                Analysis::Op | Analysis::Ac { .. } => unreachable!(),
            };
            let kind = if w.x_name == "time" { "tran" } else { "dc" };
            if nested {
                assert_eq!(w.x.len(), golden.rows.len(), "{name}: nested sweep length");
            }
            for (j, sig) in golden.names.iter().enumerate() {
                let mine = w.signal(sig).unwrap_or_else(|| panic!("{name}: no {sig}"));
                let refs: Vec<f64> = golden.rows.iter().map(|r| r[j + 1]).collect();
                let span =
                    refs.iter().copied().fold(f64::MIN, f64::max) - refs.iter().copied().fold(f64::MAX, f64::min);
                let span = span
                    .max(refs.iter().map(|v| v.abs()).fold(0.0, f64::max) * 1e-3)
                    .max(1e-12);
                let err = golden
                    .rows
                    .iter()
                    .zip(&refs)
                    .enumerate()
                    .map(|(k, (r, &y))| {
                        let m = if nested { mine[k] } else { interpolate(&w.x, mine, r[0]) };
                        (m - y).abs()
                    })
                    .fold(0.0, f64::max);
                out.push((name.to_owned(), format!("{sig} ({kind})"), 100.0 * err / span));
            }
        }
    }
    out
}

#[test]
fn matches_ngspice_within_one_percent() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/circuits");
    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| {
            let p = e.ok()?.path();
            (p.extension()? == "cir").then(|| p.file_stem().unwrap().to_string_lossy().into_owned())
        })
        .collect();
    names.sort();
    assert!(names.len() >= 10);
    let mut failures = Vec::new();
    println!("| circuit | signal | max error % |\n|---|---|---|");
    for name in &names {
        for (c, sig, err) in compare(name) {
            println!("| {c} | {sig} | {err:.4} |");
            if err.is_nan() || err >= LIMIT_PERCENT {
                failures.push(format!("{c} {sig}: {err:.3} %"));
            }
        }
    }
    assert!(failures.is_empty(), "outside {LIMIT_PERCENT} %: {failures:#?}");
}

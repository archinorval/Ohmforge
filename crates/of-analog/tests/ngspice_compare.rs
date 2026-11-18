//! Compare of-analog against ngspice reference data.
//!
//! The circuits live in `tests/circuits/*.cir`; `tests/golden/*.csv` were
//! produced by running ngspice as an external program
//! (`scripts/gen-ngspice-golden.sh`; see `tests/golden/README.md`). Every
//! compared signal must agree within 1 % of the reference signal's span
//! (or of its magnitude for operating points).
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
        a => {
            let w = match a {
                Analysis::Dc {
                    source,
                    start,
                    stop,
                    step,
                } => analysis::dc_sweep(&mut b.circuit, &b.options, &source, start, stop, step).expect("dc"),
                Analysis::Tran { .. } => {
                    let req = b.tran_request(&a).unwrap();
                    analysis::transient(&mut b.circuit, &b.options, &req).expect("tran")
                }
                Analysis::Op => unreachable!(),
            };
            let kind = if w.x_name == "time" { "tran" } else { "dc" };
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
                    .map(|(r, &y)| (interpolate(&w.x, mine, r[0]) - y).abs())
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

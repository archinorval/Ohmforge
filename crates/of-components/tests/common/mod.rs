//! Shared helpers for the bridge tests.
#![allow(dead_code)]

use std::path::Path;

use of_components::analog::Library;

/// The repository's parts library.
pub fn library() -> Library {
    Library::load_dir(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../parts")).expect("parts library")
}

/// An ngspice golden CSV from the of-analog reference set.
pub fn golden(name: &str) -> (Vec<String>, Vec<Vec<f64>>) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("../of-analog/tests/golden/{name}.csv"));
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let mut names = Vec::new();
    let mut rows = Vec::new();
    for line in text.lines() {
        if let Some(cols) = line.strip_prefix("# columns: ") {
            names = cols.split_whitespace().skip(1).map(str::to_owned).collect();
        } else if !line.starts_with('#') && !line.trim().is_empty() {
            rows.push(line.split(',').map(|v| v.trim().parse().unwrap()).collect());
        }
    }
    (names, rows)
}

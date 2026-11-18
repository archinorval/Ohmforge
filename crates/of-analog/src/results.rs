//! Analysis results and CSV/VCD writers.

use std::io::{self, Write};

use crate::analysis::OpMethod;
use crate::circuit::Circuit;

/// A DC operating point.
#[derive(Debug, Clone, PartialEq)]
pub struct OpPoint {
    /// Signal names (`v(node)`, `i(source)`), internal nodes excluded.
    pub names: Vec<String>,
    /// Values in the same order.
    pub values: Vec<f64>,
    /// Which method converged.
    pub method: OpMethod,
    /// Newton iterations used in total.
    pub iterations: usize,
}

fn output_vars(ckt: &Circuit) -> Vec<usize> {
    ckt.vars()
        .iter()
        .enumerate()
        .skip(1)
        .filter(|(_, v)| !v.internal)
        .map(|(i, _)| i)
        .collect()
}

impl OpPoint {
    pub(crate) fn new(ckt: &Circuit, x: &[f64], method: OpMethod, iterations: usize) -> Self {
        let idx = output_vars(ckt);
        Self {
            names: idx.iter().map(|&i| ckt.vars()[i].name.clone()).collect(),
            values: idx.iter().map(|&i| x[i]).collect(),
            method,
            iterations,
        }
    }

    /// Value of a signal by name (case-insensitive), e.g. `v(out)`.
    pub fn get(&self, name: &str) -> Option<f64> {
        let key = name.to_ascii_lowercase();
        self.names.iter().position(|n| *n == key).map(|i| self.values[i])
    }

    /// Write `name,value` lines.
    pub fn write_csv(&self, w: &mut dyn Write) -> io::Result<()> {
        writeln!(w, "signal,value")?;
        for (n, v) in self.names.iter().zip(&self.values) {
            writeln!(w, "{n},{v:.12e}")?;
        }
        Ok(())
    }
}

/// Counters from a transient run.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TranStats {
    /// Accepted time points.
    pub accepted: usize,
    /// Steps rejected for truncation error.
    pub rejected_lte: usize,
    /// Steps rejected because Newton did not converge.
    pub rejected_newton: usize,
    /// Newton iterations in total.
    pub newton_iterations: usize,
    /// Breakpoints hit exactly.
    pub breakpoints: usize,
    /// LU factorisations.
    pub factorisations: u64,
}

/// Signals against a swept variable (time for transient, a source value
/// for DC sweeps).
#[derive(Debug, Clone, PartialEq)]
pub struct Waveforms {
    /// Name of the independent variable (`time` or the swept source).
    pub x_name: String,
    /// Independent variable values.
    pub x: Vec<f64>,
    /// Signal names.
    pub names: Vec<String>,
    /// One column per signal.
    pub columns: Vec<Vec<f64>>,
    /// Transient statistics (zero for sweeps).
    pub stats: TranStats,
    vars: Vec<usize>,
}

/// A DC sweep result.
pub type Sweep = Waveforms;

impl Waveforms {
    pub(crate) fn new(ckt: &Circuit, x_name: String) -> Self {
        let vars = output_vars(ckt);
        Self {
            x_name,
            x: Vec::new(),
            names: vars.iter().map(|&i| ckt.vars()[i].name.clone()).collect(),
            columns: vec![Vec::new(); vars.len()],
            stats: TranStats::default(),
            vars,
        }
    }

    pub(crate) fn push(&mut self, xv: f64, sol: &[f64]) {
        self.x.push(xv);
        for (col, &i) in self.columns.iter_mut().zip(&self.vars) {
            col.push(sol[i]);
        }
    }

    /// A signal's samples by name (case-insensitive).
    pub fn signal(&self, name: &str) -> Option<&[f64]> {
        let key = name.to_ascii_lowercase();
        self.names
            .iter()
            .position(|n| *n == key)
            .map(|i| self.columns[i].as_slice())
    }

    /// Linearly interpolated value of `name` at `x`.
    pub fn at(&self, name: &str, x: f64) -> Option<f64> {
        let y = self.signal(name)?;
        Some(interpolate(&self.x, y, x))
    }

    /// Resample every signal onto a uniform grid with spacing `step`.
    pub fn resample(&self, step: f64) -> Waveforms {
        let mut out = Waveforms {
            x: Vec::new(),
            columns: vec![Vec::new(); self.columns.len()],
            ..self.clone()
        };
        if self.x.is_empty() || step <= 0.0 {
            return out;
        }
        let (x0, x1) = (self.x[0], self.x[self.x.len() - 1]);
        let n = ((x1 - x0) / step + 1e-9).floor() as usize;
        for k in 0..=n {
            let x = x0 + step * k as f64;
            out.x.push(x);
            for (c, src) in out.columns.iter_mut().zip(&self.columns) {
                c.push(interpolate(&self.x, src, x));
            }
        }
        out
    }

    /// Write a CSV table with a header row.
    pub fn write_csv(&self, w: &mut dyn Write) -> io::Result<()> {
        write!(w, "{}", self.x_name)?;
        for n in &self.names {
            write!(w, ",{n}")?;
        }
        writeln!(w)?;
        for (k, x) in self.x.iter().enumerate() {
            write!(w, "{x:.12e}")?;
            for c in &self.columns {
                write!(w, ",{:.12e}", c[k])?;
            }
            writeln!(w)?;
        }
        Ok(())
    }

    /// Write an IEEE 1364 VCD file with `real` variables (time in fs).
    pub fn write_vcd(&self, w: &mut dyn Write) -> io::Result<()> {
        writeln!(w, "$date ohmforge $end")?;
        writeln!(w, "$version ohmforge {} $end", env!("CARGO_PKG_VERSION"))?;
        writeln!(w, "$timescale 1fs $end")?;
        writeln!(w, "$scope module circuit $end")?;
        let ids: Vec<String> = (0..self.names.len()).map(vcd_id).collect();
        for (n, id) in self.names.iter().zip(&ids) {
            let clean: String = n
                .chars()
                .map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' })
                .collect();
            writeln!(w, "$var real 64 {id} {} $end", clean.trim_end_matches('_'))?;
        }
        writeln!(w, "$upscope $end")?;
        writeln!(w, "$enddefinitions $end")?;
        let mut last_t = None;
        for (k, x) in self.x.iter().enumerate() {
            let t = (x * 1e15).round() as u64;
            if last_t == Some(t) {
                continue;
            }
            last_t = Some(t);
            writeln!(w, "#{t}")?;
            for (c, id) in self.columns.iter().zip(&ids) {
                writeln!(w, "r{:.10e} {id}", c[k])?;
            }
        }
        Ok(())
    }
}

fn vcd_id(mut i: usize) -> String {
    let mut s = String::new();
    loop {
        s.push((b'!' + (i % 94) as u8) as char);
        i /= 94;
        if i == 0 {
            break s;
        }
    }
}

/// Piecewise-linear interpolation of `(xs, ys)` at `x` (clamped at the ends).
pub fn interpolate(xs: &[f64], ys: &[f64], x: f64) -> f64 {
    if xs.is_empty() {
        return f64::NAN;
    }
    if x <= xs[0] {
        return ys[0];
    }
    let last = xs.len() - 1;
    if x >= xs[last] {
        return ys[last];
    }
    let i = xs.partition_point(|&v| v <= x).max(1);
    let (x0, x1) = (xs[i - 1], xs[i]);
    if x1 == x0 {
        return ys[i];
    }
    ys[i - 1] + (ys[i] - ys[i - 1]) * (x - x0) / (x1 - x0)
}

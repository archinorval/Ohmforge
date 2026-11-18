//! `ohmforge` command-line runner.
//!
//! ```text
//! ohmforge run <netlist.cir> [--out results.csv] [--vcd waves.vcd]
//! ```
//!
//! Runs every analysis in a SPICE netlist (`.op`, `.dc`, `.tran`). Operating
//! points are printed as a table; sweeps and transients are written as CSV to
//! `--out` (or standard output) and optionally as a VCD file.

use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use of_analog::{analysis, netlist};
use of_netlist::spice::{self, Analysis};

const USAGE: &str = "Usage:
  ohmforge run <netlist.cir> [--out FILE.csv] [--vcd FILE.vcd]
  ohmforge --version";

#[derive(Debug, Default)]
struct RunArgs {
    netlist: PathBuf,
    out: Option<PathBuf>,
    vcd: Option<PathBuf>,
}

fn parse_run(args: &[String]) -> Result<RunArgs, String> {
    let mut r = RunArgs::default();
    let mut it = args.iter();
    let mut netlist = None;
    while let Some(a) = it.next() {
        match a.as_str() {
            "--out" | "-o" => r.out = Some(it.next().ok_or("--out needs a file name")?.into()),
            "--vcd" => r.vcd = Some(it.next().ok_or("--vcd needs a file name")?.into()),
            s if s.starts_with('-') => return Err(format!("unknown option '{s}'")),
            s if netlist.is_none() => netlist = Some(PathBuf::from(s)),
            s => return Err(format!("unexpected argument '{s}'")),
        }
    }
    r.netlist = netlist.ok_or("missing netlist file")?;
    Ok(r)
}

/// `base` for the first analysis of its kind, `base.N` style suffix after.
fn numbered(path: &Path, index: usize) -> PathBuf {
    if index == 0 {
        return path.to_owned();
    }
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let ext = path
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    path.with_file_name(format!("{stem}.{}{ext}", index + 1))
}

fn run(args: &RunArgs) -> Result<(), String> {
    let deck = spice::parse_file(&args.netlist).map_err(|e| format!("{}: {e}", args.netlist.display()))?;
    let mut built = netlist::build(&deck).map_err(|e| format!("{}: {e}", args.netlist.display()))?;
    for w in &built.warnings {
        eprintln!("warning: {w}");
    }
    if built.analyses.is_empty() {
        return Err("the netlist has no analysis (.op, .dc or .tran)".into());
    }
    eprintln!("{}: {} unknowns", deck.title, built.circuit.unknowns());
    let mut written = 0;
    for a in built.analyses.clone() {
        let opts = built.options.clone();
        let waves = match &a {
            Analysis::Op => {
                let op = analysis::operating_point(&mut built.circuit, &opts).map_err(|e| e.to_string())?;
                println!("Operating point ({:?}, {} iterations)", op.method, op.iterations);
                for (n, v) in op.names.iter().zip(&op.values) {
                    println!("  {n:<24} {v:>16.9e}");
                }
                continue;
            }
            Analysis::Dc {
                source,
                start,
                stop,
                step,
            } => analysis::dc_sweep(&mut built.circuit, &opts, source, *start, *stop, *step)
                .map_err(|e| e.to_string())?,
            Analysis::Tran { .. } => {
                let req = built.tran_request(&a).expect("tran");
                let w = analysis::transient(&mut built.circuit, &opts, &req).map_err(|e| e.to_string())?;
                let s = &w.stats;
                eprintln!(
                    "transient: {} points, {} LTE and {} Newton rejections, {} breakpoints, {} Newton iterations",
                    s.accepted, s.rejected_lte, s.rejected_newton, s.breakpoints, s.newton_iterations
                );
                w
            }
        };
        let io = |e: io::Error| e.to_string();
        match &args.out {
            Some(p) => {
                let p = numbered(p, written);
                let mut f = BufWriter::new(File::create(&p).map_err(|e| format!("{}: {e}", p.display()))?);
                waves.write_csv(&mut f).and_then(|()| f.flush()).map_err(io)?;
                eprintln!("wrote {}", p.display());
            }
            None => waves.write_csv(&mut io::stdout().lock()).map_err(io)?,
        }
        if let Some(p) = &args.vcd {
            let p = numbered(p, written);
            let mut f = BufWriter::new(File::create(&p).map_err(|e| format!("{}: {e}", p.display()))?);
            waves.write_vcd(&mut f).and_then(|()| f.flush()).map_err(io)?;
            eprintln!("wrote {}", p.display());
        }
        written += 1;
    }
    Ok(())
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("--version" | "-V") => {
            println!("ohmforge {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Some("run") => match parse_run(&args[1..]).and_then(|a| run(&a)) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("error: {e}");
                ExitCode::FAILURE
            }
        },
        Some("--help" | "-h") => {
            println!("{USAGE}");
            ExitCode::SUCCESS
        }
        _ => {
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
    }
}

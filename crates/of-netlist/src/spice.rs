//! SPICE netlist subset parser.
//!
//! Reads the ngspice-compatible subset Ohmforge uses for tests, imports and
//! the `ohmforge run` command: element cards (R C L K V I D Q M E G F H S W
//! X), `.model`, `.subckt`/`.ends` (flattened on parse, with `params:` and
//! subcircuit-local `.param`/`.model`), `.param` expressions (see
//! [`crate::expr`]), `.op`, `.dc` (one or two sources), `.ac`, `.tran`,
//! `.temp`, `.ic`, `.options`, `.include` and `.end`.
//! `.control` blocks and output directives (`.print`, `.save`, ...) are
//! skipped so the same file runs unchanged in ngspice.
//!
//! Everything is case-insensitive and stored lower-case. Values accept the
//! SPICE scale suffixes (`f p n u m k meg g t mil`) followed by any unit
//! letters (`10uF`, `4.7kohm`).

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

/// A parse error with its 1-based line number.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    /// Line in the (first) file where the card starts; 0 if not applicable.
    pub line: usize,
    /// What went wrong.
    pub message: String,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.line > 0 {
            write!(f, "line {}: {}", self.line, self.message)
        } else {
            f.write_str(&self.message)
        }
    }
}

impl std::error::Error for ParseError {}

fn err<T>(line: usize, message: impl Into<String>) -> Result<T, ParseError> {
    Err(ParseError {
        line,
        message: message.into(),
    })
}

/// Parse a SPICE number with an optional scale suffix and trailing units.
pub fn parse_value(token: &str) -> Option<f64> {
    let t = token.trim().to_ascii_lowercase();
    let bytes = t.as_bytes();
    let mut end = 0;
    let mut seen_digit = false;
    let mut seen_exp = false;
    while end < bytes.len() {
        let c = bytes[end];
        let ok = match c {
            b'0'..=b'9' => {
                seen_digit = true;
                true
            }
            b'.' => !seen_exp,
            b'+' | b'-' => end == 0 || matches!(bytes[end - 1], b'e'),
            b'e' if seen_digit && !seen_exp => {
                // Only an exponent if a digit (or sign + digit) follows.
                let next = bytes.get(end + 1).copied();
                let after = bytes.get(end + 2).copied();
                let is_exp = matches!(next, Some(b'0'..=b'9'))
                    || (matches!(next, Some(b'+' | b'-')) && matches!(after, Some(b'0'..=b'9')));
                if is_exp {
                    seen_exp = true;
                }
                is_exp
            }
            _ => false,
        };
        if !ok {
            break;
        }
        end += 1;
    }
    if !seen_digit {
        return None;
    }
    let base: f64 = t[..end].parse().ok()?;
    let rest = &t[end..];
    let scale = if rest.starts_with("meg") {
        1e6
    } else if rest.starts_with("mil") {
        25.4e-6
    } else {
        match rest.as_bytes().first() {
            None => 1.0,
            Some(b't') => 1e12,
            Some(b'g') => 1e9,
            Some(b'k') => 1e3,
            Some(b'm') => 1e-3,
            Some(b'u') => 1e-6,
            Some(b'n') => 1e-9,
            Some(b'p') => 1e-12,
            Some(b'f') => 1e-15,
            Some(b'a') => 1e-18,
            Some(c) if c.is_ascii_alphabetic() => 1.0, // unit only, e.g. "5v"
            Some(_) => return None,
        }
    };
    Some(base * scale)
}

/// Kind of a time-domain source waveform.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaveKind {
    /// `PULSE(v1 v2 td tr tf pw per)`
    Pulse,
    /// `SIN(vo va freq td theta phase)`
    Sin,
    /// `EXP(v1 v2 td1 tau1 td2 tau2)`
    Exp,
    /// `PWL(t1 v1 t2 v2 ...)`
    Pwl,
}

/// A source waveform with its raw arguments (defaults are applied by the
/// solver, because some depend on the transient step and stop time).
#[derive(Debug, Clone, PartialEq)]
pub struct Waveform {
    /// Waveform type.
    pub kind: WaveKind,
    /// Arguments in card order.
    pub args: Vec<f64>,
}

/// Value specification of an independent source.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SourceSpec {
    /// DC value (`DC 5` or a bare number).
    pub dc: Option<f64>,
    /// AC magnitude for `.ac` (`AC mag [phase]`).
    pub ac: Option<f64>,
    /// AC phase in degrees.
    pub ac_phase: f64,
    /// Transient waveform.
    pub wave: Option<Waveform>,
}

/// One element card after subcircuit flattening.
#[derive(Debug, Clone, PartialEq)]
pub struct Element {
    /// Full name, lower-case, e.g. `r1` or `x1.r1` inside a subcircuit.
    pub name: String,
    /// Node names in card order (ground is `0`).
    pub nodes: Vec<String>,
    /// Principal value (resistance, capacitance, gain, ...), if any.
    pub value: Option<f64>,
    /// Model name for D, Q and M elements.
    pub model: Option<String>,
    /// Controlling voltage source for F, H and W elements.
    pub control: Option<String>,
    /// Referenced elements: the two inductors of a K coupling.
    pub refs: Vec<String>,
    /// `key=value` instance parameters (`ic`, `w`, `l`, `area`, `m`, ...).
    pub params: BTreeMap<String, f64>,
    /// Flags such as `off`.
    pub flags: Vec<String>,
    /// Source specification for V and I elements.
    pub source: Option<SourceSpec>,
    /// `POLY(n)` coefficients of a nonlinear E or G source (empty for a
    /// linear one); `nodes[2..]` are then the `n` controlling node pairs.
    pub coeffs: Vec<f64>,
    /// Line number of the card (in the top-level file).
    pub line: usize,
}

impl Element {
    /// The element type letter (`r`, `c`, `q`, ...).
    pub fn letter(&self) -> char {
        // Flattened subcircuit elements are named `x1.r1`: the type letter
        // is the first character of the last path segment.
        self.name
            .rsplit('.')
            .next()
            .and_then(|s| s.chars().next())
            .unwrap_or('?')
    }
}

/// A `.model` card.
#[derive(Debug, Clone, PartialEq)]
pub struct Model {
    /// Model name.
    pub name: String,
    /// Model type: `d`, `npn`, `pnp`, `nmos`, `pmos`, ...
    pub kind: String,
    /// Parameters.
    pub params: BTreeMap<String, f64>,
    /// Line number.
    pub line: usize,
}

/// An analysis request.
#[derive(Debug, Clone, PartialEq)]
pub enum Analysis {
    /// `.op`
    Op,
    /// `.dc src start stop step [src2 start2 stop2 step2]`
    Dc {
        /// Swept source name (inner loop).
        source: String,
        /// First value.
        start: f64,
        /// Last value.
        stop: f64,
        /// Increment.
        step: f64,
        /// Optional outer sweep (`src2 start2 stop2 step2`).
        outer: Option<DcSweep>,
    },
    /// `.ac dec|oct|lin points fstart fstop`
    Ac {
        /// Point spacing.
        sweep: AcSweep,
        /// Points per decade/octave, or total points for `lin`.
        points: usize,
        /// First frequency (Hz).
        fstart: f64,
        /// Last frequency (Hz).
        fstop: f64,
    },
    /// `.tran tstep tstop [tstart [tmax]] [uic]`
    Tran {
        /// Suggested output step.
        tstep: f64,
        /// Stop time.
        tstop: f64,
        /// Time from which results are kept.
        tstart: f64,
        /// Maximum internal step (0 = automatic).
        tmax: f64,
        /// Skip the operating point and start from initial conditions.
        uic: bool,
    },
}

/// The outer source of a nested `.dc` sweep.
#[derive(Debug, Clone, PartialEq)]
pub struct DcSweep {
    /// Source name.
    pub source: String,
    /// First value.
    pub start: f64,
    /// Last value.
    pub stop: f64,
    /// Increment.
    pub step: f64,
}

/// Frequency spacing of an `.ac` sweep.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcSweep {
    /// `dec`: points per decade.
    Dec,
    /// `oct`: points per octave.
    Oct,
    /// `lin`: total points, linearly spaced.
    Lin,
}

impl AcSweep {
    /// The frequencies of a sweep (SPICE semantics: logarithmic sweeps step
    /// by a constant ratio from `fstart` and stop at `fstop` within a small
    /// tolerance).
    pub fn frequencies(self, points: usize, fstart: f64, fstop: f64) -> Vec<f64> {
        let points = points.max(1);
        match self {
            AcSweep::Lin => {
                if points == 1 {
                    return vec![fstart];
                }
                let step = (fstop - fstart) / (points - 1) as f64;
                (0..points).map(|k| fstart + step * k as f64).collect()
            }
            AcSweep::Dec | AcSweep::Oct => {
                let base: f64 = if self == AcSweep::Dec { 10.0 } else { 2.0 };
                let ratio = base.powf(1.0 / points as f64);
                let mut out = Vec::new();
                let mut f = fstart;
                let mut k = 0i32;
                while f <= fstop * (1.0 + 1e-9) && out.len() < 1_000_000 {
                    out.push(f);
                    k += 1;
                    f = fstart * ratio.powi(k);
                }
                out
            }
        }
    }
}

/// A parsed, flattened netlist.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Deck {
    /// Title line.
    pub title: String,
    /// Elements in card order.
    pub elements: Vec<Element>,
    /// Models by name.
    pub models: BTreeMap<String, Model>,
    /// Analyses in card order.
    pub analyses: Vec<Analysis>,
    /// `.options` values (flags are stored as 1.0); `method=gear` is stored
    /// as `method` = 2.0, `method=trap` as 1.0.
    pub options: BTreeMap<String, f64>,
    /// `.ic v(node)=value` initial conditions.
    pub initial_conditions: BTreeMap<String, f64>,
    /// Circuit temperature from `.temp` (°C), if given.
    pub temp: Option<f64>,
    /// Global parameters after evaluating every `.param`.
    pub params: BTreeMap<String, f64>,
    /// Non-fatal notes about ignored cards.
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone)]
struct Card {
    line: usize,
    tokens: Vec<String>,
    /// The logical line as written (for expressions).
    raw: String,
}

#[derive(Debug, Clone)]
struct Subckt {
    ports: Vec<String>,
    /// Default parameters (`params:` on the `.subckt` line), unevaluated.
    defaults: Vec<(String, String)>,
    body: Vec<Card>,
}

/// Parameters and model names visible while expanding one (sub)circuit.
#[derive(Debug, Clone, Default)]
struct Scope {
    params: BTreeMap<String, f64>,
    /// Local model name to the flattened global name.
    models: BTreeMap<String, String>,
}

/// Find `name = value` assignments in `text` (values may be bare
/// expressions with spaces, `{...}` or `'...'`). Returns the byte offset of
/// the first assignment and the pairs.
fn assignments(text: &str) -> (usize, Vec<(String, String)>) {
    let b = text.as_bytes();
    // Positions of `ident =` at bracket depth 0 (not ==, <=, >=, !=).
    let mut marks: Vec<(usize, usize, String)> = Vec::new(); // (ident start, value start, name)
    let mut depth = 0i32;
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'(' | b'{' | b'[' => depth += 1,
            b')' | b'}' | b']' => depth -= 1,
            b'\'' => {
                // Skip quoted expressions.
                i += 1;
                while i < b.len() && b[i] != b'\'' {
                    i += 1;
                }
            }
            b'=' if depth == 0
                && b.get(i + 1) != Some(&b'=')
                && !matches!(i.checked_sub(1).map(|k| b[k]), Some(b'=' | b'<' | b'>' | b'!')) =>
            {
                let mut j = i;
                while j > 0 && b[j - 1].is_ascii_whitespace() {
                    j -= 1;
                }
                let end = j;
                while j > 0 && (b[j - 1].is_ascii_alphanumeric() || b[j - 1] == b'_') {
                    j -= 1;
                }
                if j < end {
                    marks.push((j, i + 1, text[j..end].to_ascii_lowercase()));
                }
            }
            _ => {}
        }
        i += 1;
    }
    let first = marks.first().map_or(text.len(), |m| m.0);
    let mut out = Vec::new();
    for (k, (_, vstart, name)) in marks.iter().enumerate() {
        let vend = marks.get(k + 1).map_or(text.len(), |m| m.0);
        let v = text[*vstart..vend].trim().trim_end_matches(',').trim();
        out.push((name.clone(), v.to_string()));
    }
    (first, out)
}

/// Evaluate a parameter value: `{expr}`, `'expr'` or a bare expression.
fn eval_value(v: &str, params: &BTreeMap<String, f64>, line: usize) -> Result<f64, ParseError> {
    let v = v.trim();
    let inner = v
        .strip_prefix('{')
        .and_then(|t| t.strip_suffix('}'))
        .or_else(|| v.strip_prefix('\'').and_then(|t| t.strip_suffix('\'')))
        .unwrap_or(v);
    crate::expr::eval(inner, params).map_err(|message| ParseError { line, message })
}

/// Text after the first `n` whitespace-separated words of `raw`.
fn after_words(raw: &str, n: usize) -> &str {
    let mut rest = raw.trim_start();
    for _ in 0..n {
        rest = rest.trim_start();
        let cut = rest.find(char::is_whitespace).unwrap_or(rest.len());
        rest = &rest[cut..];
    }
    rest
}

/// Split the `.subckt`/`X` parameter part off a card: returns the tokens
/// before it and the assignments.
fn split_params(card: &Card) -> (Vec<String>, Vec<(String, String)>) {
    let lower = card.raw.to_ascii_lowercase();
    if let Some(at) = lower.find("params:") {
        let (_, pairs) = assignments(&card.raw[at + 7..]);
        return (tokenize(&card.raw[..at]), pairs);
    }
    let (first, pairs) = assignments(&card.raw);
    (tokenize(&card.raw[..first]), pairs)
}

/// Split a logical line into tokens: whitespace and commas separate,
/// `(`, `)` and `=` are tokens of their own, `{...}` stays one token.
fn tokenize(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut depth = 0;
    for c in line.chars() {
        if depth > 0 {
            cur.push(c);
            if c == '}' {
                depth -= 1;
            }
            continue;
        }
        match c {
            '{' => {
                depth += 1;
                cur.push(c);
            }
            c if c.is_whitespace() || c == ',' => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            '(' | ')' | '=' => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
                out.push(c.to_string());
            }
            _ => cur.push(c.to_ascii_lowercase()),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn strip_comment(line: &str) -> &str {
    let mut cut = line.len();
    for pat in [";", " $", "\t$"] {
        if let Some(i) = line.find(pat) {
            cut = cut.min(i);
        }
    }
    &line[..cut]
}

/// Join continuation lines and drop comments; returns (line number, text).
fn logical_lines(text: &str, skip_title: bool) -> (String, Vec<(usize, String)>) {
    let mut title = String::new();
    let mut lines: Vec<(usize, String)> = Vec::new();
    for (i, raw) in text.lines().enumerate() {
        let n = i + 1;
        if skip_title && i == 0 {
            title = raw.trim().to_string();
            continue;
        }
        let trimmed = raw.trim_start();
        if trimmed.starts_with('*') || trimmed.is_empty() {
            continue;
        }
        let body = strip_comment(trimmed);
        if let Some(cont) = body.strip_prefix('+')
            && let Some(last) = lines.last_mut()
        {
            last.1.push(' ');
            last.1.push_str(cont);
            continue;
        }
        lines.push((n, body.to_string()));
    }
    (title, lines)
}

/// Parse netlist text. `.include` is resolved relative to the current
/// directory; use [`parse_file`] to resolve it relative to the file.
pub fn parse_str(text: &str) -> Result<Deck, ParseError> {
    parse_with_base(text, None)
}

/// Read and parse a netlist file.
pub fn parse_file(path: &Path) -> Result<Deck, ParseError> {
    let text = std::fs::read_to_string(path).map_err(|e| ParseError {
        line: 0,
        message: format!("cannot read {}: {e}", path.display()),
    })?;
    parse_with_base(&text, path.parent().map(Path::to_path_buf))
}

fn read_cards(
    text: &str,
    skip_title: bool,
    base: &Option<PathBuf>,
    depth: usize,
) -> Result<(String, Vec<Card>), ParseError> {
    if depth > 16 {
        return err(0, ".include nested too deeply");
    }
    let (title, lines) = logical_lines(text, skip_title);
    let mut cards = Vec::new();
    let mut in_control = false;
    for (line, body) in lines {
        let lower = body.trim().to_ascii_lowercase();
        if lower.starts_with(".control") {
            in_control = true;
            continue;
        }
        if in_control {
            if lower.starts_with(".endc") {
                in_control = false;
            }
            continue;
        }
        if lower.starts_with(".include") || lower.starts_with(".inc ") {
            let arg = body
                .trim()
                .split_once(char::is_whitespace)
                .map(|x| x.1.trim())
                .unwrap_or("");
            let arg = arg.trim_matches(|c| c == '"' || c == '\'');
            let path = match base {
                Some(b) => b.join(arg),
                None => PathBuf::from(arg),
            };
            let inc = std::fs::read_to_string(&path).map_err(|e| ParseError {
                line,
                message: format!("cannot include {}: {e}", path.display()),
            })?;
            let (_, mut sub) = read_cards(&inc, false, &path.parent().map(Path::to_path_buf), depth + 1)?;
            for c in &mut sub {
                c.line = line;
            }
            cards.extend(sub);
            continue;
        }
        if lower.starts_with(".end") && !lower.starts_with(".ends") {
            break;
        }
        let tokens = tokenize(&body);
        if !tokens.is_empty() {
            cards.push(Card {
                line,
                tokens,
                raw: body.trim().to_string(),
            });
        }
    }
    Ok((title, cards))
}

fn parse_with_base(text: &str, base: Option<PathBuf>) -> Result<Deck, ParseError> {
    let (title, cards) = read_cards(text, true, &base, 0)?;
    let mut deck = Deck {
        title,
        ..Deck::default()
    };

    // Pass 1: split out subcircuit definitions.
    let mut subckts: BTreeMap<String, Subckt> = BTreeMap::new();
    let mut top: Vec<Card> = Vec::new();
    let mut open: Option<(String, Subckt)> = None;
    for card in cards {
        let head = card.tokens[0].as_str();
        if head == ".subckt" {
            if open.is_some() {
                return err(card.line, "nested .subckt definitions are not supported");
            }
            let (toks, defaults) = split_params(&card);
            let name = toks.get(1).cloned().ok_or(ParseError {
                line: card.line,
                message: ".subckt needs a name".into(),
            })?;
            let ports = toks[2..].to_vec();
            open = Some((
                name,
                Subckt {
                    ports,
                    defaults,
                    body: Vec::new(),
                },
            ));
        } else if head == ".ends" {
            let (name, sub) = open.take().ok_or(ParseError {
                line: card.line,
                message: ".ends without .subckt".into(),
            })?;
            subckts.insert(name, sub);
        } else if let Some((_, sub)) = open.as_mut() {
            sub.body.push(card);
        } else {
            top.push(card);
        }
    }
    if open.is_some() {
        return err(0, "missing .ends");
    }

    // Pass 2: parameters (in order), then models, then elements and
    // directives.
    let mut scope = Scope::default();
    for card in top.iter().filter(|c| c.tokens[0] == ".param") {
        eval_params(card, &mut scope.params)?;
    }
    for card in top.iter().filter(|c| c.tokens[0] == ".model") {
        let m = parse_model(card, &scope.params)?;
        deck.models.insert(m.name.clone(), m);
    }
    for card in &top {
        let head = card.tokens[0].as_str();
        if head == ".param" || head == ".model" {
            continue;
        }
        if head.starts_with('.') {
            parse_directive(card, &scope.params, &mut deck)?;
        } else {
            expand_element(card, "", &BTreeMap::new(), &subckts, &scope, &mut deck, 0)?;
        }
    }
    deck.params = scope.params;
    Ok(deck)
}

/// Evaluate a `.param` card into `params` (left to right, so later
/// assignments may use earlier ones).
fn eval_params(card: &Card, params: &mut BTreeMap<String, f64>) -> Result<(), ParseError> {
    let (_, pairs) = assignments(after_words(&card.raw, 1));
    if pairs.is_empty() {
        return err(card.line, "expected name=value in .param");
    }
    for (name, v) in pairs {
        if v.is_empty() {
            return err(card.line, format!("missing value for parameter '{name}'"));
        }
        let x = eval_value(&v, params, card.line)?;
        params.insert(name, x);
    }
    Ok(())
}

fn value_of(token: &str, params: &BTreeMap<String, f64>, line: usize) -> Result<f64, ParseError> {
    if token.starts_with('{') || token.starts_with('\'') {
        return eval_value(token, params, line);
    }
    if let Some(v) = parse_value(token) {
        return Ok(v);
    }
    params
        .get(token)
        .copied()
        .map_or_else(|| err(line, format!("bad number '{token}'")), Ok)
}

/// Parse `key = value` pairs from `tokens`, skipping parentheses.
fn key_values(
    tokens: &[String],
    params: &BTreeMap<String, f64>,
    line: usize,
    out: &mut BTreeMap<String, f64>,
    flags: &mut Vec<String>,
) -> Result<(), ParseError> {
    let toks: Vec<&String> = tokens.iter().filter(|t| *t != "(" && *t != ")").collect();
    let mut i = 0;
    while i < toks.len() {
        if toks.get(i + 1).map(|t| t.as_str()) == Some("=") {
            let v = toks.get(i + 2).ok_or(ParseError {
                line,
                message: format!("missing value for {}", toks[i]),
            })?;
            out.insert(toks[i].clone(), value_of(v, params, line)?);
            i += 3;
        } else {
            flags.push(toks[i].clone());
            i += 1;
        }
    }
    Ok(())
}

fn parse_model(card: &Card, params: &BTreeMap<String, f64>) -> Result<Model, ParseError> {
    let t = &card.tokens;
    if t.len() < 3 {
        return err(card.line, ".model needs a name and a type");
    }
    let mut p = BTreeMap::new();
    let mut flags = Vec::new();
    key_values(&t[3..], params, card.line, &mut p, &mut flags)?;
    if let Some(f) = flags.first() {
        return err(card.line, format!("unexpected '{f}' in .model {}", t[1]));
    }
    Ok(Model {
        name: t[1].clone(),
        kind: t[2].clone(),
        params: p,
        line: card.line,
    })
}

fn parse_directive(card: &Card, params: &BTreeMap<String, f64>, deck: &mut Deck) -> Result<(), ParseError> {
    let t = &card.tokens;
    let line = card.line;
    let num = |i: usize| -> Result<f64, ParseError> {
        let tok = t.get(i).ok_or(ParseError {
            line,
            message: format!("{} needs more arguments", t[0]),
        })?;
        value_of(tok, params, line)
    };
    match t[0].as_str() {
        ".op" => deck.analyses.push(Analysis::Op),
        ".dc" => {
            let src = |i: usize| -> Result<String, ParseError> {
                t.get(i).cloned().ok_or(ParseError {
                    line,
                    message: ".dc needs a source".into(),
                })
            };
            let outer = if t.len() > 5 {
                Some(DcSweep {
                    source: src(5)?,
                    start: num(6)?,
                    stop: num(7)?,
                    step: num(8)?,
                })
            } else {
                None
            };
            deck.analyses.push(Analysis::Dc {
                source: src(1)?,
                start: num(2)?,
                stop: num(3)?,
                step: num(4)?,
                outer,
            });
        }
        ".ac" => {
            let sweep = match t.get(1).map(String::as_str) {
                Some("dec") => AcSweep::Dec,
                Some("oct") => AcSweep::Oct,
                Some("lin") => AcSweep::Lin,
                _ => return err(line, ".ac needs dec, oct or lin"),
            };
            let points = num(2)?;
            if points.is_nan() || points < 1.0 {
                return err(line, ".ac needs at least one point");
            }
            let (fstart, fstop) = (num(3)?, num(4)?);
            if (sweep != AcSweep::Lin && (fstart.is_nan() || fstart <= 0.0)) || fstop.is_nan() || fstop < fstart {
                return err(line, ".ac needs 0 < fstart <= fstop");
            }
            deck.analyses.push(Analysis::Ac {
                sweep,
                points: points.round() as usize,
                fstart,
                fstop,
            });
        }
        ".tran" => {
            let uic = t.iter().any(|x| x == "uic");
            let nums: Vec<&String> = t[1..].iter().filter(|x| *x != "uic").collect();
            let get = |i: usize| -> Result<f64, ParseError> {
                nums.get(i).map_or(Ok(0.0), |tok| value_of(tok, params, line))
            };
            if nums.len() < 2 {
                return err(line, ".tran needs tstep and tstop");
            }
            deck.analyses.push(Analysis::Tran {
                tstep: get(0)?,
                tstop: get(1)?,
                tstart: get(2)?,
                tmax: get(3)?,
                uic,
            });
        }
        ".ic" | ".nodeset" => {
            if t[0] == ".nodeset" {
                deck.warnings.push(format!("line {line}: .nodeset is treated like .ic"));
            }
            // v ( node ) = value ...
            let mut i = 1;
            while i < t.len() {
                if t[i] != "v"
                    || t.get(i + 1).map(String::as_str) != Some("(")
                    || t.get(i + 3).map(String::as_str) != Some(")")
                    || t.get(i + 4).map(String::as_str) != Some("=")
                {
                    return err(line, "expected v(node)=value in .ic");
                }
                let v = num(i + 5)?;
                deck.initial_conditions.insert(t[i + 2].clone(), v);
                i += 6;
            }
        }
        ".options" | ".option" | ".opt" => {
            let mut i = 1;
            while i < t.len() {
                if t.get(i + 1).map(String::as_str) == Some("=") {
                    let key = t[i].clone();
                    let raw = t.get(i + 2).ok_or(ParseError {
                        line,
                        message: format!("missing value for {key}"),
                    })?;
                    let v = match (key.as_str(), raw.as_str()) {
                        ("method", "gear") => 2.0,
                        ("method", "trap" | "trapezoidal") => 1.0,
                        _ => value_of(raw, params, line)?,
                    };
                    deck.options.insert(key, v);
                    i += 3;
                } else {
                    deck.options.insert(t[i].clone(), 1.0);
                    i += 1;
                }
            }
        }
        ".print" | ".plot" | ".probe" | ".save" | ".meas" | ".measure" | ".width" | ".title" => {}
        ".temp" => {
            deck.temp = Some(num(1)?);
            if t.len() > 2 {
                deck.warnings
                    .push(format!("line {line}: only the first .temp value is used"));
            }
        }
        ".noise" | ".tf" | ".sens" | ".pz" | ".four" | ".disto" => {
            deck.warnings
                .push(format!("line {line}: {} analysis is not supported yet", t[0]));
        }
        other => return err(line, format!("unsupported directive {other}")),
    }
    Ok(())
}

fn parse_source(tokens: &[String], params: &BTreeMap<String, f64>, line: usize) -> Result<SourceSpec, ParseError> {
    let mut spec = SourceSpec::default();
    let mut i = 0;
    while i < tokens.len() {
        let tok = tokens[i].as_str();
        let wave = match tok {
            "pulse" => Some(WaveKind::Pulse),
            "sin" | "sine" => Some(WaveKind::Sin),
            "exp" => Some(WaveKind::Exp),
            "pwl" => Some(WaveKind::Pwl),
            _ => None,
        };
        if let Some(kind) = wave {
            i += 1;
            let paren = tokens.get(i).map(String::as_str) == Some("(");
            if paren {
                i += 1;
            }
            let mut args = Vec::new();
            while i < tokens.len() && tokens[i] != ")" {
                if !paren && parse_value(&tokens[i]).is_none() && !tokens[i].starts_with('{') {
                    break;
                }
                args.push(value_of(&tokens[i], params, line)?);
                i += 1;
            }
            if paren {
                i += 1; // ')'
            }
            spec.wave = Some(Waveform { kind, args });
            continue;
        }
        match tok {
            "dc" => {
                spec.dc = Some(value_of(
                    tokens.get(i + 1).ok_or(ParseError {
                        line,
                        message: "DC needs a value".into(),
                    })?,
                    params,
                    line,
                )?);
                i += 2;
            }
            "ac" => {
                let num_at = |k: usize| -> Option<f64> {
                    let tok = tokens.get(k)?;
                    if parse_value(tok).is_some() || tok.starts_with('{') {
                        value_of(tok, params, line).ok()
                    } else {
                        None
                    }
                };
                let mag = num_at(i + 1);
                spec.ac = Some(mag.unwrap_or(1.0));
                i += if mag.is_some() { 2 } else { 1 };
                if mag.is_some()
                    && let Some(ph) = num_at(i)
                {
                    spec.ac_phase = ph;
                    i += 1;
                }
            }
            _ => {
                if spec.dc.is_none() && spec.wave.is_none() {
                    spec.dc = Some(value_of(tok, params, line)?);
                    i += 1;
                } else {
                    return err(line, format!("unexpected '{tok}' in source"));
                }
            }
        }
    }
    Ok(spec)
}

#[allow(clippy::too_many_arguments)]
fn expand_element(
    card: &Card,
    prefix: &str,
    port_map: &BTreeMap<String, String>,
    subckts: &BTreeMap<String, Subckt>,
    scope: &Scope,
    deck: &mut Deck,
    depth: usize,
) -> Result<(), ParseError> {
    let t = &card.tokens;
    let line = card.line;
    let params = &scope.params;
    let name = if prefix.is_empty() {
        t[0].clone()
    } else {
        format!("{prefix}.{}", t[0])
    };
    let map_node = |n: &str| -> String {
        if n == "0" || n == "gnd" {
            "0".to_string()
        } else if let Some(m) = port_map.get(n) {
            m.clone()
        } else if prefix.is_empty() {
            n.to_string()
        } else {
            format!("{prefix}.{n}")
        }
    };
    // Element names referenced from inside a subcircuit are local to it.
    let map_ref = |n: &str| -> String {
        if prefix.is_empty() {
            n.to_string()
        } else {
            format!("{prefix}.{n}")
        }
    };
    let model_name = |n: &str| -> String { scope.models.get(n).cloned().unwrap_or_else(|| n.to_string()) };
    let known_model = |n: &str| scope.models.contains_key(n) || deck.models.contains_key(n);
    let letter = t[0].chars().next().unwrap_or('?');
    let need = |k: usize| -> Result<(), ParseError> {
        if t.len() < k + 1 {
            err(line, format!("{} needs at least {k} fields", t[0]))
        } else {
            Ok(())
        }
    };
    let mut e = Element {
        name,
        nodes: Vec::new(),
        value: None,
        model: None,
        control: None,
        refs: Vec::new(),
        params: BTreeMap::new(),
        flags: Vec::new(),
        source: None,
        coeffs: Vec::new(),
        line,
    };
    match letter {
        'r' | 'c' | 'l' => {
            need(3)?;
            e.nodes = vec![map_node(&t[1]), map_node(&t[2])];
            let rest = &t[3..];
            let (val, kv) = if rest.get(1).map(String::as_str) == Some("=") {
                (None, rest)
            } else {
                (rest.first(), rest.get(1..).unwrap_or(&[]))
            };
            if let Some(v) = val {
                e.value = Some(value_of(v, params, line)?);
            }
            key_values(kv, params, line, &mut e.params, &mut e.flags)?;
            let key = match letter {
                'r' => "r",
                'c' => "c",
                _ => "l",
            };
            if e.value.is_none() {
                e.value = e.params.remove(key);
            }
            if e.value.is_none() {
                return err(line, format!("{} has no value", t[0]));
            }
        }
        'k' => {
            need(3)?;
            e.refs = vec![map_ref(&t[1]), map_ref(&t[2])];
            e.value = Some(value_of(&t[3], params, line)?);
        }
        'v' | 'i' => {
            need(2)?;
            e.nodes = vec![map_node(&t[1]), map_node(&t[2])];
            e.source = Some(parse_source(&t[3..], params, line)?);
        }
        'd' => {
            need(3)?;
            e.nodes = vec![map_node(&t[1]), map_node(&t[2])];
            e.model = Some(model_name(&t[3]));
            instance_rest(&t[4..], "area", params, line, &mut e)?;
        }
        'q' => {
            need(4)?;
            // Optional substrate node: the model name is the first token that
            // names a known model.
            let model_at = if known_model(&t[4]) || t.len() < 6 { 4 } else { 5 };
            if !known_model(&t[model_at]) && known_model(&t[4]) {
                return err(line, format!("unknown model for {}", t[0]));
            }
            e.nodes = t[1..model_at].iter().map(|n| map_node(n)).collect();
            e.model = Some(model_name(&t[model_at]));
            instance_rest(&t[model_at + 1..], "area", params, line, &mut e)?;
        }
        'm' => {
            need(5)?;
            e.nodes = t[1..5].iter().map(|n| map_node(n)).collect();
            e.model = Some(model_name(&t[5]));
            instance_rest(&t[6..], "m", params, line, &mut e)?;
        }
        's' => {
            need(5)?;
            e.nodes = t[1..5].iter().map(|n| map_node(n)).collect();
            e.model = Some(model_name(&t[5]));
            key_values(&t[6..], params, line, &mut e.params, &mut e.flags)?;
        }
        'w' => {
            need(4)?;
            e.nodes = vec![map_node(&t[1]), map_node(&t[2])];
            e.control = Some(map_ref(&t[3]));
            e.model = Some(model_name(&t[4]));
            key_values(&t[5..], params, line, &mut e.params, &mut e.flags)?;
        }
        'e' | 'g' if t.get(3).is_some_and(|k| k == "poly") => {
            // SPICE2 polynomial source: `E n+ n- POLY(n) c1+ c1- ... p0 p1 ...`.
            let dim = match (t.get(4), t.get(5), t.get(6)) {
                (Some(o), Some(d), Some(c)) if o == "(" && c == ")" => d.parse::<usize>().ok(),
                _ => None,
            }
            .filter(|d| (1..=8).contains(d))
            .ok_or(ParseError {
                line,
                message: format!("{}: expected POLY(n) with 1 <= n <= 8", t[0]),
            })?;
            let ctrl_end = 7 + 2 * dim;
            if t.len() <= ctrl_end {
                return err(
                    line,
                    format!("{}: POLY({dim}) needs {} control nodes and coefficients", t[0], 2 * dim),
                );
            }
            e.nodes = [&t[1], &t[2]]
                .into_iter()
                .chain(&t[7..ctrl_end])
                .map(|n| map_node(n))
                .collect();
            e.coeffs = t[ctrl_end..]
                .iter()
                .map(|v| value_of(v, params, line))
                .collect::<Result<_, _>>()?;
        }
        'e' | 'g' => {
            need(5)?;
            e.nodes = t[1..5].iter().map(|n| map_node(n)).collect();
            e.value = Some(value_of(&t[5], params, line)?);
        }
        'f' | 'h' => {
            need(4)?;
            e.nodes = vec![map_node(&t[1]), map_node(&t[2])];
            e.control = Some(map_ref(&t[3]));
            e.value = Some(value_of(&t[4], params, line)?);
        }
        'x' => {
            if depth > 32 {
                return err(line, "subcircuit nesting too deep (recursive .subckt?)");
            }
            let (fields, overrides) = split_params(card);
            let (sub_name, nodes) = fields[1..].split_last().ok_or(ParseError {
                line,
                message: format!("{} needs a subcircuit name", t[0]),
            })?;
            let sub = subckts.get(sub_name.as_str()).ok_or(ParseError {
                line,
                message: format!("unknown subcircuit '{sub_name}'"),
            })?;
            if sub.ports.len() != nodes.len() {
                return err(
                    line,
                    format!(
                        "{} connects {} nodes but '{sub_name}' has {} ports",
                        t[0],
                        nodes.len(),
                        sub.ports.len()
                    ),
                );
            }
            let map: BTreeMap<String, String> = sub
                .ports
                .iter()
                .cloned()
                .zip(nodes.iter().map(|n| map_node(n)))
                .collect();
            // Child scope: caller's parameters, then the subcircuit defaults,
            // then the instance overrides (evaluated in the caller's scope),
            // then local .param cards; local models shadow outer ones.
            let mut child = scope.clone();
            for (k, v) in &sub.defaults {
                let x = eval_value(v, &child.params, line)?;
                child.params.insert(k.clone(), x);
            }
            for (k, v) in &overrides {
                if !sub.defaults.iter().any(|(d, _)| d == k) {
                    deck.warnings.push(format!(
                        "line {line}: {}: '{k}' is not a parameter of '{sub_name}'",
                        t[0]
                    ));
                }
                let x = eval_value(v, &scope.params, line)?;
                child.params.insert(k.clone(), x);
            }
            for inner in sub.body.iter().filter(|c| c.tokens[0] == ".param") {
                eval_params(inner, &mut child.params)?;
            }
            for inner in sub.body.iter().filter(|c| c.tokens[0] == ".model") {
                let mut m = parse_model(inner, &child.params)?;
                let local = m.name.clone();
                m.name = format!("{}.{local}", e.name);
                m.line = line;
                child.models.insert(local, m.name.clone());
                deck.models.insert(m.name.clone(), m);
            }
            for inner in &sub.body {
                let head = inner.tokens[0].as_str();
                if head == ".param" || head == ".model" {
                    continue;
                }
                if head.starts_with('.') {
                    deck.warnings
                        .push(format!("line {}: directive {head} inside .subckt ignored", inner.line));
                    continue;
                }
                let c = Card {
                    line,
                    tokens: inner.tokens.clone(),
                    raw: inner.raw.clone(),
                };
                expand_element(&c, &e.name, &map, subckts, &child, deck, depth + 1)?;
            }
            return Ok(());
        }
        _ => return err(line, format!("unsupported element '{}'", t[0])),
    }
    deck.elements.push(e);
    Ok(())
}

/// Remaining instance fields: an optional bare number (area or multiplier)
/// followed by `key=value` pairs and flags.
fn instance_rest(
    rest: &[String],
    bare_key: &str,
    params: &BTreeMap<String, f64>,
    line: usize,
    e: &mut Element,
) -> Result<(), ParseError> {
    let mut kv = rest;
    if let Some(first) = rest.first()
        && rest.get(1).map(String::as_str) != Some("=")
        && let Some(v) = parse_value(first)
    {
        e.params.insert(bare_key.to_string(), v);
        kv = &rest[1..];
    }
    key_values(kv, params, line, &mut e.params, &mut e.flags)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_with_suffixes() {
        let cases = [
            ("1k", 1e3),
            ("4.7K", 4.7e3),
            ("10uF", 10e-6),
            ("1meg", 1e6),
            ("2.2Meg", 2.2e6),
            ("1m", 1e-3),
            ("3mil", 3.0 * 25.4e-6),
            ("1e-3", 1e-3),
            ("1.5e+2", 150.0),
            ("-2.5n", -2.5e-9),
            ("100p", 100e-12),
            ("5v", 5.0),
            ("10ohm", 10.0),
            (".5", 0.5),
        ];
        for (s, v) in cases {
            let got = parse_value(s).unwrap_or_else(|| panic!("{s}"));
            assert!((got - v).abs() <= 1e-12 * v.abs(), "{s}: {got} vs {v}");
        }
        assert_eq!(parse_value("abc"), None);
        assert_eq!(parse_value(""), None);
    }

    #[test]
    fn parses_elements_sources_and_analyses() {
        let deck = parse_str(
            "Test deck\n\
             * comment\n\
             V1 in 0 DC 5\n\
             V2 clk 0 PULSE(0 3.3 1n 1n 1n 5u 10u)\n\
             I1 0 n2 sin(0 1m 1k)\n\
             R1 in out 1k ; inline comment\n\
             C1 out 0 1u ic=2\n\
             L1 out\n+ n2 10m\n\
             D1 out 0 dmod 2\n\
             Q1 c b 0 qn\n\
             M1 d g 0 0 nm W=10u L=1u\n\
             E1 a 0 in 0 2\n\
             .model dmod D(is=1e-14 n=1.5)\n\
             .model qn npn bf=200\n\
             .model nm nmos (vto=0.7 kp=100u)\n\
             .op\n\
             .tran 1u 1m uic\n\
             .dc v1 0 5 0.1\n\
             .options reltol=1e-4 method=gear\n\
             .ic v(out)=1\n\
             .control\nrun\n.endc\n\
             .end\n",
        )
        .unwrap();
        assert_eq!(deck.title, "Test deck");
        assert_eq!(deck.elements.len(), 10);
        let v2 = &deck.elements[1];
        assert_eq!(v2.source.as_ref().unwrap().wave.as_ref().unwrap().args.len(), 7);
        let l1 = &deck.elements[5];
        assert_eq!(l1.nodes, vec!["out", "n2"]);
        assert!((l1.value.unwrap() - 0.01).abs() < 1e-15);
        assert_eq!(deck.elements[6].params["area"], 2.0);
        assert_eq!(deck.elements[7].nodes.len(), 3);
        assert!((deck.elements[8].params["w"] - 10e-6).abs() < 1e-18);
        assert_eq!(deck.elements[4].params["ic"], 2.0);
        assert_eq!(deck.models["dmod"].params["n"], 1.5);
        assert_eq!(deck.models["qn"].kind, "npn");
        assert_eq!(deck.analyses.len(), 3);
        assert!(matches!(deck.analyses[1], Analysis::Tran { uic: true, .. }));
        assert_eq!(deck.options["method"], 2.0);
        assert_eq!(deck.initial_conditions["out"], 1.0);
    }

    #[test]
    fn poly_sources() {
        let d = parse_str("t\nG1 0 o POLY(2) a 0 b 0 0 0 0 0 1\nE1 x 0 poly(1) a 0 1 2\n").unwrap();
        assert_eq!(d.elements[0].nodes, ["0", "o", "a", "0", "b", "0"]);
        assert_eq!(d.elements[0].coeffs, [0.0, 0.0, 0.0, 0.0, 1.0]);
        assert_eq!(d.elements[1].coeffs, [1.0, 2.0]);
        assert!(d.elements[1].value.is_none());
        assert!(parse_str("t\nG1 0 o POLY(2) a 0 b 0\n").is_err());
        assert!(parse_str("t\nG1 0 o POLY 2 a 0 1\n").is_err());
    }

    #[test]
    fn flattens_subcircuits_and_params() {
        let deck = parse_str(
            "sub\n\
             .param rval=2k\n\
             .subckt div top bot mid\n\
             R1 top mid {rval}\n\
             R2 mid bot {rval}\n\
             .ends\n\
             X1 in 0 m div\n\
             X2 m 0 m2 div\n\
             V1 in 0 1\n",
        )
        .unwrap();
        let names: Vec<_> = deck.elements.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["x1.r1", "x1.r2", "x2.r1", "x2.r2", "v1"]);
        assert_eq!(deck.elements[1].nodes, vec!["m", "0"]);
        assert_eq!(deck.elements[2].nodes, vec!["m", "m2"]);
        assert_eq!(deck.elements[0].value, Some(2000.0));
    }

    #[test]
    fn param_expressions_subckt_params_and_local_models() {
        let deck = parse_str(
            "p\n\
             .param r0 = 1k  c0={1u/2}\n\
             .param fc = 1/(2*pi*r0*c0) twice = r0*2\n\
             .subckt stage in out params: r=1k gain=2\n\
             .param r2 = {r*gain}\n\
             .model dloc d is=1e-15 n={gain}\n\
             R1 in out {r2}\n\
             D1 out 0 dloc\n\
             .ends\n\
             X1 a b stage params: r={twice} gain=3\n\
             X2 b c stage\n\
             R9 c 0 {fc > 100 ? 1 : 2}\n",
        )
        .unwrap();
        assert!((deck.params["fc"] - 1.0 / (2.0 * std::f64::consts::PI * 1e3 * 0.5e-6)).abs() < 1e-9);
        let r = |n: &str| deck.elements.iter().find(|e| e.name == n).unwrap();
        assert_eq!(r("x1.r1").value, Some(6000.0));
        assert_eq!(r("x2.r1").value, Some(2000.0));
        assert_eq!(r("r9").value, Some(1.0));
        assert_eq!(r("x1.d1").model.as_deref(), Some("x1.dloc"));
        assert_eq!(deck.models["x1.dloc"].params["n"], 3.0);
        assert_eq!(deck.models["x2.dloc"].params["n"], 2.0);
    }

    #[test]
    fn ac_nested_dc_temp_and_new_elements() {
        let deck = parse_str(
            "t\n\
             V1 a 0 DC 1 AC 2 45\n\
             L1 a b 1m\n\
             L2 c 0 4m\n\
             K1 L1 L2 0.9\n\
             S1 b 0 a 0 sw1 on\n\
             W1 c 0 v1 sw2\n\
             .model sw1 sw vt=1\n\
             .model sw2 csw it=1m\n\
             .ac dec 10 1 1meg\n\
             .dc v1 0 5 1 v2 0 1 0.5\n\
             .temp 85\n",
        )
        .unwrap();
        let src = deck.elements[0].source.as_ref().unwrap();
        assert_eq!((src.dc, src.ac, src.ac_phase), (Some(1.0), Some(2.0), 45.0));
        assert_eq!(deck.elements[3].refs, vec!["l1", "l2"]);
        assert_eq!(deck.elements[4].flags, vec!["on"]);
        assert_eq!(deck.elements[5].control.as_deref(), Some("v1"));
        assert!(matches!(
            deck.analyses[0],
            Analysis::Ac {
                sweep: AcSweep::Dec,
                points: 10,
                ..
            }
        ));
        assert!(matches!(&deck.analyses[1], Analysis::Dc { outer: Some(o), .. } if o.source == "v2" && o.stop == 1.0));
        assert_eq!(deck.temp, Some(85.0));
        let f = AcSweep::Dec.frequencies(10, 1.0, 1e6);
        assert_eq!(f.len(), 61);
        assert!((f[60] - 1e6).abs() < 1e-3);
        assert_eq!(AcSweep::Lin.frequencies(5, 0.0, 4.0), vec![0.0, 1.0, 2.0, 3.0, 4.0]);
    }

    #[test]
    fn reports_errors_with_line_numbers() {
        let e = parse_str("t\nR1 a b\n").unwrap_err();
        assert_eq!(e.line, 2);
        let e = parse_str("t\nR1 a b 1k\nX1 a b nosuch\n").unwrap_err();
        assert!(e.message.contains("unknown subcircuit"), "{e}");
        assert!(parse_str("t\n.subckt a p\nR1 p 0 1\n").is_err());
        assert!(parse_str("t\nZ1 a b\n").is_err());
    }
}

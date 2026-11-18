//! SPICE netlist subset parser.
//!
//! Reads the ngspice-compatible subset Ohmforge uses for tests, imports and
//! the `ohmforge run` command: element cards (R C L V I D Q M E G F H X),
//! `.model`, `.subckt`/`.ends` (flattened on parse), `.param` constants,
//! `.op`, `.dc`, `.tran`, `.ic`, `.options`, `.include` and `.end`.
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
    /// AC magnitude (parsed, not yet used).
    pub ac: Option<f64>,
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
    /// Controlling voltage source for F and H elements.
    pub control: Option<String>,
    /// `key=value` instance parameters (`ic`, `w`, `l`, `area`, `m`, ...).
    pub params: BTreeMap<String, f64>,
    /// Flags such as `off`.
    pub flags: Vec<String>,
    /// Source specification for V and I elements.
    pub source: Option<SourceSpec>,
    /// Line number of the card (in the top-level file).
    pub line: usize,
}

impl Element {
    /// The element type letter (`r`, `c`, `q`, ...).
    pub fn letter(&self) -> char {
        self.name.chars().next().unwrap_or('?')
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
    /// `.dc src start stop step`
    Dc {
        /// Swept source name.
        source: String,
        /// First value.
        start: f64,
        /// Last value.
        stop: f64,
        /// Increment.
        step: f64,
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
    /// Non-fatal notes about ignored cards.
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone)]
struct Card {
    line: usize,
    tokens: Vec<String>,
}

#[derive(Debug, Clone)]
struct Subckt {
    ports: Vec<String>,
    body: Vec<Card>,
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
            cards.push(Card { line, tokens });
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

    // Pass 1: parameters, models and subcircuit definitions.
    let mut params: BTreeMap<String, f64> = BTreeMap::new();
    let mut subckts: BTreeMap<String, Subckt> = BTreeMap::new();
    let mut top: Vec<Card> = Vec::new();
    let mut open: Option<(String, Subckt)> = None;
    for card in cards {
        let head = card.tokens[0].as_str();
        if head == ".subckt" {
            if open.is_some() {
                return err(card.line, "nested .subckt definitions are not supported");
            }
            let name = card.tokens.get(1).cloned().ok_or(ParseError {
                line: card.line,
                message: ".subckt needs a name".into(),
            })?;
            let ports = card.tokens[2..]
                .iter()
                .take_while(|t| *t != "params:" && *t != "=")
                .cloned()
                .collect();
            open = Some((
                name,
                Subckt {
                    ports,
                    body: Vec::new(),
                },
            ));
        } else if head == ".ends" {
            let (name, sub) = open.take().ok_or(ParseError {
                line: card.line,
                message: ".ends without .subckt".into(),
            })?;
            subckts.insert(name, sub);
        } else if head == ".model" {
            let m = parse_model(&card, &params)?;
            deck.models.insert(m.name.clone(), m);
        } else if head == ".param" {
            let mut i = 1;
            while i < card.tokens.len() {
                if card.tokens.get(i + 1).map(String::as_str) != Some("=") || i + 2 >= card.tokens.len() {
                    return err(card.line, "expected name=value in .param");
                }
                let v = value_of(&card.tokens[i + 2], &params, card.line)?;
                params.insert(card.tokens[i].clone(), v);
                i += 3;
            }
        } else if let Some((_, sub)) = open.as_mut() {
            sub.body.push(card);
        } else {
            top.push(card);
        }
    }
    if open.is_some() {
        return err(0, "missing .ends");
    }

    // Pass 2: elements (flattening subcircuits) and directives.
    for card in &top {
        let head = card.tokens[0].as_str();
        if head.starts_with('.') {
            parse_directive(card, &params, &mut deck)?;
        } else {
            expand_element(card, "", &BTreeMap::new(), &subckts, &params, &mut deck, 0)?;
        }
    }
    Ok(deck)
}

fn value_of(token: &str, params: &BTreeMap<String, f64>, line: usize) -> Result<f64, ParseError> {
    if let Some(inner) = token.strip_prefix('{').and_then(|t| t.strip_suffix('}')) {
        let key = inner.trim().to_ascii_lowercase();
        return params.get(&key).copied().map_or_else(
            || {
                err(
                    line,
                    format!("unknown parameter {{{key}}} (expressions are not supported yet)"),
                )
            },
            Ok,
        );
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
            if t.len() > 5 {
                deck.warnings.push(format!(
                    "line {line}: nested .dc sweeps are not supported; only the first source is swept"
                ));
            }
            deck.analyses.push(Analysis::Dc {
                source: t.get(1).cloned().ok_or(ParseError {
                    line,
                    message: ".dc needs a source".into(),
                })?,
                start: num(2)?,
                stop: num(3)?,
                step: num(4)?,
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
        ".temp" => deck
            .warnings
            .push(format!("line {line}: .temp ignored; simulations run at 27 °C")),
        ".ac" | ".noise" | ".tf" | ".sens" | ".pz" | ".four" | ".disto" => {
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
                let mag = tokens.get(i + 1).and_then(|t| parse_value(t));
                spec.ac = Some(mag.unwrap_or(1.0));
                i += if mag.is_some() { 2 } else { 1 };
                // Optional phase.
                if mag.is_some() && tokens.get(i).and_then(|t| parse_value(t)).is_some() {
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
    params: &BTreeMap<String, f64>,
    deck: &mut Deck,
    depth: usize,
) -> Result<(), ParseError> {
    let t = &card.tokens;
    let line = card.line;
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
        params: BTreeMap::new(),
        flags: Vec::new(),
        source: None,
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
        'v' | 'i' => {
            need(2)?;
            e.nodes = vec![map_node(&t[1]), map_node(&t[2])];
            e.source = Some(parse_source(&t[3..], params, line)?);
        }
        'd' => {
            need(3)?;
            e.nodes = vec![map_node(&t[1]), map_node(&t[2])];
            e.model = Some(t[3].clone());
            instance_rest(&t[4..], "area", params, line, &mut e)?;
        }
        'q' => {
            need(4)?;
            // Optional substrate node: the model name is the first token that
            // names a known model.
            let model_at = if deck.models.contains_key(&t[4]) || t.len() < 6 {
                4
            } else {
                5
            };
            if !deck.models.contains_key(&t[model_at]) && deck.models.contains_key(&t[4]) {
                return err(line, format!("unknown model for {}", t[0]));
            }
            e.nodes = t[1..model_at].iter().map(|n| map_node(n)).collect();
            e.model = Some(t[model_at].clone());
            instance_rest(&t[model_at + 1..], "area", params, line, &mut e)?;
        }
        'm' => {
            need(5)?;
            e.nodes = t[1..5].iter().map(|n| map_node(n)).collect();
            e.model = Some(t[5].clone());
            instance_rest(&t[6..], "m", params, line, &mut e)?;
        }
        'e' | 'g' => {
            need(5)?;
            e.nodes = t[1..5].iter().map(|n| map_node(n)).collect();
            e.value = Some(value_of(&t[5], params, line)?);
        }
        'f' | 'h' => {
            need(4)?;
            e.nodes = vec![map_node(&t[1]), map_node(&t[2])];
            let ctrl = &t[3];
            e.control = Some(if prefix.is_empty() {
                ctrl.clone()
            } else {
                format!("{prefix}.{ctrl}")
            });
            e.value = Some(value_of(&t[4], params, line)?);
        }
        'x' => {
            if depth > 32 {
                return err(line, "subcircuit nesting too deep (recursive .subckt?)");
            }
            let fields: Vec<&String> = t[1..].iter().take_while(|x| *x != "params:").collect();
            let (sub_name, nodes) = fields.split_last().ok_or(ParseError {
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
            for inner in &sub.body {
                if inner.tokens[0].starts_with('.') {
                    deck.warnings.push(format!(
                        "line {}: directive {} inside .subckt ignored",
                        inner.line, inner.tokens[0]
                    ));
                    continue;
                }
                let c = Card {
                    line,
                    tokens: inner.tokens.clone(),
                };
                expand_element(&c, &e.name, &map, subckts, params, deck, depth + 1)?;
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
    fn reports_errors_with_line_numbers() {
        let e = parse_str("t\nR1 a b\n").unwrap_err();
        assert_eq!(e.line, 2);
        let e = parse_str("t\nR1 a b 1k\nX1 a b nosuch\n").unwrap_err();
        assert!(e.message.contains("unknown subcircuit"), "{e}");
        assert!(parse_str("t\n.subckt a p\nR1 p 0 1\n").is_err());
        assert!(parse_str("t\nZ1 a b\n").is_err());
    }
}

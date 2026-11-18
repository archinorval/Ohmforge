//! Arithmetic expressions for `.param` and `{...}` values.
//!
//! Grammar (case-insensitive): numbers with SPICE suffixes (`4.7k`, `10u`),
//! parameter names, `+ - * /`, `^` or `**` (power, right-associative),
//! unary `+`/`-`, comparisons (`< <= > >= == !=`, giving 1 or 0), the
//! ternary `c ? a : b`, parentheses and the functions `sqrt exp ln log
//! log10 abs sin cos tan asin acos atan sinh cosh tanh floor ceil int
//! round sgn pow pwr min max limit atan2 hypot if`. `pi` and `e` are
//! constants unless redefined.

use std::collections::BTreeMap;

/// Evaluate `src` with parameters from `params`.
pub fn eval(src: &str, params: &BTreeMap<String, f64>) -> Result<f64, String> {
    let tokens = lex(&src.to_ascii_lowercase())?;
    let mut p = Parser {
        t: &tokens,
        i: 0,
        params,
    };
    let v = p.ternary()?;
    if p.i != tokens.len() {
        return Err(format!("unexpected '{}' in expression '{src}'", tokens[p.i].text()));
    }
    Ok(v)
}

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Num(f64),
    Ident(String),
    Op(&'static str),
}

impl Tok {
    fn text(&self) -> String {
        match self {
            Tok::Num(v) => v.to_string(),
            Tok::Ident(s) => s.clone(),
            Tok::Op(o) => (*o).to_string(),
        }
    }
}

fn lex(s: &str) -> Result<Vec<Tok>, String> {
    const OPS: [&str; 18] = [
        "**", "<=", ">=", "==", "!=", "&&", "||", "+", "-", "*", "/", "^", "(", ")", ",", "<", ">", "?",
    ];
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if c == b':' {
            out.push(Tok::Op(":"));
            i += 1;
            continue;
        }
        if c.is_ascii_digit() || (c == b'.' && b.get(i + 1).is_some_and(u8::is_ascii_digit)) {
            // Number: digits, '.', exponent, then suffix letters.
            let start = i;
            while i < b.len() && (b[i].is_ascii_digit() || b[i] == b'.') {
                i += 1;
            }
            if i < b.len()
                && b[i] == b'e'
                && (b.get(i + 1).is_some_and(u8::is_ascii_digit)
                    || (matches!(b.get(i + 1), Some(b'+' | b'-')) && b.get(i + 2).is_some_and(u8::is_ascii_digit)))
            {
                i += 2;
                while i < b.len() && b[i].is_ascii_digit() {
                    i += 1;
                }
            }
            while i < b.len() && (b[i].is_ascii_alphabetic() || b[i] == b'_') {
                i += 1;
            }
            let text = &s[start..i];
            out.push(Tok::Num(
                crate::spice::parse_value(text).ok_or_else(|| format!("bad number '{text}'"))?,
            ));
            continue;
        }
        if c.is_ascii_alphabetic() || c == b'_' {
            let start = i;
            while i < b.len() && (b[i].is_ascii_alphanumeric() || matches!(b[i], b'_' | b'.' | b'#')) {
                i += 1;
            }
            out.push(Tok::Ident(s[start..i].to_string()));
            continue;
        }
        match OPS.iter().find(|op| s[i..].starts_with(**op)) {
            Some(op) => {
                out.push(Tok::Op(op));
                i += op.len();
            }
            None => return Err(format!("unexpected character '{}' in expression", c as char)),
        }
    }
    Ok(out)
}

struct Parser<'a> {
    t: &'a [Tok],
    i: usize,
    params: &'a BTreeMap<String, f64>,
}

impl Parser<'_> {
    fn peek_op(&self, op: &str) -> bool {
        matches!(self.t.get(self.i), Some(Tok::Op(o)) if *o == op)
    }

    fn eat(&mut self, op: &str) -> bool {
        if self.peek_op(op) {
            self.i += 1;
            true
        } else {
            false
        }
    }

    fn expect(&mut self, op: &str) -> Result<(), String> {
        if self.eat(op) {
            Ok(())
        } else {
            Err(format!("expected '{op}' in expression"))
        }
    }

    fn ternary(&mut self) -> Result<f64, String> {
        let c = self.or()?;
        if self.eat("?") {
            let a = self.ternary()?;
            self.expect(":")?;
            let b = self.ternary()?;
            return Ok(if c != 0.0 { a } else { b });
        }
        Ok(c)
    }

    fn or(&mut self) -> Result<f64, String> {
        let mut v = self.and()?;
        while self.eat("||") {
            let r = self.and()?;
            v = f64::from(u8::from(v != 0.0 || r != 0.0));
        }
        Ok(v)
    }

    fn and(&mut self) -> Result<f64, String> {
        let mut v = self.cmp()?;
        while self.eat("&&") {
            let r = self.cmp()?;
            v = f64::from(u8::from(v != 0.0 && r != 0.0));
        }
        Ok(v)
    }

    fn cmp(&mut self) -> Result<f64, String> {
        let mut v = self.sum()?;
        loop {
            let op = ["<=", ">=", "==", "!=", "<", ">"].into_iter().find(|o| self.peek_op(o));
            let Some(op) = op else { return Ok(v) };
            self.i += 1;
            let r = self.sum()?;
            let b = match op {
                "<=" => v <= r,
                ">=" => v >= r,
                "==" => v == r,
                "!=" => v != r,
                "<" => v < r,
                _ => v > r,
            };
            v = f64::from(u8::from(b));
        }
    }

    fn sum(&mut self) -> Result<f64, String> {
        let mut v = self.product()?;
        loop {
            if self.eat("+") {
                v += self.product()?;
            } else if self.eat("-") {
                v -= self.product()?;
            } else {
                return Ok(v);
            }
        }
    }

    fn product(&mut self) -> Result<f64, String> {
        let mut v = self.unary()?;
        loop {
            if self.eat("*") {
                v *= self.unary()?;
            } else if self.eat("/") {
                v /= self.unary()?;
            } else {
                return Ok(v);
            }
        }
    }

    fn unary(&mut self) -> Result<f64, String> {
        if self.eat("-") {
            return Ok(-self.unary()?);
        }
        if self.eat("+") {
            return self.unary();
        }
        self.power()
    }

    fn power(&mut self) -> Result<f64, String> {
        let base = self.atom()?;
        if self.eat("^") || self.eat("**") {
            let exp = self.unary()?;
            return Ok(base.powf(exp));
        }
        Ok(base)
    }

    fn atom(&mut self) -> Result<f64, String> {
        match self.t.get(self.i).cloned() {
            Some(Tok::Num(v)) => {
                self.i += 1;
                Ok(v)
            }
            Some(Tok::Op("(")) => {
                self.i += 1;
                let v = self.ternary()?;
                self.expect(")")?;
                Ok(v)
            }
            Some(Tok::Ident(name)) => {
                self.i += 1;
                if self.eat("(") {
                    let mut args = Vec::new();
                    if !self.eat(")") {
                        loop {
                            args.push(self.ternary()?);
                            if self.eat(")") {
                                break;
                            }
                            self.expect(",")?;
                        }
                    }
                    return call(&name, &args);
                }
                if let Some(v) = self.params.get(&name) {
                    return Ok(*v);
                }
                match name.as_str() {
                    "pi" => Ok(std::f64::consts::PI),
                    "e" => Ok(std::f64::consts::E),
                    _ => Err(format!("unknown parameter '{name}'")),
                }
            }
            Some(t) => Err(format!("unexpected '{}' in expression", t.text())),
            None => Err("expression ends unexpectedly".into()),
        }
    }
}

fn call(name: &str, a: &[f64]) -> Result<f64, String> {
    let n = |k: usize| -> Result<(), String> {
        if a.len() == k {
            Ok(())
        } else {
            Err(format!("{name}() takes {k} argument(s), got {}", a.len()))
        }
    };
    let one = |f: fn(f64) -> f64| -> Result<f64, String> {
        n(1)?;
        Ok(f(a[0]))
    };
    match name {
        "sqrt" => one(f64::sqrt),
        "exp" => one(f64::exp),
        "ln" | "log" => one(f64::ln),
        "log10" => one(f64::log10),
        "abs" => one(f64::abs),
        "sin" => one(f64::sin),
        "cos" => one(f64::cos),
        "tan" => one(f64::tan),
        "asin" => one(f64::asin),
        "acos" => one(f64::acos),
        "atan" => one(f64::atan),
        "sinh" => one(f64::sinh),
        "cosh" => one(f64::cosh),
        "tanh" => one(f64::tanh),
        "floor" => one(f64::floor),
        "ceil" => one(f64::ceil),
        "int" => one(f64::trunc),
        "round" => one(f64::round),
        "sgn" => one(|x| {
            if x > 0.0 {
                1.0
            } else if x < 0.0 {
                -1.0
            } else {
                0.0
            }
        }),
        "pow" | "pwr" => {
            n(2)?;
            Ok(a[0].powf(a[1]))
        }
        "atan2" => {
            n(2)?;
            Ok(a[0].atan2(a[1]))
        }
        "hypot" => {
            n(2)?;
            Ok(a[0].hypot(a[1]))
        }
        "min" => {
            n(2)?;
            Ok(a[0].min(a[1]))
        }
        "max" => {
            n(2)?;
            Ok(a[0].max(a[1]))
        }
        "limit" => {
            n(3)?;
            Ok(a[0].clamp(a[1].min(a[2]), a[1].max(a[2])))
        }
        "if" => {
            n(3)?;
            Ok(if a[0] != 0.0 { a[1] } else { a[2] })
        }
        _ => Err(format!("unknown function '{name}()'")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arithmetic_functions_and_params() {
        let mut p = BTreeMap::new();
        p.insert("r".to_string(), 1000.0);
        p.insert("c".to_string(), 1e-6);
        let cases = [
            ("1+2*3", 7.0),
            ("(1+2)*3", 9.0),
            ("2^3^2", 512.0),
            ("2**3", 8.0),
            ("-2^2", -4.0),
            ("1/(2*pi*r*c)", 1.0 / (2.0 * std::f64::consts::PI * 1e-3)),
            ("4.7k/2", 2350.0),
            ("10u*2", 20e-6),
            ("sqrt(16)+max(1,3)", 7.0),
            ("r > 500 ? 1 : 2", 1.0),
            ("if(r<500, 1, 2)", 2.0),
            ("limit(5, 0, 3)", 3.0),
            ("1e3*1e-3", 1.0),
            ("R*2", 2000.0),
        ];
        for (s, v) in cases {
            let got = eval(s, &p).unwrap_or_else(|e| panic!("{s}: {e}"));
            assert!((got - v).abs() <= 1e-12 * v.abs().max(1.0), "{s}: {got} vs {v}");
        }
        assert!(eval("x+1", &p).unwrap_err().contains("unknown parameter"));
        assert!(eval("1+", &p).is_err());
        assert!(eval("foo(1)", &p).is_err());
    }
}

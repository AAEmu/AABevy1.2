//! The game database's formulas (`unit_formulas`, `wearable_formulas`, ...): small arithmetic
//! expressions over named variables, like `( level * 150 + sta * 12 ) + 100` or
//! `10 + (level - 1) * 2 + if_negative( level - 51, 0, ( level - 50 ) * 8 )`.
//!
//! Numbers, variables (case blind), `+ - * / %`, `^` (power), unary minus, parentheses and the
//! functions the database uses: `abs`, `floor`, `ceiling`, `truncate`, `round`, `sqrt`, `max`, `min`,
//! `avg`, `clamp(x, lo, hi)`, `if_negative(x, a, b)` (a when x < 0, else b), `if_positive(x, a, b)`
//! (a when x > 0), `if_zero(x, a, b)` and `if(cond, a, b)` (a when cond is not 0).

use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq)]
enum Expr {
    Num(f64),
    Var(String),
    Neg(Box<Expr>),
    Bin(char, Box<Expr>, Box<Expr>),
    Call(String, Vec<Expr>),
}

/// A parsed formula.
#[derive(Clone, Debug, PartialEq)]
pub struct Formula(Expr);

impl Formula {
    /// Parses `source`; an error says what is wrong and where.
    pub fn parse(source: &str) -> Result<Formula, String> {
        let mut p = Parser { s: source.as_bytes(), i: 0 };
        let e = p.sum()?;
        p.skip_space();
        if p.i < p.s.len() {
            return Err(format!("unexpected {:?} at {}", p.s[p.i] as char, p.i));
        }
        Ok(Formula(e))
    }

    /// The value with `vars` giving the variables (an unknown variable is 0, like the server's).
    pub fn eval(&self, vars: &HashMap<&str, f64>) -> f64 {
        eval(&self.0, vars)
    }
}

fn eval(e: &Expr, vars: &HashMap<&str, f64>) -> f64 {
    match e {
        Expr::Num(n) => *n,
        Expr::Var(name) => vars.get(name.as_str()).copied().unwrap_or(0.0),
        Expr::Neg(a) => -eval(a, vars),
        Expr::Bin(op, a, b) => {
            let (a, b) = (eval(a, vars), eval(b, vars));
            match op {
                '+' => a + b,
                '-' => a - b,
                '*' => a * b,
                '/' => a / b,
                '%' => a % b,
                _ => a.powf(b),
            }
        }
        Expr::Call(name, args) => {
            let v: Vec<f64> = args.iter().map(|a| eval(a, vars)).collect();
            let arg = |i: usize| v.get(i).copied().unwrap_or(0.0);
            match name.as_str() {
                "abs" => arg(0).abs(),
                "floor" => arg(0).floor(),
                "ceiling" => arg(0).ceil(),
                "truncate" => arg(0).trunc(),
                "round" => arg(0).round_ties_even(),
                "sqrt" => arg(0).sqrt(),
                "loge" => arg(0).ln(),
                "log10" => arg(0).log10(),
                "max" => v.iter().copied().fold(f64::MIN, f64::max),
                "min" => v.iter().copied().fold(f64::MAX, f64::min),
                "avg" if !v.is_empty() => v.iter().sum::<f64>() / v.len() as f64,
                "clamp" => arg(0).max(arg(1)).min(arg(2)),
                "if_negative" => if arg(0) < 0.0 { arg(1) } else { arg(2) },
                "if_positive" => if arg(0) > 0.0 { arg(1) } else { arg(2) },
                "if_zero" => if arg(0) == 0.0 { arg(1) } else { arg(2) },
                "if" => if arg(0) != 0.0 { arg(1) } else { arg(2) },
                _ => 0.0,
            }
        }
    }
}

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
}

impl Parser<'_> {
    fn skip_space(&mut self) {
        while self.s.get(self.i).is_some_and(u8::is_ascii_whitespace) {
            self.i += 1;
        }
    }

    fn eat(&mut self, c: u8) -> bool {
        self.skip_space();
        if self.s.get(self.i) == Some(&c) {
            self.i += 1;
            true
        } else {
            false
        }
    }

    fn sum(&mut self) -> Result<Expr, String> {
        let mut a = self.product()?;
        loop {
            let op = if self.eat(b'+') {
                '+'
            } else if self.eat(b'-') {
                '-'
            } else {
                return Ok(a);
            };
            a = Expr::Bin(op, Box::new(a), Box::new(self.product()?));
        }
    }

    fn product(&mut self) -> Result<Expr, String> {
        let mut a = self.unary()?;
        loop {
            let op = if self.eat(b'*') {
                '*'
            } else if self.eat(b'/') {
                '/'
            } else if self.eat(b'%') {
                '%'
            } else {
                return Ok(a);
            };
            a = Expr::Bin(op, Box::new(a), Box::new(self.unary()?));
        }
    }

    fn unary(&mut self) -> Result<Expr, String> {
        if self.eat(b'-') {
            return Ok(Expr::Neg(Box::new(self.unary()?)));
        }
        if self.eat(b'+') {
            return self.unary();
        }
        self.power()
    }

    /// `a ^ b`, right to left.
    fn power(&mut self) -> Result<Expr, String> {
        let a = self.atom()?;
        if self.eat(b'^') {
            return Ok(Expr::Bin('^', Box::new(a), Box::new(self.unary()?)));
        }
        Ok(a)
    }

    fn atom(&mut self) -> Result<Expr, String> {
        self.skip_space();
        let start = self.i;
        let Some(&c) = self.s.get(self.i) else { return Err("unexpected end".into()) };
        if c == b'(' {
            self.i += 1;
            let e = self.sum()?;
            if !self.eat(b')') {
                return Err(format!("missing ) at {}", self.i));
            }
            return Ok(e);
        }
        if c.is_ascii_digit() || c == b'.' {
            while self.s.get(self.i).is_some_and(|c| c.is_ascii_digit() || *c == b'.') {
                self.i += 1;
            }
            let text = std::str::from_utf8(&self.s[start..self.i]).unwrap_or("");
            return text.parse().map(Expr::Num).map_err(|_| format!("bad number {text:?}"));
        }
        if c.is_ascii_alphabetic() || c == b'_' {
            while self.s.get(self.i).is_some_and(|c| c.is_ascii_alphanumeric() || *c == b'_') {
                self.i += 1;
            }
            let name = String::from_utf8_lossy(&self.s[start..self.i]).to_ascii_lowercase();
            if self.eat(b'(') {
                let mut args = Vec::new();
                if !self.eat(b')') {
                    loop {
                        args.push(self.sum()?);
                        if self.eat(b')') {
                            break;
                        }
                        if !self.eat(b',') {
                            return Err(format!("expected , or ) at {}", self.i));
                        }
                    }
                }
                return Ok(Expr::Call(name, args));
            }
            return Ok(Expr::Var(name));
        }
        Err(format!("unexpected {:?} at {}", c as char, self.i))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(src: &str, vars: &[(&str, f64)]) -> f64 {
        let map: HashMap<&str, f64> = vars.iter().copied().collect();
        Formula::parse(src).unwrap().eval(&map)
    }

    #[test]
    fn the_players_formulas() {
        // level 1, 10 stamina: 150 + 120 + 100
        assert_eq!(run("( level * 150 + sta * 12 ) + 100", &[("level", 1.0), ("sta", 10.0)]), 370.0);
        let stat = "10 + (level - 1) * 2 + if_negative( level - 51, 0, ( level - 50 ) * 8 )";
        assert_eq!(run(stat, &[("level", 1.0)]), 10.0);
        assert_eq!(run(stat, &[("level", 50.0)]), 108.0);
        assert_eq!(run(stat, &[("level", 55.0)]), 118.0 + 40.0);
    }

    #[test]
    fn precedence_and_functions() {
        assert_eq!(run("2 + 3 * 4 ^ 2", &[]), 50.0);
        assert_eq!(run("-2 ^ 2", &[]), -4.0);
        assert_eq!(run("( 935 / 50 * level ) * ( a * b )", &[("level", 10.0), ("a", 2.0), ("b", 3.0)]), 1122.0);
        assert_eq!(run("clamp(25, 10, 20) + min(3, 4, 5) + floor(2.7)", &[]), 25.0);
        assert_eq!(run("if_positive(6 - abs(x), 1, 0)", &[("x", -3.0)]), 1.0);
        assert_eq!(run("Level * 2", &[("level", 4.0)]), 8.0);
        assert!(Formula::parse("1 +").is_err());
        assert!(Formula::parse("(1").is_err());
    }
}

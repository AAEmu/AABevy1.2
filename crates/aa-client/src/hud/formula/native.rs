//! Source39566d00/39567fb0 and binary395676c0..395678d0, neg39567a70, floor39567e70.
use super::*;

pub(super) fn eval(expr: &Expr, vars: &HashMap<&str, f64>) -> Option<f64> {
    let child = |expr| eval(expr, vars).map(|value| f64::from(value as f32));
    Some(match expr {
        Expr::Num(value) => f64::from(*value as f32),
        Expr::Var(name) => f64::from(*vars.get(name.as_str())? as f32),
        Expr::Neg(expr) => -child(expr)?,
        Expr::Bin(op, a, b) => {
            let (a, b) = (child(a)?, child(b)?);
            match op {
                '+' => a + b,
                '-' => a - b,
                '*' => a * b,
                '/' => {
                    if b == 0. {
                        0.
                    } else {
                        a / b
                    }
                }
                '^' => a.powf(b),
                _ => return None,
            }
        }
        Expr::Call(name, args) if name == "floor" && args.len() == 1 => child(&args[0])?.floor(),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_children_literals_and_division_follow_executed_operator_contracts() {
        let vars = HashMap::new();
        assert_eq!(Formula::parse("(16777216+1)-16777216").unwrap().eval_native(&vars), Some(0.));
        assert_eq!(Formula::parse("16777217").unwrap().eval_native(&vars), Some(16777216.));
        assert_eq!(Formula::parse("1 / 0").unwrap().eval_native(&vars), Some(0.));
        assert!(
            Formula::parse("1 / denominator")
                .unwrap()
                .eval_native(&HashMap::from([("denominator", f64::NAN)]))
                .unwrap()
                .is_nan()
        );
        assert_eq!(Formula::parse("floor(2.99)").unwrap().eval_native(&vars), Some(2.));
        assert_eq!(Formula::parse("5 % 2").unwrap().eval_native(&vars), None);
        assert_eq!(Formula::parse("min(1,2)").unwrap().eval_native(&vars), None);
    }

    #[test]
    fn owner5_formulas_match_executed_native_ast_extended_and_final_float_goldens() {
        let vars = HashMap::from([("level", 5.), ("str", 26.)]);
        for (source, extended, final_float, integer) in [
            ("((level ^ 1.3 + level * 3 + 17) * 100) * 100", 401032.8369140625, 401032.84375f32, 401032),
            (
                "(((level *2) / 3 + 19) + (level^2 / 15) + (str * 0.2)) * 1000",
                29200.000762939453,
                29200.0f32,
                29200,
            ),
        ] {
            let value = Formula::parse(source).unwrap().eval_native(&vars).unwrap();
            assert_eq!(value, extended);
            assert_eq!(value as f32, final_float);
            assert_eq!((value as f32) as i32, integer);
        }
    }
}

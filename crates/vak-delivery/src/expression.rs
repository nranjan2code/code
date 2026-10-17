//! The closed expression language of a controls card
//! (docs/design/96-answer-projection-gaps.md, P1): numbers and names,
//! `+ - * / % ^`, parentheses, comparisons (1 when true, 0 when not) and a
//! fixed set of functions. No strings, no calls out, no loops. The client
//! evaluates the same language (`vak-client-ui/src/controlsExpression.ts`);
//! `tests/expression_cases.json` holds both to the same answers.

use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Number(f64),
    Name(String),
    Negate(Box<Expr>),
    Binary(Op, Box<Expr>, Box<Expr>),
    Call(Function, Vec<Expr>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Add,
    Subtract,
    Multiply,
    Divide,
    Remainder,
    Power,
    Less,
    LessOrEqual,
    Greater,
    GreaterOrEqual,
    Equal,
    NotEqual,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Function {
    If,
    Min,
    Max,
    Round,
    Floor,
    Ceil,
    Abs,
    Sqrt,
    Log,
    Exp,
    Sum,
    Avg,
}

const FUNCTIONS: &[(&str, Function)] = &[
    ("if", Function::If),
    ("min", Function::Min),
    ("max", Function::Max),
    ("round", Function::Round),
    ("floor", Function::Floor),
    ("ceil", Function::Ceil),
    ("abs", Function::Abs),
    ("sqrt", Function::Sqrt),
    ("log", Function::Log),
    ("exp", Function::Exp),
    ("sum", Function::Sum),
    ("avg", Function::Avg),
];

/// The function names, for a refusal that lists them.
pub fn function_names() -> Vec<&'static str> {
    FUNCTIONS.iter().map(|(name, _)| *name).collect()
}

impl Function {
    /// How many arguments it takes: at least, and at most when bounded.
    fn arity(self) -> (usize, Option<usize>) {
        match self {
            Function::If => (3, Some(3)),
            Function::Round => (1, Some(2)),
            Function::Min | Function::Max | Function::Sum | Function::Avg => (1, None),
            _ => (1, Some(1)),
        }
    }
}

/// Why an expression cannot be read, and where (a character offset).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub at: usize,
    pub message: String,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} at character {}", self.message, self.at + 1)
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Number(f64),
    Name(String),
    Symbol(&'static str),
}

/// The name that ends just before `at`, if what ends there is a name and
/// not the digits of a number.
fn name_before(chars: &[char], at: usize) -> Option<String> {
    let start = chars[..at]
        .iter()
        .rposition(|c| !(c.is_ascii_alphanumeric() || *c == '_'))
        .map_or(0, |found| found + 1);
    let word = &chars[start..at];
    word.first()
        .is_some_and(|first| first.is_ascii_alphabetic() || *first == '_')
        .then(|| word.iter().collect())
}

fn tokens(source: &str) -> Result<Vec<(usize, Token)>, ParseError> {
    const SYMBOLS: &[&str] = &[
        "<=", ">=", "==", "!=", "+", "-", "*", "/", "%", "^", "(", ")", ",", "<", ">",
    ];
    let chars: Vec<char> = source.chars().collect();
    let mut out = Vec::new();
    let mut at = 0;
    while at < chars.len() {
        let c = chars[at];
        if c.is_whitespace() {
            at += 1;
        } else if let Some(name) = (c == '.').then(|| name_before(&chars, at)).flatten() {
            return Err(ParseError {
                at,
                message: format!("a name has no parts after a `.`: write `{name}`"),
            });
        } else if c.is_ascii_digit() || c == '.' {
            let start = at;
            while at < chars.len() && (chars[at].is_ascii_digit() || chars[at] == '.') {
                at += 1;
            }
            let text: String = chars[start..at].iter().collect();
            let number = text.parse::<f64>().map_err(|_| ParseError {
                at: start,
                message: format!("`{text}` is not a number"),
            })?;
            out.push((start, Token::Number(number)));
        } else if c.is_ascii_alphabetic() || c == '_' {
            let start = at;
            while at < chars.len() && (chars[at].is_ascii_alphanumeric() || chars[at] == '_') {
                at += 1;
            }
            out.push((start, Token::Name(chars[start..at].iter().collect())));
        } else {
            let rest: String = chars[at..chars.len().min(at + 2)].iter().collect();
            let Some(symbol) = SYMBOLS.iter().find(|symbol| rest.starts_with(**symbol)) else {
                let instead = match c {
                    '?' | ':' => ": choose with if(condition, then, else)",
                    '\'' | '"' => ": there is no text; compare a choice by its option's number",
                    '=' => {
                        ": an expression has no assignment, so write only what follows `=`; compare with =="
                    }
                    _ => "",
                };
                return Err(ParseError {
                    at,
                    message: format!("`{c}` is not part of the language{instead}"),
                });
            };
            out.push((at, Token::Symbol(symbol)));
            at += symbol.chars().count();
        }
    }
    Ok(out)
}

struct Parser {
    tokens: Vec<(usize, Token)>,
    next: usize,
    end: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.next).map(|(_, token)| token)
    }

    fn at(&self) -> usize {
        self.tokens.get(self.next).map_or(self.end, |(at, _)| *at)
    }

    fn eat(&mut self, symbol: &str) -> bool {
        if matches!(self.peek(), Some(Token::Symbol(found)) if *found == symbol) {
            self.next += 1;
            true
        } else {
            false
        }
    }

    fn fail<T>(&self, message: impl Into<String>) -> Result<T, ParseError> {
        Err(ParseError {
            at: self.at(),
            message: message.into(),
        })
    }

    fn comparison(&mut self) -> Result<Expr, ParseError> {
        let left = self.additive()?;
        for (symbol, op) in [
            ("<=", Op::LessOrEqual),
            (">=", Op::GreaterOrEqual),
            ("==", Op::Equal),
            ("!=", Op::NotEqual),
            ("<", Op::Less),
            (">", Op::Greater),
        ] {
            if self.eat(symbol) {
                let right = self.additive()?;
                return Ok(Expr::Binary(op, Box::new(left), Box::new(right)));
            }
        }
        Ok(left)
    }

    fn additive(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.multiplicative()?;
        loop {
            let op = if self.eat("+") {
                Op::Add
            } else if self.eat("-") {
                Op::Subtract
            } else {
                return Ok(left);
            };
            let right = self.multiplicative()?;
            left = Expr::Binary(op, Box::new(left), Box::new(right));
        }
    }

    fn multiplicative(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.unary()?;
        loop {
            let op = if self.eat("*") {
                Op::Multiply
            } else if self.eat("/") {
                Op::Divide
            } else if self.eat("%") {
                Op::Remainder
            } else {
                return Ok(left);
            };
            let right = self.unary()?;
            left = Expr::Binary(op, Box::new(left), Box::new(right));
        }
    }

    fn unary(&mut self) -> Result<Expr, ParseError> {
        if self.eat("-") {
            return Ok(Expr::Negate(Box::new(self.unary()?)));
        }
        if self.eat("+") {
            return self.unary();
        }
        self.power()
    }

    fn power(&mut self) -> Result<Expr, ParseError> {
        let base = self.primary()?;
        if self.eat("^") {
            let exponent = self.unary()?;
            return Ok(Expr::Binary(Op::Power, Box::new(base), Box::new(exponent)));
        }
        Ok(base)
    }

    fn primary(&mut self) -> Result<Expr, ParseError> {
        let at = self.at();
        match self.peek().cloned() {
            Some(Token::Number(number)) => {
                self.next += 1;
                Ok(Expr::Number(number))
            }
            Some(Token::Name(name)) => {
                self.next += 1;
                if !self.eat("(") {
                    return Ok(Expr::Name(name));
                }
                let Some(function) = FUNCTIONS
                    .iter()
                    .find(|(known, _)| *known == name)
                    .map(|(_, function)| *function)
                else {
                    return Err(ParseError {
                        at,
                        message: format!(
                            "`{name}` is not a function; the functions are {}",
                            function_names().join(", ")
                        ),
                    });
                };
                let mut arguments = Vec::new();
                if !self.eat(")") {
                    loop {
                        arguments.push(self.comparison()?);
                        if self.eat(")") {
                            break;
                        }
                        if !self.eat(",") {
                            return self.fail("expected `,` or `)`");
                        }
                    }
                }
                let (least, most) = function.arity();
                if arguments.len() < least || most.is_some_and(|most| arguments.len() > most) {
                    let wanted = match most {
                        Some(most) if most == least => format!("{least}"),
                        Some(most) => format!("{least} or {most}"),
                        None => format!("at least {least}"),
                    };
                    return Err(ParseError {
                        at,
                        message: format!("`{name}` takes {wanted} arguments"),
                    });
                }
                Ok(Expr::Call(function, arguments))
            }
            Some(Token::Symbol("(")) => {
                self.next += 1;
                let inner = self.comparison()?;
                if !self.eat(")") {
                    return self.fail("expected `)`");
                }
                Ok(inner)
            }
            Some(Token::Symbol(symbol)) => self.fail(format!("unexpected `{symbol}`")),
            None => self.fail("the expression ends too soon"),
        }
    }
}

/// Read an expression.
pub fn parse(source: &str) -> Result<Expr, ParseError> {
    let tokens = tokens(source)?;
    let mut parser = Parser {
        tokens,
        next: 0,
        end: source.chars().count(),
    };
    let expr = parser.comparison()?;
    if parser.next < parser.tokens.len() {
        return parser.fail("unexpected text after the expression");
    }
    Ok(expr)
}

/// The names an expression reads, in order of first use.
pub fn names(expr: &Expr) -> Vec<String> {
    fn walk(expr: &Expr, out: &mut Vec<String>) {
        match expr {
            Expr::Number(_) => {}
            Expr::Name(name) => {
                if !out.contains(name) {
                    out.push(name.clone());
                }
            }
            Expr::Negate(inner) => walk(inner, out),
            Expr::Binary(_, left, right) => {
                walk(left, out);
                walk(right, out);
            }
            Expr::Call(_, arguments) => arguments.iter().for_each(|argument| walk(argument, out)),
        }
    }
    let mut out = Vec::new();
    walk(expr, &mut out);
    out
}

/// The expression's value, or why it has none (an unknown name, a division
/// by zero, a result that is not a finite number).
pub fn evaluate(expr: &Expr, values: &BTreeMap<String, f64>) -> Result<f64, String> {
    let value = match expr {
        Expr::Number(number) => *number,
        Expr::Name(name) => *values
            .get(name)
            .ok_or_else(|| format!("`{name}` is not an input or an earlier output"))?,
        Expr::Negate(inner) => -evaluate(inner, values)?,
        Expr::Binary(op, left, right) => {
            let (a, b) = (evaluate(left, values)?, evaluate(right, values)?);
            let truth = |holds: bool| if holds { 1.0 } else { 0.0 };
            match op {
                Op::Add => a + b,
                Op::Subtract => a - b,
                Op::Multiply => a * b,
                Op::Divide | Op::Remainder if b == 0.0 => return Err("division by zero".into()),
                Op::Divide => a / b,
                Op::Remainder => a % b,
                Op::Power => a.powf(b),
                Op::Less => truth(a < b),
                Op::LessOrEqual => truth(a <= b),
                Op::Greater => truth(a > b),
                Op::GreaterOrEqual => truth(a >= b),
                Op::Equal => truth(a == b),
                Op::NotEqual => truth(a != b),
            }
        }
        Expr::Call(Function::If, arguments) => {
            if evaluate(&arguments[0], values)? != 0.0 {
                evaluate(&arguments[1], values)?
            } else {
                evaluate(&arguments[2], values)?
            }
        }
        Expr::Call(function, arguments) => {
            let args = arguments
                .iter()
                .map(|argument| evaluate(argument, values))
                .collect::<Result<Vec<_>, _>>()?;
            match function {
                Function::Min => args.iter().copied().fold(f64::INFINITY, f64::min),
                Function::Max => args.iter().copied().fold(f64::NEG_INFINITY, f64::max),
                Function::Sum => args.iter().sum(),
                Function::Avg => args.iter().sum::<f64>() / args.len() as f64,
                Function::Round => {
                    let places = args.get(1).copied().unwrap_or(0.0).trunc();
                    let scale = 10_f64.powf(places);
                    (args[0] * scale).round() / scale
                }
                Function::Floor => args[0].floor(),
                Function::Ceil => args[0].ceil(),
                Function::Abs => args[0].abs(),
                Function::Sqrt => args[0].sqrt(),
                Function::Log => args[0].ln(),
                Function::Exp => args[0].exp(),
                // Taken lazily in the arm above.
                Function::If => args[1],
            }
        }
    };
    if value.is_finite() {
        Ok(value)
    } else {
        Err("the result is not a finite number".into())
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// A refusal says what to write instead.
    #[test]
    fn a_refusal_names_the_way_the_language_says_it() {
        let ternary = parse("a == 1 ? b : c").expect_err("refused").to_string();
        assert!(ternary.contains("if(condition, then, else)"), "{ternary}");
        let text = parse("kind == 'Miles'").expect_err("refused").to_string();
        assert!(text.contains("option's number"), "{text}");
    }

    /// The table the client's evaluator answers too.
    #[test]
    fn every_shared_case_has_its_answer() {
        let cases: Vec<serde_json::Value> =
            serde_json::from_str(include_str!("../tests/expression_cases.json"))
                .expect("the shared table is JSON");
        assert!(!cases.is_empty());
        for case in cases {
            let source = case["expr"].as_str().expect("expr");
            let values: BTreeMap<String, f64> = case["values"]
                .as_object()
                .map(|object| {
                    object
                        .iter()
                        .filter_map(|(name, value)| Some((name.clone(), value.as_f64()?)))
                        .collect()
                })
                .unwrap_or_default();
            let answer = parse(source)
                .map_err(|error| error.to_string())
                .and_then(|expr| evaluate(&expr, &values));
            match (
                case.get("value").and_then(serde_json::Value::as_f64),
                answer,
            ) {
                (Some(wanted), Ok(got)) => {
                    assert!(
                        (wanted - got).abs() < 1e-9,
                        "{source}: {got}, wanted {wanted}"
                    )
                }
                (None, Err(_)) => {}
                (wanted, got) => panic!("{source}: got {got:?}, wanted {wanted:?}"),
            }
        }
    }
}

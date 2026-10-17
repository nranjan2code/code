/**
 * The closed expression language of a controls card (docs/design/96, P1),
 * the same as the host's (`vak-delivery/src/expression.rs`): numbers and
 * names, `+ - * / % ^`, parentheses, comparisons (1 when true, 0 when not)
 * and a fixed set of functions. `vak-delivery/tests/expression_cases.json`
 * holds both evaluators to the same answers.
 */

type Expr =
  | { type: "number"; value: number }
  | { type: "name"; name: string }
  | { type: "negate"; inner: Expr }
  | { type: "binary"; op: string; left: Expr; right: Expr }
  | { type: "call"; fn: string; args: Expr[] };

const ARITY: Record<string, [number, number | null]> = {
  if: [3, 3], min: [1, null], max: [1, null], round: [1, 2], floor: [1, 1], ceil: [1, 1],
  abs: [1, 1], sqrt: [1, 1], log: [1, 1], exp: [1, 1], sum: [1, null], avg: [1, null],
};

const SYMBOLS = ["<=", ">=", "==", "!=", "+", "-", "*", "/", "%", "^", "(", ")", ",", "<", ">"];

type Token = { at: number; number?: number; name?: string; symbol?: string };

function tokenize(source: string): Token[] {
  const out: Token[] = [];
  let at = 0;
  while (at < source.length) {
    const c = source[at];
    if (/\s/.test(c)) { at += 1; continue; }
    if (/[0-9.]/.test(c)) {
      const start = at;
      while (at < source.length && /[0-9.]/.test(source[at])) at += 1;
      const text = source.slice(start, at);
      if (!/^(\d+\.?\d*|\.\d+)$/.test(text)) throw new Error(`\`${text}\` is not a number`);
      out.push({ at: start, number: Number(text) });
      continue;
    }
    if (/[A-Za-z_]/.test(c)) {
      const start = at;
      while (at < source.length && /[A-Za-z0-9_]/.test(source[at])) at += 1;
      out.push({ at: start, name: source.slice(start, at) });
      continue;
    }
    const symbol = SYMBOLS.find((candidate) => source.startsWith(candidate, at));
    if (!symbol) throw new Error(`\`${c}\` is not part of the language`);
    out.push({ at, symbol });
    at += symbol.length;
  }
  return out;
}

export function parseExpression(source: string): Expr {
  const tokens = tokenize(source);
  let next = 0;
  const peek = () => tokens[next];
  const eat = (symbol: string) => {
    if (peek()?.symbol === symbol) { next += 1; return true; }
    return false;
  };
  const comparison = (): Expr => {
    const left = additive();
    for (const op of ["<=", ">=", "==", "!=", "<", ">"]) {
      if (eat(op)) return { type: "binary", op, left, right: additive() };
    }
    return left;
  };
  const additive = (): Expr => {
    let left = multiplicative();
    for (;;) {
      const op = eat("+") ? "+" : eat("-") ? "-" : null;
      if (!op) return left;
      left = { type: "binary", op, left, right: multiplicative() };
    }
  };
  const multiplicative = (): Expr => {
    let left = unary();
    for (;;) {
      const op = eat("*") ? "*" : eat("/") ? "/" : eat("%") ? "%" : null;
      if (!op) return left;
      left = { type: "binary", op, left, right: unary() };
    }
  };
  const unary = (): Expr => {
    if (eat("-")) return { type: "negate", inner: unary() };
    if (eat("+")) return unary();
    const base = primary();
    if (eat("^")) return { type: "binary", op: "^", left: base, right: unary() };
    return base;
  };
  const primary = (): Expr => {
    const token = peek();
    if (!token) throw new Error("the expression ends too soon");
    next += 1;
    if (token.number !== undefined) return { type: "number", value: token.number };
    if (token.name !== undefined) {
      if (!eat("(")) return { type: "name", name: token.name };
      const arity = ARITY[token.name];
      if (!arity) throw new Error(`\`${token.name}\` is not a function`);
      const args: Expr[] = [];
      if (!eat(")")) {
        for (;;) {
          args.push(comparison());
          if (eat(")")) break;
          if (!eat(",")) throw new Error("expected `,` or `)`");
        }
      }
      if (args.length < arity[0] || (arity[1] !== null && args.length > arity[1])) {
        throw new Error(`\`${token.name}\` takes a different number of arguments`);
      }
      return { type: "call", fn: token.name, args };
    }
    if (token.symbol === "(") {
      const inner = comparison();
      if (!eat(")")) throw new Error("expected `)`");
      return inner;
    }
    throw new Error(`unexpected \`${token.symbol}\``);
  };
  const expr = comparison();
  if (next < tokens.length) throw new Error("unexpected text after the expression");
  return expr;
}

export function evaluateExpression(expr: Expr, values: Record<string, number>): number {
  const value = ((): number => {
    switch (expr.type) {
      case "number": return expr.value;
      case "name": {
        if (!(expr.name in values)) throw new Error(`\`${expr.name}\` is not an input or an earlier output`);
        return values[expr.name];
      }
      case "negate": return -evaluateExpression(expr.inner, values);
      case "binary": {
        const a = evaluateExpression(expr.left, values);
        const b = evaluateExpression(expr.right, values);
        const truth = (holds: boolean) => (holds ? 1 : 0);
        switch (expr.op) {
          case "+": return a + b;
          case "-": return a - b;
          case "*": return a * b;
          case "/": if (b === 0) throw new Error("division by zero"); return a / b;
          case "%": if (b === 0) throw new Error("division by zero"); return a % b;
          case "^": return Math.pow(a, b);
          case "<": return truth(a < b);
          case "<=": return truth(a <= b);
          case ">": return truth(a > b);
          case ">=": return truth(a >= b);
          case "==": return truth(a === b);
          default: return truth(a !== b);
        }
      }
      case "call": {
        if (expr.fn === "if") {
          return evaluateExpression(expr.args[0], values) !== 0
            ? evaluateExpression(expr.args[1], values)
            : evaluateExpression(expr.args[2], values);
        }
        const args = expr.args.map((arg) => evaluateExpression(arg, values));
        switch (expr.fn) {
          case "min": return Math.min(...args);
          case "max": return Math.max(...args);
          case "sum": return args.reduce((total, x) => total + x, 0);
          case "avg": return args.reduce((total, x) => total + x, 0) / args.length;
          case "round": {
            const scale = Math.pow(10, Math.trunc(args[1] ?? 0));
            // Half away from zero, as the host rounds.
            return (Math.sign(args[0]) * Math.round(Math.abs(args[0]) * scale)) / scale;
          }
          case "floor": return Math.floor(args[0]);
          case "ceil": return Math.ceil(args[0]);
          case "abs": return Math.abs(args[0]);
          case "sqrt": return Math.sqrt(args[0]);
          case "log": return Math.log(args[0]);
          default: return Math.exp(args[0]);
        }
      }
    }
  })();
  if (!Number.isFinite(value)) throw new Error("the result is not a finite number");
  return value;
}

/** Days since 1970-01-01 for `YYYY-MM-DD`, as the host counts them. */
export function daysOf(date: string): number | undefined {
  const match = /^(\d{4})-(\d{2})-(\d{2})$/.exec(date);
  if (!match) return undefined;
  const [year, month, day] = [Number(match[1]), Number(match[2]), Number(match[3])];
  if (month < 1 || month > 12 || day < 1 || day > 31) return undefined;
  return Math.round(Date.UTC(year, month - 1, day) / 86_400_000);
}

//! The decoy maths shown on the closed gate (`FEATURE_QUEUE.md` Q1, decision D14).
//!
//! Two or three whole numbers from 1 to 100, the operators `+ - * /`, a bracket around every
//! nested operation, and a result that is a whole number from 1 to 999. Everything is **built
//! forwards from exact integer arithmetic**: a division is made by choosing the divisor and the
//! quotient first and multiplying them to get the dividend, a subtraction by choosing the larger
//! number first, a product only from factors whose product fits. Nothing is ever evaluated from
//! text, no floating point is used anywhere, and no number or intermediate value falls outside
//! 1..=999. The webview only ever receives the text; the answer stays in Rust.

use crate::crypto::rng;
use zeroize::{Zeroize, ZeroizeOnDrop};

/// Source of uniform random numbers. Production uses the OS CSPRNG; the tests plug in a seedable
/// generator so the property tests are reproducible.
pub trait Rand {
    /// A uniformly distributed number in `0..n` (`n > 0`).
    fn below(&mut self, n: u32) -> u32;
}

/// The operating system's CSPRNG (rejection sampling, so there is no modulo bias).
pub struct OsRand;

impl Rand for OsRand {
    fn below(&mut self, n: u32) -> u32 {
        debug_assert!(n > 0);
        if n <= 1 {
            return 0;
        }
        // Largest multiple of `n` that fits in a u32; anything above it is re-drawn.
        let zone = u32::MAX - (u32::MAX % n);
        loop {
            let mut b = [0u8; 4];
            if rng::fill(&mut b).is_err() {
                // The OS RNG failing is not expected; a decoy sum does not need to be secret, so
                // fall back to a fixed value instead of panicking.
                return 0;
            }
            let v = u32::from_le_bytes(b);
            if v < zone {
                return v % n;
            }
        }
    }
}

/// One equation and its answer. Both are wiped when dropped.
#[derive(Clone, PartialEq, Eq, Zeroize, ZeroizeOnDrop)]
pub struct Equation {
    pub text: String,
    pub answer: u32,
}

impl core::fmt::Debug for Equation {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("Equation(<redacted>)")
    }
}

pub const MAX_RESULT: u32 = 999;
pub const MAX_OPERAND: u32 = 100;

enum Node {
    Leaf(u32),
    Op(char, Box<Node>, Box<Node>),
}

struct Built {
    node: Node,
    value: u32,
}

fn show(node: &Node, top: bool, out: &mut String) {
    match node {
        Node::Leaf(n) => out.push_str(&n.to_string()),
        Node::Op(op, l, r) => {
            if !top {
                out.push('(');
            }
            show(l, false, out);
            out.push(' ');
            out.push(*op);
            out.push(' ');
            show(r, false, out);
            if !top {
                out.push(')');
            }
        }
    }
}

/// `lo..=hi`, both inclusive (`lo <= hi`).
fn range(rng: &mut dyn Rand, lo: u32, hi: u32) -> u32 {
    debug_assert!(lo <= hi);
    lo + rng.below(hi - lo + 1)
}

fn leaf(rng: &mut dyn Rand) -> u32 {
    range(rng, 1, MAX_OPERAND)
}

fn op_char(i: u32) -> char {
    ['+', '-', '*', '/'][(i % 4) as usize]
}

/// Two numbers joined by `op`; always possible.
fn pair(rng: &mut dyn Rand, op: char) -> Built {
    let (a, b, value) = match op {
        '+' => {
            let (a, b) = (leaf(rng), leaf(rng));
            (a, b, a + b)
        }
        '-' => {
            // the larger number first: the result is at least 1
            let a = range(rng, 2, MAX_OPERAND);
            let b = range(rng, 1, a - 1);
            (a, b, a - b)
        }
        '*' => {
            let a = range(rng, 2, MAX_OPERAND);
            let b = range(rng, 2, (MAX_RESULT / a).min(MAX_OPERAND));
            (a, b, a * b)
        }
        _ => {
            // divisor and quotient first, the dividend is their product (at most 100)
            let b = range(rng, 2, 50);
            let q = range(rng, 2, MAX_OPERAND / b);
            (b * q, b, q)
        }
    };
    Built {
        node: Node::Op(op, Box::new(Node::Leaf(a)), Box::new(Node::Leaf(b))),
        value,
    }
}

/// Join an existing sub-expression `inner` (value `v`) with one more number by `op`, on a random
/// side. `None` when no whole-number result in range exists for this operator and value.
fn with_leaf(rng: &mut dyn Rand, inner: Built, op: char) -> Option<Built> {
    let v = inner.value;
    let inner_left = rng.below(2) == 0;
    let (z, value, inner_left) = match op {
        '+' => {
            let room = (MAX_RESULT - v).min(MAX_OPERAND);
            if room == 0 {
                return None;
            }
            let z = range(rng, 1, room);
            (z, v + z, inner_left)
        }
        '-' => {
            if inner_left {
                if v < 2 {
                    return None;
                }
                let z = range(rng, 1, (v - 1).min(MAX_OPERAND));
                (z, v - z, true)
            } else {
                if v >= MAX_OPERAND {
                    return None;
                }
                let z = range(rng, v + 1, MAX_OPERAND);
                (z, z - v, false)
            }
        }
        '*' => {
            let max = (MAX_RESULT / v).min(MAX_OPERAND);
            if max < 2 {
                return None;
            }
            let z = range(rng, 2, max);
            (z, v * z, inner_left)
        }
        _ => {
            if inner_left {
                // inner / z: z must be a proper divisor of v that is a valid operand
                let divisors: Vec<u32> = (2..=MAX_OPERAND.min(v.saturating_sub(1)))
                    .filter(|d| v.is_multiple_of(*d))
                    .collect();
                if divisors.is_empty() {
                    return None;
                }
                let z = divisors[rng.below(divisors.len() as u32) as usize];
                (z, v / z, true)
            } else {
                // z / inner: the dividend is inner * quotient (at most 100)
                if !(2..=MAX_OPERAND / 2).contains(&v) {
                    return None;
                }
                let q = range(rng, 2, MAX_OPERAND / v);
                (v * q, q, false)
            }
        }
    };
    let (l, r) = if inner_left {
        (inner.node, Node::Leaf(z))
    } else {
        (Node::Leaf(z), inner.node)
    };
    Some(Built {
        node: Node::Op(op, Box::new(l), Box::new(r)),
        value,
    })
}

/// Three numbers: an inner pair of two numbers joined with a third. A random inner pair and outer
/// operator are drawn until the outer step has a whole-number result in range; that happens within
/// a try or two for every operator.
fn three_numbers(rng: &mut dyn Rand) -> Built {
    for _ in 0..256 {
        let inner_op = op_char(rng.below(4));
        let inner = pair(rng, inner_op);
        let outer_op = op_char(rng.below(4));
        if let Some(built) = with_leaf(rng, inner, outer_op) {
            return built;
        }
    }
    // Not reachable in practice; keeps the function total without a panic.
    pair(rng, '+')
}

/// One new equation: half of them with two numbers, half with three.
pub fn generate(rng: &mut dyn Rand) -> Equation {
    let built = if rng.below(2) == 0 {
        three_numbers(rng)
    } else {
        let op = op_char(rng.below(4));
        pair(rng, op)
    };
    debug_assert!((1..=MAX_RESULT).contains(&built.value));
    let mut text = String::new();
    show(&built.node, true, &mut text);
    Equation {
        text,
        answer: built.value,
    }
}

#[cfg(test)]
pub(crate) mod testing {
    use super::Rand;

    /// SplitMix64: tiny, seedable and good enough to drive property tests reproducibly.
    pub struct SeededRand(pub u64);

    impl SeededRand {
        fn next(&mut self) -> u64 {
            self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = self.0;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^ (z >> 31)
        }
    }

    impl Rand for SeededRand {
        fn below(&mut self, n: u32) -> u32 {
            (self.next() >> 32) as u32 % n.max(1)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testing::SeededRand;
    use super::*;

    // ---- an independent evaluator: parses the rendered text with the usual precedence rules and
    // checks exactness on its own, sharing no code with the generator.

    #[derive(Debug, PartialEq)]
    enum Tok {
        Num(i64),
        Sym(char),
    }

    fn lex(s: &str) -> Vec<Tok> {
        let mut out = Vec::new();
        let mut chars = s.chars().peekable();
        while let Some(&c) = chars.peek() {
            if c.is_ascii_digit() {
                let mut n = 0i64;
                while let Some(&d) = chars.peek() {
                    if let Some(v) = d.to_digit(10) {
                        n = n * 10 + i64::from(v);
                        chars.next();
                    } else {
                        break;
                    }
                }
                out.push(Tok::Num(n));
            } else if c == ' ' {
                chars.next();
            } else {
                assert!("+-*/()".contains(c), "unexpected character {c:?} in {s:?}");
                out.push(Tok::Sym(c));
                chars.next();
            }
        }
        out
    }

    struct Parser {
        toks: Vec<Tok>,
        pos: usize,
        numbers: Vec<i64>,
        /// every intermediate result, to check the 1..=999 rule step by step
        steps: Vec<i64>,
    }

    impl Parser {
        fn peek(&self) -> Option<&Tok> {
            self.toks.get(self.pos)
        }
        fn expr(&mut self) -> i64 {
            let mut v = self.term();
            while let Some(Tok::Sym(c @ ('+' | '-'))) = self.peek() {
                let c = *c;
                self.pos += 1;
                let r = self.term();
                v = if c == '+' { v + r } else { v - r };
                self.steps.push(v);
            }
            v
        }
        fn term(&mut self) -> i64 {
            let mut v = self.atom();
            while let Some(Tok::Sym(c @ ('*' | '/'))) = self.peek() {
                let c = *c;
                self.pos += 1;
                let r = self.atom();
                if c == '*' {
                    v *= r;
                } else {
                    assert!(r != 0 && v % r == 0, "inexact division {v} / {r}");
                    v /= r;
                }
                self.steps.push(v);
            }
            v
        }
        fn atom(&mut self) -> i64 {
            match self.toks.get(self.pos) {
                Some(Tok::Num(n)) => {
                    let n = *n;
                    self.pos += 1;
                    self.numbers.push(n);
                    n
                }
                Some(Tok::Sym('(')) => {
                    self.pos += 1;
                    let v = self.expr();
                    assert_eq!(self.toks.get(self.pos), Some(&Tok::Sym(')')));
                    self.pos += 1;
                    v
                }
                other => panic!("unexpected token {other:?}"),
            }
        }
    }

    fn evaluate(text: &str) -> (i64, Vec<i64>, Vec<i64>) {
        let mut p = Parser {
            toks: lex(text),
            pos: 0,
            numbers: Vec::new(),
            steps: Vec::new(),
        };
        let v = p.expr();
        assert_eq!(p.pos, p.toks.len(), "trailing input in {text:?}");
        (v, p.numbers, p.steps)
    }

    #[test]
    fn thousands_of_seeds_obey_every_rule() {
        let mut three = 0u32;
        let mut ops = std::collections::BTreeSet::new();
        for seed in 0..6000u64 {
            let mut rng = SeededRand(seed);
            let e = generate(&mut rng);
            let (value, numbers, steps) = evaluate(&e.text);
            assert_eq!(value, i64::from(e.answer), "{} (seed {seed})", e.text);
            assert!((1..=999).contains(&value), "{} = {value}", e.text);
            assert!(
                (2..=3).contains(&numbers.len()),
                "{} has {} numbers",
                e.text,
                numbers.len()
            );
            assert!(
                numbers.iter().all(|n| (1..=100).contains(n)),
                "operand out of range in {}",
                e.text
            );
            assert!(
                steps.iter().all(|s| (1..=999).contains(s)),
                "an intermediate value left 1..=999 in {}: {steps:?}",
                e.text
            );
            if numbers.len() == 3 {
                three += 1;
            }
            for c in e.text.chars().filter(|c| "+-*/".contains(*c)) {
                ops.insert(c);
            }
        }
        assert!(
            three > 2000 && three < 4000,
            "about half have three numbers, got {three} of 6000"
        );
        assert_eq!(ops.len(), 4, "all four operators appear: {ops:?}");
    }

    #[test]
    fn brackets_surround_every_nested_operation_and_only_those() {
        for seed in 0..500u64 {
            let e = generate(&mut SeededRand(seed));
            let opens = e.text.matches('(').count();
            let closes = e.text.matches(')').count();
            assert_eq!(opens, closes, "{}", e.text);
            let numbers = e
                .text
                .split(|c: char| !c.is_ascii_digit())
                .filter(|s| !s.is_empty())
                .count();
            // two numbers: no bracket; three numbers: exactly one bracket pair
            assert_eq!(opens, numbers - 2, "{}", e.text);
        }
    }

    #[test]
    fn exact_division_never_leaves_a_remainder() {
        let mut seen_division = 0;
        for seed in 0..4000u64 {
            let e = generate(&mut SeededRand(seed ^ 0xD1F));
            if e.text.contains('/') {
                seen_division += 1;
                // `evaluate` asserts exactness on every division
                evaluate(&e.text);
            }
        }
        assert!(seen_division > 500, "divisions are common: {seen_division}");
    }

    #[test]
    fn the_os_generator_works_and_varies() {
        let mut os = OsRand;
        let set: std::collections::HashSet<String> =
            (0..200).map(|_| generate(&mut os).text.clone()).collect();
        assert!(set.len() > 150, "equations vary: {}", set.len());
        for _ in 0..2000 {
            assert!(os.below(7) < 7);
        }
        assert_eq!(os.below(1), 0);
    }

    #[test]
    fn debug_hides_the_answer() {
        let e = generate(&mut SeededRand(1));
        assert_eq!(format!("{e:?}"), "Equation(<redacted>)");
    }
}

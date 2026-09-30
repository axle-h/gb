//! A reader of the disassembly's textual asm, not an assembler. It runs `includes.asm` far enough to
//! know every constant rgbasm would, and hands a data table back as the macro invocations its author
//! wrote, for `tables.rs` to read each one in its own terms.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

/// One statement of a table: a macro or directive and its arguments, unevaluated.
#[derive(Debug)]
pub struct Row {
    pub op: String,
    pub args: Vec<String>,
}

pub struct Asm {
    root: PathBuf,
    numbers: HashMap<String, i64>,
    strings: HashMap<String, String>,
    macros: HashMap<String, Vec<String>>,
    unique: usize,
    files: Vec<String>,
    /// Every name `const` or `shift_const` defined, and the file that invoked it.
    consts: Vec<(String, String)>,
}

impl Asm {
    /// Everything `includes.asm` defines, as the Red build sees it.
    pub fn load(root: &Path) -> Self {
        let mut asm = Asm {
            root: root.to_path_buf(),
            numbers: HashMap::new(),
            strings: HashMap::new(),
            macros: HashMap::new(),
            unique: 0,
            files: Vec::new(),
            consts: Vec::new(),
        };
        for (name, value) in [("_RED", 1), ("_RS", 0), ("__RGBDS_MAJOR__", 1), ("__RGBDS_MINOR__", 0), ("__RGBDS_PATCH__", 0)] {
            asm.numbers.insert(name.to_string(), value);
        }
        asm.include("includes.asm");
        asm
    }

    /// An expression as rgbasm evaluates it, or a build failure naming it.
    pub fn eval(&self, expr: &str) -> i64 {
        self.try_eval(expr).unwrap_or_else(|| panic!("cannot evaluate `{expr}`"))
    }

    pub fn try_eval(&self, expr: &str) -> Option<i64> {
        let tokens = self.tokens(expr, 0)?;
        let mut parser = Parser { asm: self, tokens: &tokens, at: 0 };
        let value = parser.binary(0)?;
        (parser.at == tokens.len()).then_some(value)
    }

    /// A `db` argument as the byte it assembles to.
    pub fn byte(&self, expr: &str) -> u8 {
        let value = self.eval(expr);
        assert!((-128..=255).contains(&value), "`{expr}` is {value}, not a byte");
        value as u8
    }

    /// The statements after `label:` in `file`, up to the next label: conditionals resolved, loops
    /// unrolled, `INCLUDE`s read in place and the assertion macros dropped.
    pub fn rows(&mut self, file: &str, label: &str) -> Vec<Row> {
        self.span(file, label, label)
    }

    /// Every statement of `file`, for one that has no label of its own.
    pub fn file_rows(&mut self, file: &str) -> Vec<Row> {
        println!("cargo:rerun-if-changed={}", self.root.join(file).display());
        let lines = self.lines(file);
        let mut rows = Vec::new();
        self.collect(&lines, &mut rows);
        rows
    }

    /// A file of the disassembly, for a build script to embed or to read line by line.
    pub fn path(&self, file: &str) -> PathBuf {
        let path = self.root.join(file);
        println!("cargo:rerun-if-changed={}", path.display());
        path.canonicalize().unwrap_or_else(|e| panic!("{}: {e}", path.display()))
    }

    /// A number defined outside `includes.asm`, as a `const` in a map script is.
    pub fn set(&mut self, name: &str, value: i64) {
        self.numbers.insert(name.to_string(), value);
    }

    /// `rows` from `first:` through `last:`, for a table the code reads across its labels. Either
    /// may be a local `.label`, the first of its name in `file`.
    pub fn span(&mut self, file: &str, first: &str, last: &str) -> Vec<Row> {
        println!("cargo:rerun-if-changed={}", self.root.join(file).display());
        let lines = self.lines(file);
        // A local label, `.name` with or without its colon, is found by its own line, and its rows end
        // at the next label of either kind.
        let after_local = |line: &str, label: &str| line.trim_start().strip_prefix(label)
            .filter(|rest| rest.is_empty() || rest.starts_with([':', ' ', '\t'])).map(|rest| rest.trim_start_matches(':').to_string());
        let local = |line: &str, label: &str| after_local(line, label).is_some();
        let find = |label: &str, from: usize| lines[from..].iter().position(|line| match label.starts_with('.') {
            true => local(line, label),
            false => label_of(line).is_some_and(|(name, _)| name == label),
        }).map(|n| from + n).unwrap_or_else(|| panic!("no `{label}:` in {file}"));
        let start = find(first, 0);
        let last_at = find(last, start);
        let ends = |line: &String| label_of(line).is_some() || last.starts_with('.') && line.trim_start().starts_with('.');
        let end = lines[last_at + 1..].iter().position(ends).map_or(lines.len(), |n| last_at + 1 + n);
        let body: Vec<String> = lines[start..end].iter().enumerate().map(|(i, line)| match label_of(line) {
            Some((_, rest)) => format!("\t{rest}"),
            None if i == 0 => format!("\t{}", after_local(line, first).unwrap()),
            None => line.clone(),
        }).collect();
        let mut rows = Vec::new();
        self.collect(&body, &mut rows);
        rows
    }

    /// Every statement of `file` as `rows` reads a table's, each label a row of its own.
    pub fn file(&mut self, file: &str) -> Vec<Row> {
        println!("cargo:rerun-if-changed={}", self.root.join(file).display());
        let lines = self.lines(file);
        let mut rows = Vec::new();
        self.collect(&lines, &mut rows);
        rows
    }

    /// What an `EQUS` name stands for.
    pub fn equs(&self, name: &str) -> Option<&str> {
        self.strings.get(name).map(String::as_str)
    }

    fn collect(&mut self, lines: &[String], rows: &mut Vec<Row>) {
        let mut conds = Conditions::default();
        let mut at = 0;
        while at < lines.len() {
            let mut line = self.interpolate(&lines[at]);
            at += 1;
            // A label with a statement after it on its line is a row of its own.
            if let (label, rest) = split_word(&line) && label.ends_with(':') && !rest.is_empty() && conds.active() {
                rows.push(Row { op: label.to_string(), args: Vec::new() });
                line = rest.to_string();
            }
            let (word, rest) = split_word(&line);
            let keyword = word.trim_end_matches('?').to_ascii_uppercase();
            if conds.directive(self, &keyword, rest) || !conds.active() {
                continue;
            }
            match keyword.as_str() {
                "" | "TABLE_WIDTH" | "ASSERT_TABLE_LENGTH" | "ASSERT_MAX_TABLE_LENGTH" | "ASSERT_LIST_LENGTH" | "ASSERT" => {}
                "MACRO" => at = endm(lines, at) + 1,
                "DEF" | "REDEF" => self.define(rest),
                "INCLUDE" => {
                    let file = rest.trim().trim_matches('"');
                    println!("cargo:rerun-if-changed={}", self.root.join(file).display());
                    let included = self.lines(file);
                    self.collect(&included, rows);
                }
                "FOR" | "REPT" => {
                    let body = loop_body(lines, &mut at);
                    let args = split_args(rest);
                    let (name, range) = match (keyword.as_str(), args.as_slice()) {
                        ("REPT", [count]) => (None, (0, self.eval(count), 1)),
                        ("FOR", [name, stop]) => (Some(name), (0, self.eval(stop), 1)),
                        ("FOR", [name, start, stop]) => (Some(name), (self.eval(start), self.eval(stop), 1)),
                        ("FOR", [name, start, stop, step]) => (Some(name), (self.eval(start), self.eval(stop), self.eval(step))),
                        _ => panic!("`{line}`"),
                    };
                    let (mut value, stop, step) = range;
                    while if step > 0 { value < stop } else { value > stop } {
                        if let Some(name) = name {
                            self.numbers.insert(name.clone(), value);
                        }
                        self.collect(&body, rows);
                        value += step;
                    }
                }
                _ => rows.push(Row { op: word.to_string(), args: split_args(rest) }),
            }
        }
    }

    /// A module of every name `const` or `shift_const` defined in `file`, and `NAMES` listing them.
    pub fn write_consts(&self, output: &mut String, module: &str, file: &str) {
        use std::fmt::Write;
        writeln!(output, "#[allow(dead_code)]").unwrap();
        writeln!(output, "pub mod {module} {{").unwrap();
        let mut names = Vec::new();
        for (_, name) in self.consts.iter().filter(|(from, _)| from == file) {
            writeln!(output, "    pub const {name}: u16 = {};", u16::try_from(self.numbers[name]).unwrap()).unwrap();
            names.push(format!("(\"{name}\", {name})"));
        }
        writeln!(output, "    pub const NAMES: &[(&str, u16)] = &[{}];", names.join(", ")).unwrap();
        writeln!(output, "}}").unwrap();
    }

    fn include(&mut self, file: &str) {
        println!("cargo:rerun-if-changed={}", self.root.join(file).display());
        let lines = self.lines(file);
        self.files.push(file.to_string());
        self.run(&lines, &mut Vec::new(), 0);
        self.files.pop();
    }

    /// A file's statements: comments stripped and `\` continuations joined.
    pub fn lines(&self, file: &str) -> Vec<String> {
        let path = self.root.join(file);
        let source = fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let mut lines = Vec::new();
        let mut pending = String::new();
        for line in source.lines() {
            let line = strip_comment(line).trim_end();
            match line.strip_suffix('\\').filter(|body| !body.ends_with('\\')) {
                Some(body) => pending.push_str(body),
                None => lines.push(std::mem::take(&mut pending) + line),
            }
        }
        lines
    }

    /// `lines` executed, as the body of a macro invocation numbered `unique` when `args` has any.
    fn run(&mut self, lines: &[String], args: &mut Vec<String>, unique: usize) {
        let mut conds = Conditions::default();
        let mut at = 0;
        while at < lines.len() {
            let line = self.interpolate(&substitute(&lines[at], args, unique));
            at += 1;
            let (word, rest) = split_word(&line);
            let keyword = word.trim_end_matches('?').to_ascii_uppercase();
            if conds.directive(self, &keyword, rest) || !conds.active() {
                if keyword == "MACRO" {
                    at = endm(lines, at) + 1;
                }
                continue;
            }
            match keyword.as_str() {
                "" | "EXPORT" | "PURGE" | "ASSERT" | "STATIC_ASSERT" | "WARN" | "SECTION" | "CHARMAP"
                | "NEWCHARMAP" | "SETCHARMAP" | "PUSHC" | "POPC" | "OPT" => {}
                "MACRO" => {
                    let end = endm(lines, at);
                    self.macros.insert(rest.trim().to_string(), lines[at..end].to_vec());
                    at = end + 1;
                }
                "REPT" | "FOR" => panic!("`{line}`: a loop outside a table is not read"),
                "FAIL" => panic!("`{line}` in {:?}", self.files.last()),
                "INCLUDE" => self.include(rest.trim().trim_matches('"')),
                "DEF" | "REDEF" => self.define(rest),
                "RSRESET" => { self.numbers.insert("_RS".to_string(), 0); }
                "RSSET" => { self.numbers.insert("_RS".to_string(), self.eval(rest)); }
                "SHIFT" => {
                    let count = if rest.trim().is_empty() { 1 } else { self.eval(rest) as usize };
                    args.drain(..count.min(args.len()));
                }
                _ => {
                    let Some(body) = self.macros.get(word).cloned() else { continue };
                    let mut call_args = split_args(rest);
                    if word == "const" || word == "shift_const" {
                        let file = self.files.last().unwrap().clone();
                        self.consts.push((file, call_args[0].clone()));
                    }
                    self.unique += 1;
                    self.run(&body, &mut call_args, self.unique);
                }
            }
        }
    }

    /// `DEF name EQU|=|+=|EQUS|RB|RW|RL ...`. A value that names something not yet known, a label
    /// most often, leaves the name undefined.
    fn define(&mut self, rest: &str) {
        let (name, rest) = split_word(rest);
        let (op, expr) = split_word(rest);
        let name = name.to_string();
        let op = op.to_ascii_uppercase();
        let assign = |asm: &mut Asm, value: Option<i64>| {
            if let Some(value) = value {
                asm.numbers.insert(name.clone(), value);
            }
        };
        match op.as_str() {
            "EQU" | "=" => assign(self, self.try_eval(expr)),
            "EQUS" => {
                if let Some(text) = expr.trim().strip_prefix('"').and_then(|t| t.strip_suffix('"')) {
                    self.strings.insert(name, text.replace("\\\"", "\""));
                }
            }
            "RB" | "RW" | "RL" => {
                let size = match op.as_str() { "RB" => 1, "RW" => 2, _ => 4 };
                let count = if expr.trim().is_empty() { 1 } else { self.eval(expr) };
                let offset = self.numbers["_RS"];
                assign(self, Some(offset));
                self.numbers.insert("_RS".to_string(), offset + size * count);
            }
            compound if compound.ends_with('=') => {
                let current = self.numbers.get(&name).copied();
                let operand = self.try_eval(expr);
                let op = &compound[..compound.len() - 1];
                assign(self, current.zip(operand).and_then(|(a, b)| binary_op(op, a, b)));
            }
            _ => panic!("DEF {name} {op} {expr}"),
        }
    }

    /// rgbasm's `{sym}` and `{fmt:sym}`, left alone when the symbol is unknown.
    fn interpolate(&self, line: &str) -> String {
        let mut out = String::new();
        let mut rest = line;
        while let Some(open) = rest.find('{') {
            let Some(close) = rest[open..].find('}') else { break };
            let inner = &rest[open + 1..open + close];
            let (format, name) = inner.split_once(':').unwrap_or(("", inner));
            let text = match (self.strings.get(name), self.numbers.get(name)) {
                (Some(text), _) => Some(text.clone()),
                (None, Some(&value)) => Some(format_number(format, value)),
                _ => None,
            };
            out.push_str(&rest[..open]);
            out.push_str(text.as_deref().unwrap_or(&rest[open..=open + close]));
            rest = &rest[open + close + 1..];
        }
        out + rest
    }

    fn tokens(&self, expr: &str, depth: usize) -> Option<Vec<Token>> {
        assert!(depth < 16, "EQUS recursion in `{expr}`");
        let mut tokens = Vec::new();
        let chars: Vec<char> = expr.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            let c = chars[i];
            let take = |i: &mut usize, allowed: &dyn Fn(char) -> bool| {
                let start = *i;
                while *i < chars.len() && allowed(chars[*i]) {
                    *i += 1;
                }
                chars[start..*i].iter().filter(|&&c| c != '_').collect::<String>()
            };
            if c.is_whitespace() {
                i += 1;
            } else if c == '$' {
                i += 1;
                tokens.push(Token::Number(i64::from_str_radix(&take(&mut i, &|c| c.is_ascii_hexdigit() || c == '_'), 16).ok()?));
            } else if c == '%' && chars.get(i + 1).is_some_and(|&c| c == '0' || c == '1') {
                i += 1;
                tokens.push(Token::Number(i64::from_str_radix(&take(&mut i, &|c| c == '0' || c == '1' || c == '_'), 2).ok()?));
            } else if c.is_ascii_digit() {
                tokens.push(Token::Number(take(&mut i, &|c| c.is_ascii_digit() || c == '_').parse().ok()?));
            } else if c.is_alphabetic() || c == '_' || c == '.' {
                let start = i;
                while i < chars.len() && (chars[i].is_alphanumeric() || "_#$@.".contains(chars[i])) {
                    i += 1;
                }
                let name: String = chars[start..i].iter().collect();
                match self.strings.get(&name) {
                    Some(text) => tokens.extend(self.tokens(text, depth + 1)?),
                    None => tokens.push(Token::Name(name)),
                }
            } else {
                let op = ["**", "<<", ">>>", ">>", "<=", ">=", "==", "!=", "&&", "||"].into_iter()
                    .find(|op| chars[i..].starts_with(&op.chars().collect::<Vec<_>>()))
                    .map_or_else(|| c.to_string(), str::to_string);
                i += op.len();
                tokens.push(Token::Op(op));
            }
        }
        Some(tokens)
    }
}

#[derive(Clone, Debug)]
enum Token {
    Number(i64),
    Name(String),
    Op(String),
}

/// Loosest first. rgbasm puts the bitwise operators above `+` and `-`, and `&&` level with `||`.
const LEVELS: &[&[&str]] = &[
    &["&&", "||"],
    &["==", "!=", "<", ">", "<=", ">="],
    &["+", "-"],
    &["&", "|", "^"],
    &["<<", ">>", ">>>"],
    &["*", "/", "%"],
];

struct Parser<'a> {
    asm: &'a Asm,
    tokens: &'a [Token],
    at: usize,
}

impl Parser<'_> {
    fn op(&self) -> Option<&str> {
        match self.tokens.get(self.at) {
            Some(Token::Op(op)) => Some(op),
            _ => None,
        }
    }

    fn expect(&mut self, op: &str) -> Option<()> {
        (self.op() == Some(op)).then(|| self.at += 1)
    }

    fn binary(&mut self, level: usize) -> Option<i64> {
        if level == LEVELS.len() {
            return self.power();
        }
        let mut left = self.binary(level + 1)?;
        while let Some(op) = self.op().filter(|op| LEVELS[level].contains(op)).map(str::to_string) {
            self.at += 1;
            let right = self.binary(level + 1)?;
            left = binary_op(&op, left, right)?;
        }
        Some(left)
    }

    fn power(&mut self) -> Option<i64> {
        let base = self.unary()?;
        if self.expect("**").is_some() {
            let exponent = self.power()?;
            return base.checked_pow(u32::try_from(exponent).ok()?);
        }
        Some(base)
    }

    fn unary(&mut self) -> Option<i64> {
        let op = self.op().map(str::to_string);
        match op.as_deref() {
            Some("-") => { self.at += 1; Some(-self.unary()?) }
            Some("+") => { self.at += 1; self.unary() }
            Some("~") => { self.at += 1; Some(!self.unary()?) }
            Some("!") => { self.at += 1; Some((self.unary()? == 0) as i64) }
            Some("(") => {
                self.at += 1;
                let value = self.binary(0)?;
                self.expect(")")?;
                Some(value)
            }
            _ => self.primary(),
        }
    }

    fn primary(&mut self) -> Option<i64> {
        let token = self.tokens.get(self.at)?.clone();
        self.at += 1;
        match token {
            Token::Number(value) => Some(value),
            Token::Name(name) if self.op() == Some("(") => {
                self.at += 1;
                let value = if name.eq_ignore_ascii_case("DEF") {
                    let Some(Token::Name(symbol)) = self.tokens.get(self.at).cloned() else { return None };
                    self.at += 1;
                    let asm = self.asm;
                    (asm.numbers.contains_key(&symbol) || asm.strings.contains_key(&symbol) || asm.macros.contains_key(&symbol)) as i64
                } else {
                    let argument = self.binary(0)?;
                    match name.to_ascii_uppercase().as_str() {
                        "HIGH" => argument >> 8 & 0xFF,
                        "LOW" => argument & 0xFF,
                        _ => return None,
                    }
                };
                self.expect(")")?;
                Some(value)
            }
            Token::Name(name) => self.asm.numbers.get(&name).copied(),
            Token::Op(_) => None,
        }
    }
}

fn binary_op(op: &str, a: i64, b: i64) -> Option<i64> {
    Some(match op {
        "+" => a + b,
        "-" => a - b,
        "*" => a * b,
        "/" => a.checked_div(b)? - ((a % b != 0 && (a < 0) != (b < 0)) as i64),
        "%" => a - b * (a.checked_div(b)? - ((a % b != 0 && (a < 0) != (b < 0)) as i64)),
        "&" => a & b,
        "|" => a | b,
        "^" => a ^ b,
        "<<" => a << b,
        ">>" => a >> b,
        ">>>" => ((a as u32) >> b) as i64,
        "==" => (a == b) as i64,
        "!=" => (a != b) as i64,
        "<" => (a < b) as i64,
        ">" => (a > b) as i64,
        "<=" => (a <= b) as i64,
        ">=" => (a >= b) as i64,
        "&&" => (a != 0 && b != 0) as i64,
        "||" => (a != 0 || b != 0) as i64,
        _ => return None,
    })
}

/// `IF`/`ELIF`/`ELSE`/`ENDC`, nested.
#[derive(Default)]
struct Conditions(Vec<(bool, bool)>);

impl Conditions {
    fn active(&self) -> bool {
        self.0.iter().all(|&(active, _)| active)
    }

    /// Whether `keyword` was a conditional, which this has now applied.
    fn directive(&mut self, asm: &Asm, keyword: &str, rest: &str) -> bool {
        let outer = self.0.len() <= 1 || self.0[..self.0.len() - 1].iter().all(|&(active, _)| active);
        match keyword {
            "IF" => {
                let active = self.active() && asm.eval(rest) != 0;
                self.0.push((active, active));
            }
            "ELIF" => {
                let (active, taken) = self.0.last_mut().expect("ELIF outside IF");
                *active = outer && !*taken && asm.eval(rest) != 0;
                *taken |= *active;
            }
            "ELSE" => {
                let (active, taken) = self.0.last_mut().expect("ELSE outside IF");
                *active = !*taken;
                *taken = true;
            }
            "ENDC" => { self.0.pop().expect("ENDC outside IF"); }
            _ => return false,
        }
        true
    }
}

/// The global label `line` starts with, and what follows it on the line.
fn label_of(line: &str) -> Option<(&str, &str)> {
    let (name, rest) = line.split_once(':')?;
    (!name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '_')).then(|| (name, rest.trim_start_matches(':')))
}

/// The lines of a `FOR` or `REPT` up to its `ENDR`, leaving `at` past it.
fn loop_body(lines: &[String], at: &mut usize) -> Vec<String> {
    let mut depth = 0;
    let start = *at;
    while *at < lines.len() {
        let keyword = split_word(&lines[*at]).0.to_ascii_uppercase();
        *at += 1;
        match keyword.as_str() {
            "FOR" | "REPT" => depth += 1,
            "ENDR" if depth == 0 => return lines[start..*at - 1].to_vec(),
            "ENDR" => depth -= 1,
            _ => {}
        }
    }
    panic!("a loop with no ENDR");
}

/// The index of the `ENDM` closing a macro whose body starts at `from`.
fn endm(lines: &[String], from: usize) -> usize {
    from + lines[from..].iter().position(|line| split_word(line).0.eq_ignore_ascii_case("ENDM")).expect("a MACRO with no ENDM")
}

/// `\1` to `\9`, `\#`, `\@` and `_NARG` in a macro body line.
fn substitute(line: &str, args: &[String], unique: usize) -> String {
    if !line.contains('\\') && !line.contains("_NARG") {
        return line.to_string();
    }
    let mut out = String::new();
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match (c, chars.peek()) {
            ('\\', Some(&digit)) if digit.is_ascii_digit() => {
                chars.next();
                out.push_str(args.get(digit as usize - '1' as usize).map_or("", String::as_str));
            }
            ('\\', Some('#')) => { chars.next(); out.push_str(&args.join(", ")); }
            ('\\', Some('@')) => { chars.next(); out.push_str(&format!("_{unique}")); }
            _ => out.push(c),
        }
    }
    out.replace("_NARG", &args.len().to_string())
}

fn format_number(format: &str, value: i64) -> String {
    let kind = format.chars().last().filter(char::is_ascii_alphabetic);
    let spec = format.trim_end_matches(|c: char| c.is_ascii_alphabetic());
    let width: usize = spec.trim_start_matches('0').parse().unwrap_or(0);
    let text = match kind {
        Some('d') | Some('u') => value.to_string(),
        Some('x') => format!("{value:x}"),
        Some('X') => format!("{value:X}"),
        Some('b') => format!("{value:b}"),
        _ => return format!("${value:X}"),
    };
    if spec.starts_with('0') { format!("{text:0>width$}") } else { format!("{text:>width$}") }
}

fn split_word(line: &str) -> (&str, &str) {
    let line = line.trim_start();
    line.split_once(char::is_whitespace).unwrap_or((line, ""))
}

/// Comma-separated arguments, commas inside quotes or brackets excepted.
pub fn split_args(text: &str) -> Vec<String> {
    let mut args = Vec::new();
    let (mut depth, mut quoted, mut current) = (0, false, String::new());
    for c in text.chars() {
        match c {
            '"' => quoted = !quoted,
            '(' if !quoted => depth += 1,
            ')' if !quoted => depth -= 1,
            ',' if !quoted && depth == 0 => {
                args.push(std::mem::take(&mut current).trim().to_string());
                continue;
            }
            _ => {}
        }
        current.push(c);
    }
    if !current.trim().is_empty() {
        args.push(current.trim().to_string());
    }
    args
}

fn strip_comment(line: &str) -> &str {
    let mut quoted = false;
    for (i, c) in line.char_indices() {
        match c {
            '"' => quoted = !quoted,
            ';' if !quoted => return &line[..i],
            _ => {}
        }
    }
    line
}

//! A deliberately small Clausewitz-script tokenizer + parser, scoped to what the
//! focus-tree adapter needs: comments (`#…`), `@var` arithmetic constants,
//! quoted/bare tokens, `key = value`, `key = { … }` nesting, and the comparison
//! operators (`=`, `==`, `>`, `<`, `>=`, `<=`) that appear inside triggers.
//!
//! This is NOT a general engine — it produces a generic AST [`Value`] that the
//! focus parser ([`crate::parse`]) and the trigger evaluator ([`crate::eval`])
//! walk. It preserves DUPLICATE keys (Clausewitz allows repeated keys such as
//! several `focus = { … }` and several `prerequisite = { … }` blocks), which a
//! map would silently drop — so blocks keep an ordered Vec of (key, op, value).

/// A comparison/assignment operator between a key and its value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Eq,
    EqEq,
    Gt,
    Lt,
    Ge,
    Le,
}

/// A parsed value: a scalar token or a nested block of (key, op, value) entries.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// A bare or quoted scalar (identifier, number, `yes`/`no`, string).
    Scalar(String),
    /// A `{ … }` block. Ordered, duplicate keys preserved.
    Block(Vec<Entry>),
}

/// One `key op value` inside a block.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub key: String,
    pub op: Op,
    pub value: Value,
}

impl Value {
    pub fn as_block(&self) -> Option<&[Entry]> {
        match self {
            Value::Block(v) => Some(v),
            _ => None,
        }
    }
    pub fn as_scalar(&self) -> Option<&str> {
        match self {
            Value::Scalar(s) => Some(s.as_str()),
            _ => None,
        }
    }
}

impl Value {
    /// First child value for `key` in this block (None if not a block / absent).
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.as_block()?
            .iter()
            .find(|e| e.key == key)
            .map(|e| &e.value)
    }
    /// All child entries whose key == `key` (preserves duplicates).
    pub fn get_all<'a>(&'a self, key: &str) -> impl Iterator<Item = &'a Entry> {
        let key = key.to_string();
        let block: &'a [Entry] = self.as_block().unwrap_or(&[]);
        block.iter().filter(move |e| e.key == key)
    }
    /// Scalar child for `key`.
    pub fn get_scalar(&self, key: &str) -> Option<&str> {
        self.get(key).and_then(Value::as_scalar)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Tok<'a> {
    Open,
    Close,
    Op(Op),
    Word(&'a str),
}

struct Lexer<'a> {
    s: &'a [u8],
    i: usize,
}

impl<'a> Lexer<'a> {
    fn new(s: &'a str) -> Self {
        Lexer { s: s.as_bytes(), i: 0 }
    }

    fn next_tok(&mut self) -> Option<Tok<'a>> {
        loop {
            // skip whitespace
            while self.i < self.s.len() && (self.s[self.i] as char).is_ascii_whitespace() {
                self.i += 1;
            }
            if self.i >= self.s.len() {
                return None;
            }
            let c = self.s[self.i];
            if c == b'#' {
                // comment to end of line
                while self.i < self.s.len() && self.s[self.i] != b'\n' {
                    self.i += 1;
                }
                continue;
            }
            break;
        }
        let c = self.s[self.i];
        match c {
            b'{' => {
                self.i += 1;
                Some(Tok::Open)
            }
            b'}' => {
                self.i += 1;
                Some(Tok::Close)
            }
            b'=' => {
                if self.peek(1) == Some(b'=') {
                    self.i += 2;
                    Some(Tok::Op(Op::EqEq))
                } else {
                    self.i += 1;
                    Some(Tok::Op(Op::Eq))
                }
            }
            b'>' => {
                if self.peek(1) == Some(b'=') {
                    self.i += 2;
                    Some(Tok::Op(Op::Ge))
                } else {
                    self.i += 1;
                    Some(Tok::Op(Op::Gt))
                }
            }
            b'<' => {
                if self.peek(1) == Some(b'=') {
                    self.i += 2;
                    Some(Tok::Op(Op::Le))
                } else {
                    self.i += 1;
                    Some(Tok::Op(Op::Lt))
                }
            }
            b'"' => {
                // quoted string (no escape handling needed for focus files)
                let start = self.i + 1;
                let mut j = start;
                while j < self.s.len() && self.s[j] != b'"' {
                    j += 1;
                }
                let w = std::str::from_utf8(&self.s[start..j]).unwrap_or("");
                self.i = (j + 1).min(self.s.len());
                Some(Tok::Word(w))
            }
            _ => {
                // bare word: up to whitespace or a delimiter
                let start = self.i;
                while self.i < self.s.len() {
                    let b = self.s[self.i];
                    if (b as char).is_ascii_whitespace()
                        || b == b'{'
                        || b == b'}'
                        || b == b'='
                        || b == b'#'
                        || b == b'"'
                    {
                        break;
                    }
                    // `>` / `<` end a bare word only when standalone; in focus
                    // files comparison ops are always surrounded by spaces, so a
                    // bare word never contains them — treat as delimiters.
                    if b == b'>' || b == b'<' {
                        break;
                    }
                    self.i += 1;
                }
                let w = std::str::from_utf8(&self.s[start..self.i]).unwrap_or("");
                Some(Tok::Word(w))
            }
        }
    }

    fn peek(&self, off: usize) -> Option<u8> {
        self.s.get(self.i + off).copied()
    }
}

/// Parse a whole focus file's top-level entries (a sequence of `key op value`).
/// `@var` definitions are collected and arithmetic `@[expr]` / `@var` references
/// inside scalar positions are resolved to their numeric value where possible.
pub fn parse_file(src: &str) -> Vec<Entry> {
    // Strip a leading UTF-8 BOM (some the game focus files, e.g. france.txt, start
    // with one; otherwise it glues onto the first `focus_tree` token).
    let src = src.strip_prefix('\u{feff}').unwrap_or(src);
    let mut lex = Lexer::new(src);
    let mut toks = Vec::new();
    while let Some(t) = lex.next_tok() {
        toks.push(t);
    }
    let mut p = Parser { toks: &toks, i: 0, vars: Default::default() };
    p.parse_entries(true)
}

use std::collections::HashMap;

struct Parser<'a> {
    toks: &'a [Tok<'a>],
    i: usize,
    vars: HashMap<String, String>,
}

impl<'a> Parser<'a> {
    fn peek(&self) -> Option<Tok<'a>> {
        self.toks.get(self.i).copied()
    }
    fn bump(&mut self) -> Option<Tok<'a>> {
        let t = self.toks.get(self.i).copied();
        if t.is_some() {
            self.i += 1;
        }
        t
    }

    /// Parse a run of entries until `}` (or EOF at top level).
    fn parse_entries(&mut self, top: bool) -> Vec<Entry> {
        let mut out = Vec::new();
        loop {
            match self.peek() {
                None => break,
                Some(Tok::Close) => {
                    if top {
                        // stray close at top level — consume defensively
                        self.bump();
                        continue;
                    }
                    self.bump();
                    break;
                }
                Some(Tok::Word(w)) => {
                    self.bump();
                    // expect an operator
                    let op = match self.peek() {
                        Some(Tok::Op(o)) => {
                            self.bump();
                            o
                        }
                        // A bare word with no operator (e.g. inside a list block
                        // like search_filters = { A B C }). Represent as a
                        // value-only entry keyed by the word itself, op Eq, so
                        // list membership ("A" present) is still queryable.
                        _ => {
                            out.push(Entry {
                                key: w.to_string(),
                                op: Op::Eq,
                                value: Value::Scalar(self.resolve(w)),
                            });
                            continue;
                        }
                    };
                    let value = self.parse_value();
                    // capture @var definitions
                    if let Some(name) = w.strip_prefix('@') {
                        if let Value::Scalar(s) = &value {
                            self.vars.insert(name.to_string(), s.clone());
                        }
                    }
                    out.push(Entry { key: w.to_string(), op, value });
                }
                Some(Tok::Open) => {
                    // anonymous block (rare) — skip its contents
                    self.bump();
                    let _ = self.parse_entries(false);
                }
                Some(Tok::Op(_)) => {
                    // stray operator — skip
                    self.bump();
                }
            }
        }
        out
    }

    fn parse_value(&mut self) -> Value {
        match self.bump() {
            Some(Tok::Open) => Value::Block(self.parse_entries(false)),
            Some(Tok::Word(w)) => Value::Scalar(self.resolve(w)),
            _ => Value::Scalar(String::new()),
        }
    }

    /// Resolve `@var` references in a scalar to the recorded numeric value.
    /// `@[expr]` arithmetic is NOT evaluated (returns the literal) — focus node
    /// coordinates in our trees use plain integers, not `@`-expressions.
    fn resolve(&self, w: &str) -> String {
        if let Some(name) = w.strip_prefix('@') {
            if !name.starts_with('[') {
                if let Some(v) = self.vars.get(name) {
                    return v.clone();
                }
            }
        }
        w.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_nested_and_dups() {
        let src = r#"
            # comment
            focus_tree = {
                id = german_focus
                focus = { id = A x = 10 y = 0 }
                focus = { id = B x = 20 y = 5
                    prerequisite = { focus = A }
                    prerequisite = { focus = C }
                    mutually_exclusive = { focus = D }
                }
            }
        "#;
        let top = parse_file(src);
        let ft = top.iter().find(|e| e.key == "focus_tree").unwrap();
        assert_eq!(ft.value.get_scalar("id"), Some("german_focus"));
        let focuses: Vec<_> = ft.value.get_all("focus").collect();
        assert_eq!(focuses.len(), 2);
        let b = &focuses[1].value;
        assert_eq!(b.get_scalar("id"), Some("B"));
        let prereqs: Vec<_> = b.get_all("prerequisite").collect();
        assert_eq!(prereqs.len(), 2);
    }

    #[test]
    fn resolves_at_vars() {
        let src = r#"
            @cost = 7
            focus = { id = A cost = @cost }
        "#;
        let top = parse_file(src);
        let f = top.iter().find(|e| e.key == "focus").unwrap();
        assert_eq!(f.value.get_scalar("cost"), Some("7"));
    }

    #[test]
    fn comparison_ops_in_trigger() {
        let src = r#"trigger = { num_of_factories > 5 size >= 2 }"#;
        let top = parse_file(src);
        let t = &top[0].value;
        let e = t.as_block().unwrap();
        assert_eq!(e[0].key, "num_of_factories");
        assert_eq!(e[0].op, Op::Gt);
        assert_eq!(e[1].op, Op::Ge);
    }

    #[test]
    fn list_block_membership() {
        let src = r#"search_filters = { FOCUS_FILTER_POLITICAL FOCUS_FILTER_INDUSTRY }"#;
        let top = parse_file(src);
        let v = &top[0].value;
        let keys: Vec<_> = v.as_block().unwrap().iter().map(|e| e.key.as_str()).collect();
        assert!(keys.contains(&"FOCUS_FILTER_POLITICAL"));
    }
}

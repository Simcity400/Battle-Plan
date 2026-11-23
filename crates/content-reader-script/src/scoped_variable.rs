
use content_reader_parser::fixed5::{decimal_i32, parse_fixed5_string};

pub trait Content {
    fn is_country(&self, tag: &str) -> bool;
    fn constant(&self, parts: &[&str]) -> Constant;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Constant {
    Value(i64),
    Unset(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventTargetKind {
    Global,
    Scoped,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Link {
    From,
    Root,
    Prev,
    This,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ScopedVariable {
    pub parsed: bool,
    pub literal: Option<i64>,
    pub token: Option<String>,
    pub unset: Option<String>,
    pub mtth: Option<String>,
    pub mio: Option<String>,
    pub special_project: Option<String>,
    pub random: bool,
    pub ai_random: bool,
    pub global: bool,
    pub temp: bool,
    pub event_target: Option<EventTargetKind>,
    pub event_target_name: Option<String>,
    pub country: Option<String>,
    pub state: Option<i32>,
    pub prefix: Option<Box<ScopedVariable>>,
    pub chain: Vec<Link>,
    pub name: Vec<u8>,
    pub default: Option<Box<ScopedVariable>>,
    pub target: Option<Box<ScopedVariable>>,
    pub num: bool,
    pub index: Option<Box<ScopedVariable>>,
    pub invalid_syntax: bool,
}

const VARIABLE: &str = "var:";

pub const MAX_NESTING: usize = 64;

pub fn parse(text: &str, content: &impl Content) -> Option<ScopedVariable> {
    nested(text, content, 0)
}

fn nested(text: &str, content: &impl Content, depth: usize) -> Option<ScopedVariable> {
    if depth > MAX_NESTING {
        return None;
    }
    let mut out = ScopedVariable::default();
    if text.is_empty() {
        return Some(out);
    }
    out.parsed = true;
    let rest = if let Some(rest) = text.strip_prefix(VARIABLE) {
        rest
    } else if let Some(rest) = text.strip_prefix("temp_var:") {
        out.temp = true;
        rest
    } else if let Some(rest) = text.strip_prefix("token:") {
        out.token = Some(rest.to_owned());
        return Some(out);
    } else if let Some(rest) = text.strip_prefix("mtth:") {
        out.mtth = Some(rest.to_owned());
        return Some(out);
    } else if let Some(rest) = text.strip_prefix("mio:") {
        out.mio = Some(rest.to_owned());
        return Some(out);
    } else if let Some(rest) = text.strip_prefix("sp:") {
        out.special_project = Some(rest.to_owned());
        return Some(out);
    } else if let Some(rest) = text.strip_prefix("constant:") {
        let parts: Vec<&str> = rest.split('.').collect();
        match content.constant(&parts) {
            Constant::Value(value) => out.literal = Some(value),
            Constant::Unset(why) => out.unset = Some(why),
        }
        return Some(out);
    } else {
        text
    };
    let mut w = rest;
    if w.bytes().all(|byte| b"-+0123456789.".contains(&byte)) {
        out.literal = Some(parse_fixed5_string(w.as_bytes()));
        return Some(out);
    }
    if let Some(at) = w.find('?').filter(|at| *at > 0) {
        out.default = Some(Box::new(nested(&w[at + 1..], content, depth + 1)?));
        w = &w[..at];
    }
    if let Some(at) = w.find('^').filter(|at| *at > 0) {
        let index = w[at + 1..].to_ascii_lowercase();
        out.index = Some(Box::new(nested(&index, content, depth + 1)?));
        w = &w[..at];
    }
    if let Some(at) = w.find('@') {
        let target = w[at + 1..].to_ascii_lowercase();
        let target = target_text(&target, content);
        out.target = Some(Box::new(nested(&target, content, depth + 1)?));
        w = &w[..at];
    }
    if let Some(at) = w.find(':') {
        let prefix = w[..at].to_ascii_lowercase();
        match prefix.as_str() {
            "event_target" => out.event_target = Some(EventTargetKind::Scoped),
            "global_event_target" => out.event_target = Some(EventTargetKind::Global),
            _ => out.prefix = Some(Box::new(nested(&prefix, content, depth + 1)?)),
        }
        w = &w[at + 1..];
    }
    parts(&mut out, w, content);
    Some(out)
}

fn target_text(target: &str, content: &impl Content) -> String {
    let with_id = || format!("{target}.id");
    if signed_integer(target) {
        return if decimal_i32(target.as_bytes()) > 0 {
            with_id()
        } else {
            target.to_owned()
        };
    }
    if scope_word(target) {
        return with_id();
    }
    if target.starts_with(VARIABLE) || target.starts_with("token:") {
        return target.to_owned();
    }
    if target.contains(':') {
        return with_id();
    }
    if let Some((_, rest)) = target.split_once('.') {
        return if scope_word(rest) {
            target.to_owned()
        } else {
            with_id()
        };
    }
    if target.len() == 3 && content.is_country(&target.to_ascii_uppercase()) {
        return with_id();
    }
    format!("token:{target}")
}

fn signed_integer(text: &str) -> bool {
    text.bytes()
        .enumerate()
        .all(|(at, byte)| byte.is_ascii_digit() || (at == 0 && matches!(byte, b'+' | b'-')))
}

fn scope_word(text: &str) -> bool {
    matches!(text, "id" | "this" | "from" | "root" | "prev")
}

fn parts(out: &mut ScopedVariable, w: &str, content: &impl Content) {
    let parts: Vec<&str> = w.split('.').collect();
    let same = |part: &str, word: &str| part.eq_ignore_ascii_case(word);
    match parts.as_slice() {
        [part] => {
            if same(part, "random") {
                out.random = true;
                return;
            }
            if same(part, "ai_random") {
                out.ai_random = true;
                return;
            }
            if same(part, "num") {
                out.num = true;
                return;
            }
            if part.len() == 3 {
                let tag = part.to_ascii_uppercase();
                if content.is_country(&tag) {
                    out.country = Some(tag);
                    return;
                }
            }
        }
        [first, second] => {
            let tag = first.to_ascii_uppercase();
            let matched = if content.is_country(&tag) {
                out.country = Some(tag);
                true
            } else if signed_integer(first) {
                out.state = Some(decimal_i32(first.as_bytes()));
                true
            } else if same(first, "global") {
                out.global = true;
                true
            } else {
                false
            };
            if matched {
                if *second != "id" {
                    out.name = second.to_ascii_lowercase().into_bytes();
                }
                return;
            }
        }
        _ => {}
    }
    let count = parts.len();
    let mut named = false;
    for (at, part) in parts.iter().enumerate() {
        let link = if same(part, "from") {
            Some(Link::From)
        } else if same(part, "root") {
            Some(Link::Root)
        } else if same(part, "prev") {
            Some(Link::Prev)
        } else if same(part, "this") || same(part, "id") {
            Some(Link::This)
        } else {
            None
        };
        if let Some(link) = link {
            out.chain.push(link);
            continue;
        }
        let cut = || w.get(..part.len()).unwrap_or(w).to_owned();
        if count >= 2 && at == count - 2 && !named {
            out.invalid_syntax |= count != 2;
            out.event_target_name = Some(cut());
            out.event_target.get_or_insert(EventTargetKind::Scoped);
            named = true;
        } else if at == count - 1 {
            if out.event_target.is_none() || named {
                out.name = part.to_ascii_lowercase().into_bytes();
            } else {
                out.event_target_name = Some(cut());
                named = true;
            }
        }
    }
}

impl ScopedVariable {
    pub fn literal(value: i64) -> Self {
        Self {
            parsed: true,
            literal: Some(value),
            ..Self::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Facts;
    impl Content for Facts {
        fn is_country(&self, tag: &str) -> bool {
            matches!(tag, "GER" | "SOV" | "SB1")
        }
        fn constant(&self, parts: &[&str]) -> Constant {
            match parts {
                ["a", "b"] => Constant::Value(250_000),
                _ => Constant::Unset("No constant value".into()),
            }
        }
    }

    fn p(text: &str) -> ScopedVariable {
        parse(text, &Facts).unwrap()
    }

    fn named(name: &str) -> ScopedVariable {
        ScopedVariable {
            parsed: true,
            name: name.as_bytes().to_vec(),
            ..ScopedVariable::default()
        }
    }

    #[test]
    fn plain_names_prefixes_and_stores() {
        assert_eq!(p(""), ScopedVariable::default());
        assert_eq!(p("var:X"), named("x"));
        assert_eq!(p("x"), named("x"));
        assert_eq!(
            p("temp_var:x"),
            ScopedVariable {
                temp: true,
                ..named("x")
            }
        );
        assert_eq!(
            p("global.Foo"),
            ScopedVariable {
                global: true,
                ..named("foo")
            }
        );
        assert_eq!(
            p("ger.x"),
            ScopedVariable {
                country: Some("GER".into()),
                ..named("x")
            }
        );
        assert_eq!(
            p("123.x"),
            ScopedVariable {
                state: Some(123),
                ..named("x")
            }
        );
        assert_eq!(
            p("ROOT.x"),
            ScopedVariable {
                chain: vec![Link::Root],
                ..named("x")
            }
        );
        assert_eq!(
            p("PREV.ROOT.x"),
            ScopedVariable {
                chain: vec![Link::Prev, Link::Root],
                ..named("x")
            }
        );
        assert_eq!(
            p("SB1"),
            ScopedVariable {
                parsed: true,
                country: Some("SB1".into()),
                ..ScopedVariable::default()
            }
        );
        assert_eq!(
            p("GER.id"),
            ScopedVariable {
                parsed: true,
                country: Some("GER".into()),
                ..ScopedVariable::default()
            }
        );
    }

    #[test]
    fn numbers_and_constants_are_literals() {
        assert_eq!(p("10"), ScopedVariable::literal(1_000_000));
        assert_eq!(p("-1.5"), ScopedVariable::literal(-150_000));
        assert_eq!(p("var:10"), ScopedVariable::literal(1_000_000));
        assert_eq!(p("constant:a.b"), ScopedVariable::literal(250_000));
        assert_eq!(
            p("constant:a.c").unset.as_deref(),
            Some("No constant value")
        );
        assert_eq!(p("token:foo").token.as_deref(), Some("foo"));
        assert!(p("random").random);
        assert!(p("num").num);
    }

    #[test]
    fn event_targets_and_prefixes() {
        let target = p("event_target:t");
        assert_eq!(target.event_target, Some(EventTargetKind::Scoped));
        assert_eq!(target.event_target_name.as_deref(), Some("t"));
        assert!(target.name.is_empty());
        let target = p("event_target:t.x");
        assert_eq!(target.event_target_name.as_deref(), Some("t"));
        assert_eq!(target.name, b"x");
        let target = p("global_event_target:T.x");
        assert_eq!(target.event_target, Some(EventTargetKind::Global));
        assert_eq!(target.event_target_name.as_deref(), Some("T"));
        let target = p("t.x");
        assert_eq!(target.event_target, Some(EventTargetKind::Scoped));
        assert_eq!(target.event_target_name.as_deref(), Some("t"));
        let target = p("ROOT.owner.x");
        assert_eq!(target.chain, vec![Link::Root]);
        assert_eq!(target.event_target_name.as_deref(), Some("ROOT."));
        assert!(target.invalid_syntax);
        assert_eq!(target.name, b"x");
        let prefixed = p("a:x");
        assert_eq!(prefixed.prefix.as_deref(), Some(&named("a")));
        assert_eq!(prefixed.name, b"x");
    }

    #[test]
    fn defaults_indices_and_targets() {
        let value = p("x?5");
        assert_eq!(
            value.default.as_deref(),
            Some(&ScopedVariable::literal(500_000))
        );
        assert_eq!(value.name, b"x");
        let value = p("x^2");
        assert_eq!(
            value.index.as_deref(),
            Some(&ScopedVariable::literal(200_000))
        );
        let value = p("x^num");
        assert!(value.index.as_deref().is_some_and(|index| index.num));
        assert_eq!(p("x^var:i").index.as_deref(), Some(&named("i")));
        let value = p("x_@ROOT");
        assert_eq!(value.name, b"x_");
        assert_eq!(
            value.target.as_deref().map(|target| target.chain.clone()),
            Some(vec![Link::Root, Link::This])
        );
        assert_eq!(
            p("x@GER").target.as_deref().and_then(|t| t.country.clone()),
            Some("GER".into())
        );
        assert_eq!(
            p("x@123").target.as_deref().and_then(|t| t.state),
            Some(123)
        );
        assert_eq!(
            p("x@-4").target.as_deref(),
            Some(&ScopedVariable::literal(-400_000))
        );
        assert_eq!(
            p("x@communism")
                .target
                .as_deref()
                .and_then(|t| t.token.clone()),
            Some("communism".into())
        );
    }

    #[test]
    fn nesting_beyond_the_backstop_is_refused() {
        let chain = |links: usize| format!("{}1", "x?".repeat(links));
        let deepest = p(&chain(MAX_NESTING));
        let mut default = deepest.default.as_deref();
        for _ in 1..MAX_NESTING {
            default = default.and_then(|value| value.default.as_deref());
        }
        assert_eq!(default, Some(&ScopedVariable::literal(100_000)));
        assert_eq!(parse(&chain(MAX_NESTING + 1), &Facts), None);
        assert_eq!(parse(&chain(100_000), &Facts), None);
        assert_eq!(parse(&"x^".repeat(1_000), &Facts), None);
    }
}

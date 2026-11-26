

use std::{
    collections::HashMap,
    fmt,
    sync::{Arc, LazyLock},
};

use crate::{key_names::KeyNames, native};
use content_reader_parser::parser::{Item, Kind, Operator, Value};

pub type Triggers = Vec<Item>;

pub fn body(value: &Value) -> Option<Triggers> {
    value.items().map(<[Item]>::to_vec)
}

#[derive(Clone)]
pub struct Body {
    code: Box<[u8]>,
    strings: Arc<[Box<[u8]>]>,
}

static EMPTY: LazyLock<Arc<[Box<[u8]>]>> = LazyLock::new(|| Arc::from(Vec::new()));

pub struct Bodies<'a> {
    index: HashMap<&'a [u8], u64>,
    strings: Arc<[Box<[u8]>]>,
}

impl<'a> Bodies<'a> {
    pub fn new(items: &'a [Item]) -> Self {
        let mut index = HashMap::new();
        let mut strings = Vec::new();
        intern(items, &mut index, &mut strings);
        Self {
            index,
            strings: strings.into(),
        }
    }

    pub fn body(&self, items: &[Item]) -> Body {
        if items.is_empty() {
            return Body::default();
        }
        let mut code = Vec::new();
        for item in items {
            self.encode(item, &mut code);
        }
        Body {
            code: code.into(),
            strings: Arc::clone(&self.strings),
        }
    }

    fn encode(&self, item: &Item, code: &mut Vec<u8>) {
        let (variant, nested) = match &item.value {
            Value::Scalar(_) => (0, None),
            Value::Block(items) => (1, Some(items)),
            Value::Tagged { items, .. } => (2, Some(items)),
        };
        code.push(kind_code(item.kind) | operator_code(item.operator) << 4 | variant << 6);
        let key = item.key.as_deref().map_or(0, |key| self.index[key] + 1);
        varint(code, key << 1 | u64::from(item.key_quoted));
        match &item.value {
            Value::Scalar(text) => varint(code, self.index[&text[..]]),
            Value::Tagged { tag, .. } => varint(code, self.index[&tag[..]]),
            Value::Block(_) => {}
        }
        if let Some(items) = nested {
            varint(code, items.len() as u64);
            for item in items {
                self.encode(item, code);
            }
        }
    }
}

impl Body {
    pub fn of(items: &[Item]) -> Self {
        Bodies::new(items).body(items)
    }

    pub fn is_empty(&self) -> bool {
        self.code.is_empty()
    }

    pub fn items(&self) -> Triggers {
        let mut at = 0;
        let mut items = Vec::new();
        while at < self.code.len() {
            items.push(self.item(&mut at));
        }
        items
    }

    fn item(&self, at: &mut usize) -> Item {
        let head = self.code[*at];
        *at += 1;
        let key = self.read(at);
        let text = |index: u64| self.strings[index as usize].to_vec();
        let value = match head >> 6 {
            0 => Value::Scalar(text(self.read(at))),
            1 => Value::Block(self.block(at)),
            _ => {
                let tag = text(self.read(at));
                Value::Tagged {
                    tag,
                    items: self.block(at),
                }
            }
        };
        Item {
            key: (key >> 1).checked_sub(1).map(text),
            key_quoted: key & 1 == 1,
            operator: operator_of(head >> 4 & 3),
            value,
            kind: kind_of(head & 15),
        }
    }

    fn block(&self, at: &mut usize) -> Vec<Item> {
        let count = self.read(at);
        (0..count).map(|_| self.item(at)).collect()
    }

    fn read(&self, at: &mut usize) -> u64 {
        let mut value = 0;
        let mut shift = 0;
        loop {
            let byte = self.code[*at];
            *at += 1;
            value |= u64::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                return value;
            }
            shift += 7;
        }
    }
}

impl Default for Body {
    fn default() -> Self {
        Self {
            code: Box::default(),
            strings: Arc::clone(&EMPTY),
        }
    }
}

impl PartialEq for Body {
    fn eq(&self, other: &Self) -> bool {
        self.items() == other.items()
    }
}

impl Eq for Body {}

impl fmt::Debug for Body {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.items()).finish()
    }
}

fn intern<'a>(items: &'a [Item], index: &mut HashMap<&'a [u8], u64>, strings: &mut Vec<Box<[u8]>>) {
    let mut add = |text: &'a [u8]| {
        index.entry(text).or_insert_with(|| {
            strings.push(text.into());
            strings.len() as u64 - 1
        });
    };
    for item in items {
        if let Some(key) = &item.key {
            add(key);
        }
        match &item.value {
            Value::Scalar(text) => add(text),
            Value::Block(_) => {}
            Value::Tagged { tag, .. } => add(tag),
        }
    }
    for item in items {
        if let Value::Block(nested) | Value::Tagged { items: nested, .. } = &item.value {
            intern(nested, index, strings);
        }
    }
}

fn varint(code: &mut Vec<u8>, mut value: u64) {
    while value >= 0x80 {
        code.push(value as u8 | 0x80);
        value >>= 7;
    }
    code.push(value as u8);
}

fn operator_code(operator: Operator) -> u8 {
    match operator {
        Operator::Equals => 0,
        Operator::Less => 1,
        Operator::Greater => 2,
    }
}

fn operator_of(code: u8) -> Operator {
    match code {
        1 => Operator::Less,
        2 => Operator::Greater,
        _ => Operator::Equals,
    }
}

const KINDS: [Kind; 11] = [
    Kind::Bare,
    Kind::Quoted,
    Kind::Equals,
    Kind::Less,
    Kind::Greater,
    Kind::Open,
    Kind::Close,
    Kind::Other,
    Kind::End,
    Kind::Comment,
    Kind::Code,
];

fn kind_code(kind: Kind) -> u8 {
    match kind {
        Kind::Bare => 0,
        Kind::Quoted => 1,
        Kind::Equals => 2,
        Kind::Less => 3,
        Kind::Greater => 4,
        Kind::Open => 5,
        Kind::Close => 6,
        Kind::Other => 7,
        Kind::End => 8,
        Kind::Comment => 9,
        Kind::Code => 10,
    }
}

fn kind_of(code: u8) -> Kind {
    KINDS[usize::from(code)]
}

pub fn read(text: &[u8], names: &KeyNames) -> native::Read {
    let mut shaper = crate::trigger_grammar::Shaper::new(Some(names));
    native::read(
        text,
        native::Mode::KeepMark,
        native::Top::File(crate::trigger_grammar::Node::Trigger),
        Some(names),
        &mut |node, field| shaper.shape(node, field),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assignment_triggers_keep_their_reader_class_and_scalar_arguments() {
        use crate::trigger_keys::Reader;
        let rows = [
            ("synthetic_region_assignment", 0x10, "0x25"),
            ("synthetic_flag_assignment", 0x8, "yes"),
            ("synthetic_tag_assignment", 0x4, "SYN"),
        ];
        crate::test_game::with(|game| {
            rows.iter().fold(game, |game, &(name, mask, _)| {
                game.trigger(name, mask, Reader::Assign, 0)
            })
        });
        let names = KeyNames::default();
        for (name, mask, operand) in rows {
            let token = crate::trigger_keys::registered_trigger(name.as_bytes()).unwrap();
            assert_eq!(
                crate::trigger_keys::trigger_class(token),
                Some((mask, Reader::Assign))
            );
            for operand in [operand.to_owned(), format!("\"{operand}\"")] {
                let read = read(
                    format!("{name} = {operand} always = yes").as_bytes(),
                    &names,
                );
                assert!(read.unsettled.is_none(), "{name}: {read:?}");
                assert!(read.misreads.is_empty(), "{name}: {read:?}");
                assert_eq!(read.items.len(), 2, "{name}: {read:?}");
                assert!(read.items[0].keyword_is(name));
                assert_eq!(
                    read.items[0].value.scalar(),
                    Some(operand.trim_matches('"').as_bytes())
                );
                assert!(read.items[1].keyword_is("always"));
            }
        }
    }

    #[test]
    fn nested_collection_and_mtth_triggers_keep_content_names() {
        crate::test_game::enter();
        let names = KeyNames::default();
        let read = read(
            b"collection_size = { input = { operators = { limit = { \
                CONTROLLER = { always = yes } } } } value > 0 } \
              can_build_railway = { controller_priority = { modifier = { \
                factor = 2 CONTROLLER = { always = yes } } } }",
            &names,
        );
        assert!(read.unsettled.is_none(), "{read:?}");
        assert!(read.misreads.is_empty(), "{read:?}");
        assert_eq!(read.items.len(), 2, "{read:?}");
        let nested = |item: &Item| item.value.items().unwrap().to_vec();
        let collection = nested(&read.items[0]);
        let input = nested(&collection[0]);
        let operators = nested(&input[0]);
        let limit = nested(&operators[0]);
        assert!(limit[0].keyword_is("controller"));
        assert!(nested(&limit[0])[0].keyword_is("always"));
        let railway = nested(&read.items[1]);
        let priority = nested(&railway[0]);
        let modifier = nested(&priority[0]);
        assert!(modifier[1].keyword_is("controller"));
        assert!(nested(&modifier[1])[0].keyword_is("always"));
    }

    #[test]
    fn a_skipped_limit_defines_nothing() {
        crate::test_game::enter();
        let none = KeyNames::default();
        let read = read(
            b"if = { limit = { always = yes } limit = { @v = yes always = yes } always = @v }",
            &none,
        );
        assert!(read.unsettled.is_none(), "{:?}", read.unsettled);
        let body = read.items[0].value.items().unwrap();
        assert_eq!(body[1].value.items(), Some(&[][..]));
        assert_ne!(body[2].value.scalar(), Some(&b"yes"[..]));
        let read = super::read(
            b"if = { limit = { } limit = { @v = yes always = yes } always = @v }",
            &none,
        );
        let body = read.items[0].value.items().unwrap();
        assert_eq!(body.len(), 3, "{read:?}");
        assert_eq!(body[2].value.scalar(), Some(&b"yes"[..]));
        let text = b"if = { limit = { maybe_scripted = yes } limit = { @v = yes always = yes } always = @v }";
        let scripted = KeyNames {
            scripted: [b"maybe_scripted".to_vec()].into(),
            ..KeyNames::default()
        };
        let body = super::read(text, &scripted).items[0]
            .value
            .items()
            .unwrap()
            .to_vec();
        assert_eq!(body[1].value.items(), Some(&[][..]));
        assert_ne!(body[2].value.scalar(), Some(&b"yes"[..]));
        let body = super::read(text, &none).items[0]
            .value
            .items()
            .unwrap()
            .to_vec();
        assert_eq!(body[2].value.scalar(), Some(&b"yes"[..]));
        let mut gap = KeyNames::default();
        gap.counts.ideologies_gap = Some("common/ideologies/b.txt is unknown from line 3".into());
        let why = super::read(text, &gap).unsettled.unwrap().to_string();
        assert!(
            why.contains("reads a later limit only into an empty one"),
            "{why}"
        );
    }

    #[test]
    fn a_body_class_reads_its_own_keys_first() {
        use crate::trigger_keys::Reader;
        crate::test_game::with(|game| {
            game.trigger_at(0x3C75, "naval_strength_comparison", 0, Reader::Body, 0x140459C80)
                .trigger("has_opinion", 0, Reader::Own(0x1404540c0), 0x141446BF0)
                .trigger("check_variable", 0, Reader::Body, 0x14045AC70)
        });
        let names = KeyNames::default();
        let read = read(
            b"naval_strength_comparison = { other = ENG sub_unit_def_weights = { carrier = 0.5 } nothing = { x = 1 } } \
              has_opinion = { target = GER value > 10 always = { x = 1 } }",
            &names,
        );
        assert!(read.unsettled.is_none(), "{:?}", read.unsettled);
        let naval = read.items[0].value.items().unwrap();
        assert_eq!(naval[1].value.items().map(<[Item]>::len), Some(1));
        assert_eq!(naval[2].value, Value::Block(Vec::new()));
        let opinion = read.items[1].value.items().unwrap();
        assert_eq!(opinion.len(), 3);
        assert_eq!(opinion[2].value, Value::Block(Vec::new()));
        let read = super::read(b"check_variable = { focus_progress > 699.9 }", &names);
        assert!(read.misreads.is_empty(), "{:?}", read.misreads);
        let variable = read.items[0].value.items().unwrap();
        assert_eq!(variable[0].value.scalar(), Some(b"699.9".as_slice()));
    }

    #[test]
    fn a_body_keeps_each_key_operator_and_value() {
        crate::test_game::enter();
        let read = read(
            b"a > 2 b<2 OR = { has_tech = radar c = yes } flat = yes",
            &KeyNames::default(),
        );
        assert!(read.unsettled.is_none());
        let operators: Vec<_> = read.items.iter().map(|item| item.operator).collect();
        use content_reader_parser::parser::Operator::*;
        assert_eq!(operators, [Greater, Less, Equals, Equals]);
        assert_eq!(read.items[2].value.items().map(<[Item]>::len), Some(2));
        assert_eq!(body(&read.items[3].value), None);
        assert_eq!(body(&read.items[2].value).map(|body| body.len()), Some(2));
    }
}

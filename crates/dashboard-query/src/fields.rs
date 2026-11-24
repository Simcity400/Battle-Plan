use serde::de::{self, DeserializeOwned, DeserializeSeed, EnumAccess, MapAccess, SeqAccess, VariantAccess, Visitor, value::BorrowedStrDeserializer};
use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    fmt,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HourRole {
    Undated,
    Read,
    Start,
    End,
    Bucket,
}

impl HourRole {
    pub const fn name(self) -> Option<&'static str> {
        match self {
            Self::Undated => None,
            Self::Read => Some("read"),
            Self::Start => Some("start"),
            Self::End => Some("end"),
            Self::Bucket => Some("bucket"),
        }
    }
}

pub type Fields = &'static [(&'static str, HourRole)];

pub fn field_head(path: &str) -> &str {
    let end = path.find(['.', '[']).unwrap_or(path.len());
    &path[..end]
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Leaf {
    Text,
    Number,
    Opaque,
}

#[derive(Clone, Debug, Default)]
pub struct Walk {
    pub heads: Option<Vec<String>>,
    pub leaves: BTreeMap<String, Leaf>,
    pub interiors: BTreeSet<String>,
    pub failed: Option<String>,
}

fn joined(prefix: &str, path: &str) -> String {
    match (prefix.is_empty(), path.is_empty() || path.starts_with('[')) {
        (true, _) => path.to_owned(),
        (false, true) => format!("{prefix}{path}"),
        (false, false) => format!("{prefix}.{path}"),
    }
}

impl Walk {
    pub fn join(mut self, other: Walk) -> Walk {
        self.heads = match (self.heads.take(), other.heads) {
            (Some(mut heads), Some(more)) => {
                heads.extend(more.into_iter().filter(|head| !heads.contains(head)).collect::<Vec<_>>());
                Some(heads)
            }
            (heads, more) => heads.or(more),
        };
        for (path, kind) in other.leaves {
            let held = self.leaves.entry(path).or_insert(kind);
            *held = (*held).max(kind);
        }
        self.interiors.extend(other.interiors);
        self.leaves.retain(|path, _| !self.interiors.contains(path));
        self.failed = self.failed.or(other.failed);
        self
    }

    pub fn nest(mut self, prefix: &str, other: Walk) -> Walk {
        self.leaves.remove(prefix);
        self.interiors.insert(prefix.to_owned());
        let nested = Walk {
            heads: None,
            leaves: other.leaves.into_iter().map(|(path, kind)| (joined(prefix, &path), kind)).collect(),
            interiors: other.interiors.into_iter().map(|path| joined(prefix, &path)).collect(),
            failed: other.failed.map(|path| joined(prefix, &path)),
        };
        self.join(nested)
    }

    pub fn problems(&self, declared: Fields) -> Vec<String> {
        let mut out = Vec::new();
        if let Some(path) = &self.failed {
            out.push(format!("{} cannot be walked", if path.is_empty() { "the request" } else { path.as_str() }));
        }
        let Some(heads) = &self.heads else {
            out.push("its request fields cannot be listed".to_owned());
            return out;
        };
        let roles: BTreeMap<&str, HourRole> = declared.iter().copied().collect();
        let declared_heads: BTreeSet<&str> = declared.iter().map(|(path, _)| field_head(path)).collect();
        let asked: BTreeSet<&str> = heads.iter().map(String::as_str).collect();
        if declared_heads != asked {
            out.push(format!("declares {declared_heads:?}, asks {asked:?}"));
        }
        for (path, kind) in &self.leaves {
            if *kind != Leaf::Text && !roles.contains_key(path.as_str()) {
                out.push(format!("{path} can carry a number and declares no role"));
            }
        }
        for (path, role) in &roles {
            if path.split(['.', '[']).any(|segment| segment == "*") && *role != HourRole::Undated {
                out.push(format!("{path} is dated inside a map, which no request may be"));
            }
            match (self.leaves.get(*path), self.interiors.contains(*path)) {
                (None, false) => out.push(format!("{path} is not a request field")),
                (Some(Leaf::Text), _) | (None, true) if *role != HourRole::Undated => out.push(format!("{path} cannot hold an hour")),
                _ => {}
            }
        }
        out
    }
}

#[derive(Debug)]
pub struct Stop(String);

impl fmt::Display for Stop {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for Stop {}

impl de::Error for Stop {
    fn custom<T: fmt::Display>(message: T) -> Self {
        Self(message.to_string())
    }
}

const DEEPEST: usize = 12;
const MOST_WALKS: usize = 512;

#[derive(Default)]
struct State {
    walk: Walk,
    choices: BTreeMap<String, usize>,
    variants: BTreeMap<String, usize>,
}

struct Walker<'s> {
    path: String,
    depth: usize,
    state: &'s RefCell<State>,
}

impl<'s> Walker<'s> {
    fn at(&self, segment: &str) -> Walker<'s> {
        Walker { path: joined(&self.path, segment), depth: self.depth + 1, state: self.state }
    }

    fn leaf(&self, kind: Leaf) {
        let mut state = self.state.borrow_mut();
        let held = state.walk.leaves.entry(self.path.clone()).or_insert(kind);
        *held = (*held).max(kind);
    }

    fn interior(&self) -> Result<(), Stop> {
        if self.depth > DEEPEST {
            self.leaf(Leaf::Opaque);
            return Err(Stop(format!("{} nests too deep to walk", self.path)));
        }
        if !self.path.is_empty() {
            let mut state = self.state.borrow_mut();
            state.walk.leaves.remove(&self.path);
            state.walk.interiors.insert(self.path.clone());
        }
        Ok(())
    }

    fn walk<'de, T: DeserializeSeed<'de>>(self, seed: T) -> Result<T::Value, Stop> {
        let (path, state) = (self.path.clone(), self.state);
        seed.deserialize(self).inspect_err(|_| {
            state.borrow_mut().walk.failed.get_or_insert(path);
        })
    }
}

macro_rules! numbers {
    ($($method:ident => $visit:ident($zero:expr)),* $(,)?) => {$(
        fn $method<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Stop> {
            self.leaf(Leaf::Number);
            visitor.$visit($zero)
        }
    )*};
}

impl<'de> de::Deserializer<'de> for Walker<'_> {
    type Error = Stop;

    numbers! {
        deserialize_i8 => visit_i64(0), deserialize_i16 => visit_i64(0), deserialize_i32 => visit_i64(0), deserialize_i64 => visit_i64(0),
        deserialize_i128 => visit_i128(0), deserialize_u8 => visit_u64(0), deserialize_u16 => visit_u64(0), deserialize_u32 => visit_u64(0),
        deserialize_u64 => visit_u64(0), deserialize_u128 => visit_u128(0), deserialize_f32 => visit_f64(0.0), deserialize_f64 => visit_f64(0.0),
    }

    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Stop> {
        self.leaf(Leaf::Opaque);
        visitor.visit_borrowed_str("0")
    }

    fn deserialize_ignored_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Stop> {
        self.leaf(Leaf::Opaque);
        visitor.visit_unit()
    }

    fn deserialize_bool<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Stop> {
        self.leaf(Leaf::Text);
        visitor.visit_bool(false)
    }

    fn deserialize_char<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Stop> {
        self.leaf(Leaf::Text);
        visitor.visit_char('a')
    }

    fn deserialize_str<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Stop> {
        self.leaf(Leaf::Text);
        visitor.visit_borrowed_str("0")
    }

    fn deserialize_string<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Stop> {
        self.deserialize_str(visitor)
    }

    fn deserialize_identifier<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Stop> {
        self.deserialize_str(visitor)
    }

    fn deserialize_bytes<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Stop> {
        self.leaf(Leaf::Text);
        visitor.visit_borrowed_bytes(&[])
    }

    fn deserialize_byte_buf<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Stop> {
        self.deserialize_bytes(visitor)
    }

    fn deserialize_option<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Stop> {
        visitor.visit_some(self)
    }

    fn deserialize_unit<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Stop> {
        self.leaf(Leaf::Text);
        visitor.visit_unit()
    }

    fn deserialize_unit_struct<V: Visitor<'de>>(self, _name: &'static str, visitor: V) -> Result<V::Value, Stop> {
        self.deserialize_unit(visitor)
    }

    fn deserialize_newtype_struct<V: Visitor<'de>>(self, _name: &'static str, visitor: V) -> Result<V::Value, Stop> {
        visitor.visit_newtype_struct(self)
    }

    fn deserialize_seq<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Stop> {
        self.interior()?;
        visitor.visit_seq(Items(vec![self.at("[]")].into_iter()))
    }

    fn deserialize_tuple<V: Visitor<'de>>(self, len: usize, visitor: V) -> Result<V::Value, Stop> {
        self.interior()?;
        visitor.visit_seq(Items((0..len).map(|index| self.at(&index.to_string())).collect::<Vec<_>>().into_iter()))
    }

    fn deserialize_tuple_struct<V: Visitor<'de>>(self, _name: &'static str, len: usize, visitor: V) -> Result<V::Value, Stop> {
        self.deserialize_tuple(len, visitor)
    }

    fn deserialize_map<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Stop> {
        self.interior()?;
        visitor.visit_map(Sampled { walker: Some(self), keyed: false })
    }

    fn deserialize_struct<V: Visitor<'de>>(self, _name: &'static str, fields: &'static [&'static str], visitor: V) -> Result<V::Value, Stop> {
        self.interior()?;
        if self.path.is_empty() {
            self.state.borrow_mut().walk.heads = Some(fields.iter().map(|field| (*field).to_owned()).collect());
        }
        visitor.visit_map(Entries { walker: self, fields, at: 0 })
    }

    fn deserialize_enum<V: Visitor<'de>>(self, _name: &'static str, variants: &'static [&'static str], visitor: V) -> Result<V::Value, Stop> {
        let choice = {
            let mut state = self.state.borrow_mut();
            state.variants.insert(self.path.clone(), variants.len());
            state.choices.get(&self.path).copied().unwrap_or(0)
        };
        let name = variants.get(choice).or(variants.first()).copied().ok_or_else(|| Stop(format!("{} has no variant", self.path)))?;
        visitor.visit_enum(Variant { walker: self, name })
    }
}

struct Items<'s>(std::vec::IntoIter<Walker<'s>>);

impl<'de> SeqAccess<'de> for Items<'_> {
    type Error = Stop;

    fn next_element_seed<T: DeserializeSeed<'de>>(&mut self, seed: T) -> Result<Option<T::Value>, Stop> {
        self.0.next().map(|walker| walker.walk(seed)).transpose()
    }
}

struct Entries<'s> {
    walker: Walker<'s>,
    fields: &'static [&'static str],
    at: usize,
}

impl<'de> MapAccess<'de> for Entries<'_> {
    type Error = Stop;

    fn next_key_seed<K: DeserializeSeed<'de>>(&mut self, seed: K) -> Result<Option<K::Value>, Stop> {
        self.fields.get(self.at).map(|field| seed.deserialize(BorrowedStrDeserializer::new(*field))).transpose()
    }

    fn next_value_seed<T: DeserializeSeed<'de>>(&mut self, seed: T) -> Result<T::Value, Stop> {
        let field = self.fields.get(self.at).ok_or_else(|| Stop(format!("{} has no value to walk", self.walker.path)))?;
        self.at += 1;
        self.walker.at(field).walk(seed)
    }
}

struct Sampled<'s> {
    walker: Option<Walker<'s>>,
    keyed: bool,
}

impl<'de> MapAccess<'de> for Sampled<'_> {
    type Error = Stop;

    fn next_key_seed<K: DeserializeSeed<'de>>(&mut self, seed: K) -> Result<Option<K::Value>, Stop> {
        if self.keyed || self.walker.is_none() {
            return Ok(None);
        }
        self.keyed = true;
        let scratch = RefCell::new(State::default());
        seed.deserialize(Walker { path: String::new(), depth: 0, state: &scratch }).map(Some)
    }

    fn next_value_seed<T: DeserializeSeed<'de>>(&mut self, seed: T) -> Result<T::Value, Stop> {
        let walker = self.walker.take().ok_or_else(|| Stop("a sampled map has one entry".to_owned()))?;
        walker.at("*").walk(seed)
    }
}

struct Variant<'s> {
    walker: Walker<'s>,
    name: &'static str,
}

impl<'de, 's> EnumAccess<'de> for Variant<'s> {
    type Error = Stop;
    type Variant = Self;

    fn variant_seed<T: DeserializeSeed<'de>>(self, seed: T) -> Result<(T::Value, Self), Stop> {
        Ok((seed.deserialize(BorrowedStrDeserializer::new(self.name))?, self))
    }
}

impl<'de> VariantAccess<'de> for Variant<'_> {
    type Error = Stop;

    fn unit_variant(self) -> Result<(), Stop> {
        self.walker.leaf(Leaf::Text);
        Ok(())
    }

    fn newtype_variant_seed<T: DeserializeSeed<'de>>(self, seed: T) -> Result<T::Value, Stop> {
        self.walker.interior()?;
        self.walker.at(self.name).walk(seed)
    }

    fn tuple_variant<V: Visitor<'de>>(self, len: usize, visitor: V) -> Result<V::Value, Stop> {
        self.walker.interior()?;
        de::Deserializer::deserialize_tuple(self.walker.at(self.name), len, visitor)
    }

    fn struct_variant<V: Visitor<'de>>(self, fields: &'static [&'static str], visitor: V) -> Result<V::Value, Stop> {
        self.walker.interior()?;
        let inner = self.walker.at(self.name);
        inner.interior()?;
        visitor.visit_map(Entries { walker: inner, fields, at: 0 })
    }
}

fn walk_once<T: DeserializeOwned>(choices: &BTreeMap<String, usize>) -> State {
    let state = RefCell::new(State { choices: choices.clone(), ..State::default() });
    if T::deserialize(Walker { path: String::new(), depth: 0, state: &state }).is_err() {
        state.borrow_mut().walk.failed.get_or_insert_with(String::new);
    }
    state.into_inner()
}

pub fn walk<T: DeserializeOwned>() -> Walk {
    let mut pending = vec![BTreeMap::new()];
    let mut walked: BTreeSet<BTreeMap<String, usize>> = BTreeSet::from([BTreeMap::new()]);
    let mut out = Walk::default();
    while let Some(choices) = pending.pop() {
        let next = walk_once::<T>(&choices);
        for (path, count) in &next.variants {
            if choices.contains_key(path) {
                continue;
            }
            for other in 1..*count {
                let mut more = choices.clone();
                more.insert(path.clone(), other);
                if walked.len() >= MOST_WALKS {
                    out.failed.get_or_insert_with(|| path.clone());
                } else if walked.insert(more.clone()) {
                    pending.push(more);
                }
            }
        }
        out = out.join(next.walk);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    #[expect(dead_code, reason = "only the field names are read")]
    struct Asked {
        branch_id: String,
        first_hour: i32,
        #[serde(rename = "to")]
        last: i32,
        parts: Vec<Part>,
        family: Option<Family>,
        pairs: Vec<(String, Vec<Span>)>,
        country: Country,
        key: Key,
        view: View,
        after: i32,
        #[serde(deserialize_with = "number_text")]
        page: u64,
    }

    fn number_text<'de, D: serde::Deserializer<'de>>(text: D) -> Result<u64, D::Error> {
        <String as serde::Deserialize>::deserialize(text)?.parse().map_err(serde::de::Error::custom)
    }

    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    #[expect(dead_code, reason = "only the field names are read")]
    struct View {
        kind: ViewKind,
        month: Option<i32>,
    }

    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    #[cfg_attr(not(test), expect(dead_code, reason = "only the field names are read"))]
    enum ViewKind {
        All,
        Month,
    }

    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    #[expect(dead_code, reason = "only the field names are read")]
    struct Part {
        slot: u32,
        first_observed_hour: i32,
        started: bool,
    }

    #[derive(serde::Deserialize)]
    #[expect(dead_code, reason = "only the field names are read")]
    struct Family {
        kind: String,
        membership: Vec<Span>,
    }

    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    #[expect(dead_code, reason = "only the field names are read")]
    struct Span {
        first_hour: i32,
        last_hour: i32,
    }

    #[derive(serde::Deserialize)]
    #[serde(untagged)]
    #[expect(dead_code, reason = "only the field names are read")]
    enum Country {
        Tag(String),
        Slot(u32),
    }

    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    #[expect(dead_code, reason = "only the field names are read")]
    enum Key {
        Home,
        At { hour: i32 },
        Nested(Inner),
    }

    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    #[expect(dead_code, reason = "only the field names are read")]
    enum Inner {
        Plain,
        Deep { hour: i32 },
    }

    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    #[expect(dead_code, reason = "only the field names are read")]
    struct Hidden {
        branch_id: String,
        view: Tagged,
        after: i32,
    }

    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase", tag = "kind")]
    #[expect(dead_code, reason = "only the field names are read")]
    enum Tagged {
        Rules { branch: String },
        Manpower { hours: Vec<i32> },
    }

    #[test]
    fn a_request_walk_names_every_nested_field_that_can_carry_a_number() {
        let walk = walk::<Asked>();
        assert_eq!(walk.failed, None);
        assert_eq!(walk.heads, Some(["branchId", "firstHour", "to", "parts", "family", "pairs", "country", "key", "view", "after", "page"].map(str::to_owned).to_vec()));
        let numbers: Vec<&str> = walk.leaves.iter().filter(|(_, kind)| **kind != Leaf::Text).map(|(path, _)| path.as_str()).collect();
        assert_eq!(numbers, ["after", "country", "family.membership[].firstHour", "family.membership[].lastHour", "firstHour", "key.at.hour",
            "key.nested.deep.hour", "pairs[].1[].firstHour", "pairs[].1[].lastHour", "parts[].firstObservedHour", "parts[].slot", "to", "view.month"],
            "a variant reachable only through another non-default variant is still walked");
        assert!(walk.interiors.contains("family") && walk.interiors.contains("parts[]") && walk.interiors.contains("key"));
        assert_eq!(field_head("reads[].hour"), "reads");
        assert_eq!(field_head("members.spans"), "members");
    }

    #[test]
    fn a_declaration_names_each_number_once_and_dates_only_numbers() {
        let walk = walk::<Asked>();
        use HourRole::{End, Read, Start, Undated};
        let complete: Fields = &[("branchId", Undated), ("firstHour", Start), ("to", End), ("parts[].firstObservedHour", Read), ("parts[].slot", Undated),
            ("family.membership[].firstHour", Start), ("family.membership[].lastHour", End), ("pairs[].1[].firstHour", Start), ("pairs[].1[].lastHour", End),
            ("country", Undated), ("key.at.hour", Read), ("key.nested.deep.hour", Read), ("view", Undated), ("view.month", Undated), ("after", Undated), ("page", Undated)];
        assert_eq!(walk.problems(complete), Vec::<String>::new());
        let undated_family: Fields = &[("branchId", Undated), ("firstHour", Start), ("to", End), ("parts[].firstObservedHour", Start), ("parts[].slot", Undated),
            ("family", Undated), ("pairs[].1[].firstHour", Start), ("pairs[].1[].lastHour", End), ("country", Undated), ("key", Read), ("parts[].started", Read), ("key.nested.deep.hour", Read), ("view", Undated), ("after", Undated), ("page", Undated)];
        assert_eq!(walk.problems(undated_family), [
            "family.membership[].firstHour can carry a number and declares no role",
            "family.membership[].lastHour can carry a number and declares no role",
            "key.at.hour can carry a number and declares no role",
            "view.month can carry a number and declares no role",
            "key cannot hold an hour",
            "parts[].started cannot hold an hour",
        ]);
    }

    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    #[expect(dead_code, reason = "only the field names are read")]
    struct Keyed {
        branch_id: String,
        spans: BTreeMap<String, Span>,
        by_slot: BTreeMap<u32, Vec<i32>>,
    }

    #[test]
    fn a_map_is_walked_through_one_entry() {
        let walk = walk::<Keyed>();
        assert_eq!(walk.failed, None);
        assert_eq!(walk.problems(&[("branchId", HourRole::Undated), ("spans", HourRole::Undated), ("bySlot", HourRole::Undated)]), [
            "bySlot.*[] can carry a number and declares no role",
            "spans.*.firstHour can carry a number and declares no role",
            "spans.*.lastHour can carry a number and declares no role",
        ], "a map's values are walked, so a dated field inside one declares its role");
        let dated: Fields = &[("branchId", HourRole::Undated), ("spans", HourRole::Undated), ("bySlot", HourRole::Undated), ("bySlot.*[]", HourRole::Undated),
            ("spans.*.firstHour", HourRole::Start), ("spans.*.lastHour", HourRole::Undated)];
        assert_eq!(walk.problems(dated), ["spans.*.firstHour is dated inside a map, which no request may be"], "a client cannot walk a map's keys, so no hour may hide in one");
    }

    #[test]
    fn a_field_the_walk_cannot_enter_fails_the_walk_by_name() {
        let walk = walk::<Hidden>();
        assert_eq!(walk.failed.as_deref(), Some("view"), "an internally tagged enum hides its variants' fields, so it is refused, not skipped");
        assert_eq!(walk.problems(&[("branchId", HourRole::Undated), ("view", HourRole::Undated), ("after", HourRole::Undated)])[0], "view cannot be walked");
    }
}

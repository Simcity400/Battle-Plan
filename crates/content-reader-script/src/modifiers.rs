
use crate::{key_names::KeyNames, native::{self, Field, Shape}};
use content_reader_parser::parser::{Item, Unheld, Value, c_string};
use std::collections::BTreeSet;

fn builtin_registered(key: &[u8]) -> bool {
    engine_executable::executable_tables::required(
        engine_executable::executable_tables::get()
            .keywords
            .as_ref(),
    )
    .modifier_names
    .iter()
    .any(|name| name.as_bytes().eq_ignore_ascii_case(key))
}

pub fn registered(key: &[u8], names: Option<&KeyNames>) -> Option<bool> {
    let key = c_string(key);
    if builtin_registered(key) {
        return Some(true);
    }
    let effects = &names?.effects;
    if effects.modifiers.contains(&key.to_ascii_lowercase()) {
        Some(true)
    } else if effects.modifiers_gap.is_some() {
        None
    } else {
        Some(false)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Modifier {
    pub key: String,
    pub name: String,
    pub text: String,
    pub value: i64,
}

pub fn stored(item: &Item) -> i64 {
    read_fixed5(item).unwrap_or(0)
}

pub fn read_fixed5(item: &Item) -> Option<i64> {
    item.value.scalar().and_then(content_reader_parser::fixed5::script_fixed5)
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Modifiers {
    pub values: Vec<Modifier>,
    pub hidden: BTreeSet<String>,
    tooltips: TooltipTable,
    pub children: Vec<Modifiers>,
    pub unheld: Unheld,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TargetedModifier {
    pub tag: Option<String>,
    pub operand: Option<Vec<u8>>,
    pub modifiers: Modifiers,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EquipmentBonus {
    pub equipment: String,
    pub instant: bool,
    pub values: Vec<Modifier>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Node {
    Modifier,
    Hidden,
    Targeted,
    EquipmentBonuses,
    EquipmentBonus,
    ResearchBonus,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Undecided {
    pub name: String,
    pub line: u32,
    pub reader: &'static str,
    pub untraced: bool,
}

impl Undecided {
    pub fn untraced(field: &Field<'_>, reader: &'static str) -> Self {
        Self {
            name: String::from_utf8_lossy(&field.key.text).into_owned(),
            line: field.key.line,
            reader,
            untraced: true,
        }
    }
}

impl std::fmt::Display for Undecided {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.untraced {
            return write!(
                f,
                "line {}: {} is read by {}, which the port does not trace, so what it consumes and defines is unknown",
                self.line, self.name, self.reader
            );
        }
        write!(
            f,
            "line {}: {} has a block value, which {} reads as its value when {} is registered (the block's contents then read as the enclosing entries) and skips otherwise; the content does not say which",
            self.line, self.name, self.reader, self.name
        )
    }
}

pub type Registry<'a> = &'a dyn Fn(&[u8]) -> Option<bool>;

pub const UNKNOWN: Registry<'static> = &|_| None;

pub fn shape(
    node: Node,
    field: &Field<'_>,
    registry: Registry<'_>,
    undecided: &mut Vec<Undecided>,
) -> Shape<Node> {
    let keyword = field.keyword();
    let mut decide = |reader, registry: Registry<'_>| {
        if keyword.is_none() && field.key.kind != native::Kind::Quoted {
            return Shape::Skip;
        }
        if !field.skips_block() {
            return Shape::One;
        }
        match registry(&field.key.text) {
            Some(true) => Shape::One,
            Some(false) => Shape::Skip,
            None => {
                undecided.push(Undecided {
                    name: String::from_utf8_lossy(&field.key.text).into_owned(),
                    line: field.key.line,
                    reader,
                    untraced: false,
                });
                Shape::Skip
            }
        }
    };
    match node {
        Node::Modifier => match keyword.as_deref() {
            Some(b"name" | b"data" | b"custom_modifier_tooltip") => Shape::One,
            Some(b"hidden_modifier") => Shape::ObjectOrSkip(Node::Hidden),
            Some(b"added_modifier") => Shape::Object(Node::Modifier),
            _ => decide("modifier value reader", registry),
        },
        Node::Hidden => decide("modifier value reader", registry),
        Node::Targeted => match keyword.as_deref() {
            Some(b"key" | b"tag") => Shape::One,
            Some(b"valid_relation_trigger") => Shape::Trigger,
            _ => shape(Node::Modifier, field, registry, undecided),
        },
        Node::EquipmentBonuses => Shape::Object(Node::EquipmentBonus),
        Node::EquipmentBonus if field.keyword_is("instant") => Shape::One,
        Node::EquipmentBonus => decide("equipment stat reader", UNKNOWN),
        Node::ResearchBonus => Shape::One,
    }
}

fn modifier_name<'a>(item: &'a Item, unheld: &mut Unheld) -> Option<&'a str> {
    if item.keyword().is_none() && !item.key_quoted {
        return None;
    }
    unheld.keep("a modifier key", item.key_str())
}

fn value(item: &Item, unheld: &mut Unheld) -> Option<Modifier> {
    let key = modifier_name(item, unheld)?;
    let text = match &item.value {
        Value::Scalar(bytes) => unheld.keep(
            &format!("modifier {key}'s value"),
            content_reader_parser::parser::text(bytes),
        )?,
        Value::Block(_) | Value::Tagged { .. } => "{",
    };
    Some(Modifier {
        key: key.to_owned(),
        name: key.to_ascii_lowercase(),
        text: text.to_owned(),
        value: stored(item),
    })
}

impl Modifiers {
    pub fn copied(&self) -> Self {
        Self {
            values: self.values.clone(),
            hidden: self.hidden.clone(),
            tooltips: self.tooltips.copied(),
            children: self.children.iter().map(Self::copied).collect(),
            unheld: self.unheld.clone(),
        }
    }

    pub fn effectively_empty(
        &self,
        eligible: &mut impl FnMut(&str) -> Result<bool, String>,
    ) -> Result<bool, String> {
        if !self.tooltips.is_empty() || !self.children.is_empty() {
            return Ok(false);
        }
        if let Some(reason) = self.unheld.cause() {
            return Err(format!("modifier emptiness: {reason} (count: 1)"));
        }
        for value in &self.values {
            if eligible(&value.name)? {
                return Ok(false);
            }
        }
        Ok(true)
    }
    pub fn read(&mut self, item: &Item) {
        match item.keyword().map(|keyword| keyword.folded()).as_deref() {
            Some(b"name" | b"data") => {}
            Some(b"custom_modifier_tooltip") => {
                if let Some(text) = self
                    .unheld
                    .keep("custom_modifier_tooltip", item.value.read_str())
                {
                    self.tooltips.insert(text.to_owned());
                }
            }
            Some(b"hidden_modifier") => {
                for entry in item.value.items().unwrap_or_default() {
                    if let Some(modifier) = value(entry, &mut self.unheld) {
                        self.hidden.insert(modifier.name.clone());
                        self.store(modifier);
                    }
                }
            }
            Some(b"added_modifier") => {
                let child = values(item.value.items().unwrap_or_default());
                self.unheld.absorb(&child.unheld);
                self.children.push(child);
            }
            _ => {
                if let Some(modifier) = value(item, &mut self.unheld) {
                    self.store(modifier);
                }
            }
        }
    }

    pub fn read_all(&mut self, items: &[Item]) {
        for item in items {
            self.read(item);
        }
    }

    fn store(&mut self, modifier: Modifier) {
        match self
            .values
            .iter_mut()
            .find(|existing| existing.name == modifier.name)
        {
            Some(existing) => *existing = modifier,
            None => self.values.push(modifier),
        }
    }

    pub fn tooltips(&self) -> Vec<String> {
        self.tooltips.keys().map(str::to_owned).collect()
    }

    pub fn tooltip_table(&self) -> &TooltipTable {
        &self.tooltips
    }

    pub fn listed(&self) -> impl Iterator<Item = &Modifier> {
        self.values
            .iter()
            .filter(|modifier| !self.hidden.contains(&modifier.name))
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Slot {
    hash: u32,
    distance: u8,
    key: String,
}

const LOAD: f32 = f32::from_bits(0x3f66_6666);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TooltipTable {
    slots: Vec<Slot>,
    count: i32,
    mask: u32,
    overflow: u8,
    shared: bool,
}

impl Default for TooltipTable {
    fn default() -> Self {
        Self {
            slots: vec![
                Slot::default(),
                Slot {
                    distance: 0xff,
                    ..Slot::default()
                },
            ],
            count: 0,
            mask: 0,
            overflow: 0,
            shared: true,
        }
    }
}

impl TooltipTable {
    pub fn keys(&self) -> impl Iterator<Item = &str> {
        let end = self.mask as usize + self.overflow as usize + 1;
        self.slots[..end]
            .iter()
            .filter(|slot| slot.distance != 0)
            .map(|slot| slot.key.as_str())
    }

    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    pub fn copied(&self) -> Self {
        let mut copy = Self::default();
        if self.count == 0 {
            return copy;
        }
        let factor = (1.0_f32 / LOAD).max(1.9_f32);
        let size = (factor * self.count as f32 + 1.0_f32) as i32;
        copy.rehash(f64::from(size).log2().ceil() as u32);
        let end = self.mask as usize + self.overflow as usize + 1;
        for slot in self.slots[..end].iter().filter(|slot| slot.distance != 0) {
            copy.insert_hashed(slot.hash, slot.key.clone());
        }
        copy
    }

    fn insert(&mut self, key: String) {
        let hash = key.bytes().fold(0x811c_9dc5_u32, |hash, byte| {
            (hash ^ u32::from(byte)).wrapping_mul(0x0100_0193)
        });
        self.insert_hashed(hash, key);
    }

    fn insert_hashed(&mut self, hash: u32, key: String) {
        let mut position = (self.mask & hash) as usize;
        let mut distance = 1_u8;
        if self.slots[position].distance != 0 {
            loop {
                if self.slots[position].key == key {
                    return;
                }
                position += 1;
                distance = distance.wrapping_add(1);
                if distance > self.slots[position].distance {
                    break;
                }
            }
        }
        let loaded = (self.count + 1) as f32 / self.mask as i32 as f32;
        if self.overflow < distance || (LOAD <= loaded && loaded != LOAD) {
            self.grow();
            return self.insert_hashed(hash, key);
        }
        if self.slots[position].distance == 0 {
            self.slots[position] = Slot {
                hash,
                distance,
                key,
            };
            self.count += 1;
            return;
        }
        let mut carried = std::mem::replace(
            &mut self.slots[position],
            Slot {
                hash,
                distance,
                key,
            },
        );
        carried.distance = carried.distance.wrapping_add(1);
        let mut at = position;
        loop {
            loop {
                let next = at + 1;
                if self.slots[next].distance == 0 {
                    self.slots[next] = carried;
                    self.count += 1;
                    return;
                }
                if carried.distance <= self.slots[next].distance {
                    break;
                }
                std::mem::swap(&mut carried, &mut self.slots[next]);
                carried.distance = carried.distance.wrapping_add(1);
                at = next;
            }
            carried.distance = carried.distance.wrapping_add(1);
            at += 1;
            if carried.distance > self.overflow {
                break;
            }
        }
        std::mem::swap(&mut carried, &mut self.slots[position]);
        self.grow();
        self.insert_hashed(carried.hash, carried.key);
    }

    fn grow(&mut self) {
        let power = match self.mask {
            0 | 1 | 3 => 3,
            mask => (mask + 1).trailing_zeros() + 1,
        };
        self.rehash(power);
    }

    fn rehash(&mut self, power: u32) {
        if !(1..=30).contains(&power) {
            return;
        }
        let buckets = 1_usize << power;
        let overflow = power as u8 + 2;
        let mut slots = vec![Slot::default(); buckets + usize::from(overflow) + 1];
        slots.last_mut().expect("sentinel").distance = 0xff;
        let old = std::mem::replace(&mut self.slots, slots);
        let shared = std::mem::replace(&mut self.shared, false);
        let mut remaining = self.count;
        self.count = 0;
        self.mask = buckets as u32 - 1;
        self.overflow = overflow;
        if shared {
            return;
        }
        for slot in old {
            if remaining <= 0 {
                break;
            }
            if slot.distance != 0 {
                self.insert_hashed(slot.hash, slot.key);
                remaining -= 1;
            }
        }
    }
}

pub fn values(items: &[Item]) -> Modifiers {
    let mut result = Modifiers::default();
    result.read_all(items);
    result
}

pub fn targeted(items: &[Item]) -> TargetedModifier {
    let mut tag = None;
    let mut operand = None;
    let mut modifiers = Modifiers::default();
    for item in items {
        match item.keyword().map(|keyword| keyword.folded()).as_deref() {
            Some(b"key" | b"tag") => {
                let text = modifiers
                    .unheld
                    .keep("a targeted modifier's tag", item.value.read_str());
                operand = text.map(|text| text.as_bytes().to_vec());
                tag = text.map(str::to_ascii_uppercase);
            }
            Some(b"valid_relation_trigger") => {}
            _ => modifiers.read(item),
        }
    }
    TargetedModifier {
        tag: tag.filter(|tag| !tag.contains(['[', '@', ':'])),
        operand,
        modifiers,
    }
}

pub fn equipment_bonuses(items: &[Item], unheld: &mut Unheld) -> Vec<EquipmentBonus> {
    items
        .iter()
        .filter_map(|item| {
            let equipment = unheld.keep("an equipment bonus's equipment", item.key_str())?;
            let mut instant = false;
            let mut values: Vec<Modifier> = Vec::new();
            for field in item.value.items().unwrap_or_default() {
                if field.keyword_is("instant") {
                    instant = field
                        .value
                        .scalar()
                        .and_then(content_reader_parser::parser::read_bool)
                        .unwrap_or(instant);
                    continue;
                }
                let Some(stat) = value(field, unheld) else {
                    continue;
                };
                match values
                    .iter_mut()
                    .find(|existing| existing.name == stat.name)
                {
                    Some(existing) => existing.value = existing.value.wrapping_add(stat.value),
                    None => values.push(stat),
                }
            }
            Some(EquipmentBonus {
                equipment: equipment.to_owned(),
                instant,
                values,
            })
        })
        .collect()
}

pub fn research_bonuses(items: &[Item], unheld: &mut Unheld) -> Vec<Modifier> {
    items
        .iter()
        .filter_map(|item| value(item, unheld))
        .collect()
}

#[derive(Default)]
pub struct ScanFailures {
    count: usize,
    examples: Vec<String>,
    distinct: Option<BTreeSet<(String, String, String)>>,
    unknown_modifiers: Vec<String>,
}

impl ScanFailures {
    pub fn distinct() -> Self {
        Self {
            distinct: Some(BTreeSet::new()),
            ..Self::default()
        }
    }

    pub fn note(&mut self, owner: &str, key: &str, text: &str) {
        if let Some(seen) = &mut self.distinct
            && !seen.insert((owner.to_owned(), key.to_owned(), text.to_owned()))
        {
            return;
        }
        self.count += 1;
        if self.examples.len() < 5 {
            self.examples.push(format!("{owner}: {key} = {text}"));
        }
    }

    pub fn fixed5(&mut self, owner: &str, item: &Item) {
        if item.kind == native::Kind::Open {
            return;
        }
        if let Some(text) = item.value.scalar()
            && read_fixed5(item).is_none()
        {
            self.note(
                owner,
                item.key_str().unwrap_or("<non-text key>"),
                &String::from_utf8_lossy(text),
            );
        }
    }

    pub fn int32(&mut self, owner: &str, item: &Item) {
        if item.kind == native::Kind::Open {
            return;
        }
        if let Some(text) = item.value.scalar()
            && !matches!(content_reader_parser::crt::scan_i32(text), content_reader_parser::crt::Scanned::Stored(_))
        {
            self.note(
                owner,
                item.key_str().unwrap_or("<non-text key>"),
                &String::from_utf8_lossy(text),
            );
        }
    }

    pub fn scan<'a>(&mut self, owner: &str, values: impl IntoIterator<Item = &'a Modifier>) {
        for value in values {
            if content_reader_parser::fixed5::script_fixed5(value.text.as_bytes()).is_none() {
                self.note(owner, &value.key, &value.text);
            }
        }
    }

    pub fn modifiers(&mut self, owner: &str, values: &Modifiers) {
        self.scan(owner, &values.values);
        for child in &values.children {
            self.modifiers(owner, child);
        }
    }

    pub fn idea_modifiers(&mut self, owner: &str, values: &Modifiers, names: &KeyNames) {
        for value in &values.values {
            if content_reader_parser::fixed5::script_fixed5(value.text.as_bytes()).is_some() {
                continue;
            }
            match registered(value.name.as_bytes(), Some(names)) {
                Some(false) => self
                    .unknown_modifiers
                    .push(format!("{owner}: {} = {}", value.key, value.text)),
                Some(true) | None => self.note(owner, &value.key, &value.text),
            }
        }
        for child in &values.children {
            self.idea_modifiers(owner, child, names);
        }
    }

    pub fn warning(&self) -> Option<String> {
        let more = match self.count - self.examples.len() {
            0 => String::new(),
            more => format!("; and {more} more"),
        };
        let numeric = (self.count > 0).then(|| {
            format!(
                "{} content values failed native numeric conversion; the game's stored \
                 destinations are retained: {}{more}.",
                self.count,
                self.examples.join("; "),
            )
        });
        let unknown = (!self.unknown_modifiers.is_empty()).then(|| {
            format!(
                "{} unknown modifier keys were reported and refused by the modifier reader: {}.",
                self.unknown_modifiers.len(),
                self.unknown_modifiers.join("; ")
            )
        });
        match (numeric, unknown) {
            (Some(numeric), Some(unknown)) => Some(format!("{numeric} {unknown}")),
            (Some(note), None) | (None, Some(note)) => Some(note),
            (None, None) => None,
        }
    }
}

#[cfg(test)]
mod tests;

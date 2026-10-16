//! Authored modifier blocks, targeted modifiers and equipment bonuses.
//!
//! Modifier identity folds ASCII case. Repeated values replace in their original position;
//! hidden values remain stored, added blocks remain children and custom tooltips follow
//! their string table's slot order. Fixed names use verified executable metadata, while
//! authored modifier names come from the recorded content stack at the reader's load point.

use crate::{
    key_names::KeyNames,
    parser::{
        Item, Unheld, Value, c_string,
        native::{self, Field, Shape},
    },
};
use std::collections::BTreeSet;

fn builtin_registered(key: &[u8]) -> bool {
    engine::modifier_keys::names()
        .iter()
        .any(|name| name.as_bytes().eq_ignore_ascii_case(key))
}

/// Bare keys are lexer-folded; quoted keys fold ASCII before hashing
/// and comparing. A known name wins even if a later recorded-stack
/// modifier/sub-unit suffix is unread.
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

/// One stored modifier value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Modifier {
    /// Authored spelling, retained separately from folded identity.
    pub key: String,
    /// The modifier it names: the key ASCII-folded, whether bare or quoted.
    /// Entries merge and hide, and join the modifier definitions, by this.
    pub name: String,
    /// The resolved value token's text, kept apart from the value.
    pub text: String,
    pub value: i64,
}

/// The fixed5 destination starts at 0. The value token's text is scanned
/// as an integer, so a numeric prefix reads (`1tail` is 1), `0x10` is
/// hexadecimal and empty text is 0; text it cannot convert (`yes`, `[?x]`,
/// an undefined `@name`) is reported and leaves the 0, which the game
/// then holds.
pub fn stored(item: &Item) -> i64 {
    read_fixed5(item).unwrap_or(0)
}

/// Fixed5 conversion: `None` when it fails and the destination keeps
/// what it held.
pub fn read_fixed5(item: &Item) -> Option<i64> {
    item.value.scalar().and_then(crate::fixed5::script_fixed5)
}

/// A modifier block held by one modifier object.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Modifiers {
    /// One entry per folded identity, in first-read order.
    pub values: Vec<Modifier>,
    /// The modifiers `hidden_modifier` named: stored in `values`,
    /// never listed.
    pub hidden: BTreeSet<String>,
    tooltips: TooltipTable,
    /// `added_modifier` blocks, each its own modifier object.
    pub children: Vec<Modifiers>,
    pub unheld: Unheld,
}

/// `targeted_modifier`: its tag and its own modifier object.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TargetedModifier {
    /// Absent when the target is omitted or requires runtime evaluation.
    pub tag: Option<String>,
    /// Original string bytes for the country operand. Unlike the
    /// specifier's uppercase copy, scoped-variable parsing uses these
    /// bytes unchanged when the tag lookup fails.
    pub operand: Option<Vec<u8>>,
    pub modifiers: Modifiers,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EquipmentBonus {
    pub equipment: String,
    pub instant: bool,
    /// One entry per stat; a repeated stat adds its value.
    pub values: Vec<Modifier>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Node {
    /// A modifier object and its fields.
    Modifier,
    /// The `hidden_modifier` loop: one modifier value per key.
    Hidden,
    /// A `targeted_modifier` object.
    Targeted,
    /// The `equipment_bonus` loop: each entry a specific
    /// equipment bonus object.
    EquipmentBonuses,
    /// `instant` reads a boolean; anything else names an equipment
    /// stat whose value is added to any earlier stored value.
    EquipmentBonus,
    /// A `research_bonus` loop: each entry reads one fixed5 value.
    ResearchBonus,
}

/// A field whose reading the port cannot settle, so what follows it is
/// unknown:
///
/// - a block value the game decides by a name the content does not settle:
///   a registered modifier reads `{` as its fixed5 value and the block
///   contents as enclosing entries. Any other name is reported and its
///   block skipped; equipment stats behave the same way. The replay reads
///   the skip; what follows is known only if the name is not registered;
/// - a field a reader hands to another reader the port does not trace
///   ([`Undecided::untraced`]).
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

/// Whether a name is a registered modifier: `None` only where
/// the supplied recorded-stack roster has an unread suffix or is absent.
pub type Registry<'a> = &'a dyn Fn(&[u8]) -> Option<bool>;

pub const UNKNOWN: Registry<'static> = &|_| None;

/// The modifier readers' dispatch of one field. A block value under a
/// modifier name is read as `registry` decides it; where it does not say,
/// and for an equipment stat whose stat map the port does not hold, it
/// is read as the skip and recorded in `undecided`.
pub fn shape(
    node: Node,
    field: &Field<'_>,
    registry: Registry<'_>,
    undecided: &mut Vec<Undecided>,
) -> Shape<Node> {
    let keyword = field.keyword();
    // A registered name reads one fixed5 value and consumes nothing more.
    // Any other name is reported and its block skipped when the current
    // token is `{`. A punctuation or number key names no modifier.
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
        // `name` and `custom_modifier_tooltip` read strings; `data` reads
        // a signed integer. `hidden_modifier` loops only when its value is
        // `{`, otherwise reporting and skipping a block at the current token.
        // `added_modifier` reads a new child modifier object. Anything else,
        // a quoted key included, reads a modifier value.
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

/// A modifier lookup uses the text of a keyword or quoted token.
/// Punctuation and number tokens never name a modifier.
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
            crate::parser::text(bytes),
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
    /// One entry of a modifier block as dispatched.
    pub fn read(&mut self, item: &Item) {
        match item.keyword().map(|keyword| keyword.folded()).as_deref() {
            Some(b"name" | b"data") => {}
            // Read the string; its FNV-1a hash has no case folding.
            Some(b"custom_modifier_tooltip") => {
                if let Some(text) = self
                    .unheld
                    .keep("custom_modifier_tooltip", item.value.read_str())
                {
                    self.tooltips.insert(text.to_owned());
                }
            }
            // A `{` value opens the loop; any other value is reported,
            // storing nothing.
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

    /// Every entry of another block read into this modifier object
    /// (a repeated `modifier` field reads into the same object).
    pub fn read_all(&mut self, items: &[Item]) {
        for item in items {
            self.read(item);
        }
    }

    /// A new modifier is added; a known one takes the last value
    /// while keeping its place.
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

    /// The custom tooltip keys, one per text, listed from the first
    /// table slot.
    pub fn tooltips(&self) -> Vec<String> {
        self.tooltips.keys().map(str::to_owned).collect()
    }

    pub fn tooltip_table(&self) -> &TooltipTable {
        &self.tooltips
    }

    /// Whether the tooltip lists a modifier: hidden entries are skipped.
    pub fn listed(&self) -> impl Iterator<Item = &Modifier> {
        self.values
            .iter()
            .filter(|modifier| !self.hidden.contains(&modifier.name))
    }
}

/// One tooltip-table slot: the key's hash, its probe distance plus
/// one (0 empty, 0xFF the end sentinel) and the key.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Slot {
    hash: u32,
    distance: u8,
    key: String,
}

/// The accumulator's load factor, 0.9 as f32; a copy keeps the source's.
const LOAD: f32 = f32::from_bits(0x3f66_6666);

/// The `custom_modifier_tooltip` string set: insertion, growth, rehashing
/// and copying ([`Self::copied`]) preserve the slot order used to list
/// tooltip text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TooltipTable {
    slots: Vec<Slot>,
    count: i32,
    mask: u32,
    /// The longest distance plus one a slot may take: `n + 2`.
    overflow: u8,
    /// Still the shared initial static table.
    shared: bool,
}

impl Default for TooltipTable {
    /// The shared initial static table: an empty slot then the sentinel,
    /// mask 0.
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
    /// The keys in tooltip display slot order.
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

    /// Copy to the default table for an empty source; otherwise rehash to
    /// 2^ceil(log2(int(max(1.9, 1/load) * count + 1))) buckets without
    /// reinserting, then copy the source's keys in slot order with their
    /// stored hashes.
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

    /// Insert or find with the key's FNV-1a hash, without case folding.
    fn insert(&mut self, key: String) {
        let hash = key.bytes().fold(0x811c_9dc5_u32, |hash, byte| {
            (hash ^ u32::from(byte)).wrapping_mul(0x0100_0193)
        });
        self.insert_hashed(hash, key);
    }

    /// The probe from `hash & mask` compares keys while the distance stays
    /// within the occupant's. Grow first when the distance passes the
    /// overflow or `(count + 1) / mask` (f32, a zero mask dividing to
    /// infinity) passes the load. The key takes the slot and carries the
    /// occupant on, swapping with any occupant nearer its home. A carried
    /// distance past the overflow puts the carried entry in the key's slot,
    /// takes the key back out, grows the table and inserts it again.
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

    /// Grow from a mask of 0, 1 or 3 to 2^3 buckets; otherwise double.
    fn grow(&mut self) {
        let power = match self.mask {
            0 | 1 | 3 => 3,
            mask => (mask + 1).trailing_zeros() + 1,
        };
        self.rehash(power);
    }

    /// Rehash to 2^power buckets, power + 2 overflow slots and the sentinel;
    /// the old occupied slots, up to the count, are inserted again in slot
    /// order with their hashes. Leaving the shared static table inserts
    /// nothing. A power outside 1..=30 reports invalid growth and keeps
    /// the table.
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

/// Every entry of a modifier block in document order.
pub fn values(items: &[Item]) -> Modifiers {
    let mut result = Modifiers::default();
    result.read_all(items);
    result
}

/// A `targeted_modifier` block as read.
pub fn targeted(items: &[Item]) -> TargetedModifier {
    let mut tag = None;
    let mut operand = None;
    let mut modifiers = Modifiers::default();
    for item in items {
        match item.keyword().map(|keyword| keyword.folded()).as_deref() {
            // Read the string and uppercase it before parsing the tag.
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
        // `event_target:…`, `var:…`, `[…]` and an undefined `@…` name a
        // country only at run time; failed tag lookup leaves their text
        // to be parsed as a scoped variable.
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
                    // Exactly `yes` or `no`; a failed boolean conversion leaves
                    // it as it was.
                    instant = field
                        .value
                        .scalar()
                        .and_then(crate::parser::read_bool)
                        .unwrap_or(instant);
                    continue;
                }
                let Some(stat) = value(field, unheld) else {
                    continue;
                };
                // A stat read again adds to the one stored.
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

/// A `research_bonus` block: each entry a new bonus with a zeroed
/// fixed5 destination.
pub fn research_bonuses(items: &[Item], unheld: &mut Unheld) -> Vec<Modifier> {
    items
        .iter()
        .filter_map(|item| value(item, unheld))
        .collect()
}

/// Failed numeric conversions, named and counted without changing
/// the stored value the game retains.
#[derive(Default)]
pub(crate) struct ScanFailures {
    count: usize,
    examples: Vec<String>,
    distinct: Option<BTreeSet<(String, String, String)>>,
    unknown_modifiers: Vec<String>,
}

impl ScanFailures {
    pub(crate) fn distinct() -> Self {
        Self {
            distinct: Some(BTreeSet::new()),
            ..Self::default()
        }
    }

    pub(crate) fn note(&mut self, owner: &str, key: &str, text: &str) {
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

    pub(crate) fn fixed5(&mut self, owner: &str, item: &Item) {
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

    pub(crate) fn int32(&mut self, owner: &str, item: &Item) {
        if item.kind == native::Kind::Open {
            return;
        }
        if let Some(text) = item.value.scalar()
            && !matches!(crate::crt::scan_i32(text), crate::crt::Scanned::Stored(_))
        {
            self.note(
                owner,
                item.key_str().unwrap_or("<non-text key>"),
                &String::from_utf8_lossy(text),
            );
        }
    }

    pub(crate) fn scan<'a>(&mut self, owner: &str, values: impl IntoIterator<Item = &'a Modifier>) {
        for value in values {
            if crate::fixed5::script_fixed5(value.text.as_bytes()).is_none() {
                self.note(owner, &value.key, &value.text);
            }
        }
    }

    pub(crate) fn modifiers(&mut self, owner: &str, values: &Modifiers) {
        self.scan(owner, &values.values);
        for child in &values.children {
            self.modifiers(owner, child);
        }
    }

    /// An unrecognized modifier field is looked up before its fixed5 value
    /// is read; report key lookup failure for a failed text value. Scripted
    /// and sub-unit names come from the recorded stack, and built-in names
    /// from the shared modifier registry.
    pub(crate) fn idea_modifiers(&mut self, owner: &str, values: &Modifiers, names: &KeyNames) {
        for value in &values.values {
            if crate::fixed5::script_fixed5(value.text.as_bytes()).is_some() {
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

    pub(crate) fn warning(&self) -> Option<String> {
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

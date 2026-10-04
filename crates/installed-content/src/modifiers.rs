//! Modifier blocks as ideas, traits and terrain author them: `key = value`
//! pairs whose keys are modifier names, plus the few forms around them
//! (`custom_modifier_tooltip`, `hidden_modifier`, `added_modifier`,
//! targeted and equipment modifiers).
//!
//! The file's reader replay ([`crate::parser::native`]) reads them: [`shape`]
//! is the modifier readers' field dispatch, which a reader's own dispatch
//! embeds as [`Node`]; [`Modifiers::read`] then stores the items the replay
//! returns (variables already resolved, 1424AB540) as CModifier's field
//! dispatch (1406031A0) stores them:
//!
//! - a modifier value (1406042A0) is one entry per modifier, whatever the
//!   key's case (the lexer folds a bare key, 1424A7A90; a quoted key is
//!   looked up through FindTokenNoCase, 1424A7060), its fixed5 starting at 0
//!   and read by ReadFixed5 (1424ABB10 -> 1424B0480); a repeated modifier
//!   keeps its place and takes the last value;
//! - `hidden_modifier` stores its entries the same way and marks them
//!   hidden (+0x88), so the tooltip never lists them (FormatLine 14054F6D0);
//! - `custom_modifier_tooltip` keys go into a string table (+0x68,
//!   CPdxRobinHoodTable_String::FindOrInsert 140546110), one per text,
//!   listed in the table's slot order (FormatDescription 14054E050);
//! - `added_modifier` is a child CModifier (+0x18), which the tooltip does
//!   not list.

use crate::parser::{
    Item, Unheld, Value,
    native::{self, Field, Shape},
};
use std::collections::BTreeSet;

/// One modifier value as 1406042A0 stores it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Modifier {
    /// The key as authored.
    pub key: String,
    /// The modifier it names, as the game identifies it: the key ASCII-folded
    /// (1424A7A90 folds a bare key before its lookup, FindTokenNoCase
    /// 1424A7060 a quoted one). Entries merge and hide, and join the
    /// modifier definitions, by this.
    pub name: String,
    /// The value token's text (after 1424AB540), kept apart from the value.
    pub text: String,
    /// The stored fixed5 ([`stored`]).
    pub value: i64,
}

/// The fixed5 1406042A0 and the entry readers (140625D00, 140FBF890,
/// 1413CA860) store: the destination starts at 0 and ReadFixed5 (1424ABB10 ->
/// 1424B0480) converts the value token's text with the CRT's `%lli`, so a
/// numeric prefix reads (`1tail` is 1), `0x10` is hexadecimal and an empty
/// text is 0; a text it cannot convert (`yes`, `[?x]`, an undefined `@name`)
/// is reported (1424ACEE0) and leaves the 0, which the game then holds.
pub fn stored(item: &Item) -> i64 {
    read_fixed5(item).unwrap_or(0)
}

/// CReader::ReadFixed5 (1424ABB10 -> 1424B0480): the conversion, `None` when
/// it fails and the destination keeps what it held.
pub fn read_fixed5(item: &Item) -> Option<i64> {
    item.value.scalar().and_then(crate::fixed5::script_fixed5)
}

/// A modifier block as one CModifier holds it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Modifiers {
    /// One entry per modifier, in the order first read.
    pub values: Vec<Modifier>,
    /// The modifiers `hidden_modifier` named (+0x88): stored in `values`,
    /// never listed.
    pub hidden: BTreeSet<String>,
    tooltips: TooltipTable,
    /// `added_modifier` blocks (+0x18), each its own CModifier.
    pub children: Vec<Modifiers>,
    /// A key, value text or tooltip the game stores that the port cannot
    /// hold: the reader owning the block refuses it.
    pub unheld: Unheld,
}

/// `targeted_modifier` as CStaticModifier (1406039B0) reads it: its tag and
/// its own CModifier.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TargetedModifier {
    /// None when the block names no tag or one decided at run time.
    pub tag: Option<String>,
    pub modifiers: Modifiers,
}

/// Modifiers to one equipment type or archetype, as `equipment_bonus` authors them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EquipmentBonus {
    pub equipment: String,
    /// Applies at once to existing equipment rather than to new production.
    pub instant: bool,
    /// One entry per stat; a repeated stat adds its value (140625D00).
    pub values: Vec<Modifier>,
}

/// The modifier readers, as a reader's dispatch embeds them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Node {
    /// A CModifier (CPersistent::Read with CModifier::vf4 1406039A0, whose
    /// field dispatch is 1406031A0).
    Modifier,
    /// 1406031A0's `hidden_modifier` loop: 1406042A0 per key.
    Hidden,
    /// A CStaticModifier (`targeted_modifier`, CStaticModifier::vf4
    /// 1406039B0).
    Targeted,
    /// `equipment_bonus`'s own loop (CIdea::vf4 140FBF9C0,
    /// CCountryLeaderTrait::vf4 140713020): each entry a CTraitEquipmentBonus,
    /// read by CSpecificEquipmentBonus::vf3 (1406255F0 -> CPersistent::Read).
    EquipmentBonuses,
    /// CSpecificEquipmentBonus::vf4 (140625FD0): `instant` (0x3474) through
    /// read_bool_field, anything else a stat of its CEquipmentStats
    /// (140625E90 -> ReadAndCombineStatToken 140625D00).
    EquipmentBonus,
    /// A `research_bonus` loop: each entry through ReadFixed5
    /// (CIdeaResearchBonus::vf3 140FBF890, SAdvisorBonus::vf3 1413CA860).
    ResearchBonus,
}

/// A field whose reading the port cannot settle, so what follows it is
/// unknown:
///
/// - a block value the game decides by a name the content does not settle:
///   1406042A0 reads a registered modifier through ReadFixed5 (so a `{` is
///   its value and the block's contents are the enclosing entries) and
///   records any other name (1424ACF90), which skips the block; 140625D00
///   does the same for an equipment stat. The replay reads the skip; what
///   follows is known only if the name is not registered;
/// - a field a reader hands to another reader the port does not trace
///   ([`Undecided::untraced`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Undecided {
    pub name: String,
    pub line: u32,
    /// The reader whose registry decides, or which is not traced.
    pub reader: &'static str,
    pub untraced: bool,
}

impl Undecided {
    /// `field` goes to `reader`, which the port does not trace.
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

/// Whether a name is a registered modifier (1406042A0), as far as the
/// caller knows: `None` when it does not. A recording's modifier
/// definitions answer it; content alone does not.
pub type Registry<'a> = &'a dyn Fn(&[u8]) -> Option<bool>;

/// The registry of a reader that has no recording: it knows no name.
pub const UNKNOWN: Registry<'static> = &|_| None;

/// The modifier readers' dispatch of one field. A block value under a
/// modifier name is read as `registry` decides it (1406042A0); where it
/// does not say, and for an equipment stat (140625D00, whose stat map the
/// port does not hold), it is read as the skip and recorded in
/// `undecided`.
pub fn shape(
    node: Node,
    field: &Field<'_>,
    registry: Registry<'_>,
    undecided: &mut Vec<Undecided>,
) -> Shape<Node> {
    let keyword = field.keyword();
    // A registered name is read through ReadFixed5, which consumes nothing
    // more; any other through RecordError (1424ACF90), whose skip
    // (1424AD270) passes over a block when the current token is `{`. A
    // punctuation or number key is no name (its token id names none).
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
        // 1406031A0: `name` 0x1B through ReadStringInto, `data` 0xF0 through
        // ReadScriptSignedInt32, `custom_modifier_tooltip` 0x300 through
        // ReadStringInto; `hidden_modifier` 0x301 a loop only when the value
        // is `{` (else RecordError, which skips a block the reader stands
        // on, 1424AD270); `added_modifier`
        // 0x311 a new CModifier through its Read (vtable +0x18); anything
        // else, a quoted key included, a modifier value (1406042A0).
        Node::Modifier => match keyword.as_deref() {
            Some(b"name" | b"data" | b"custom_modifier_tooltip") => Shape::One,
            Some(b"hidden_modifier") => Shape::ObjectOrSkip(Node::Hidden),
            Some(b"added_modifier") => Shape::Object(Node::Modifier),
            _ => decide("1406042A0", registry),
        },
        Node::Hidden => decide("1406042A0", registry),
        // 1406039B0: `key` 0xDC and `tag` 0x2A02 through ReadStringInto;
        // `valid_relation_trigger` 0x37C3 a CAndTrigger (vtable +0x28,
        // LoadBody 1405431C0), its fields read by key (the key-validity
        // gap); anything else to 1406031A0.
        Node::Targeted => match keyword.as_deref() {
            Some(b"key" | b"tag") => Shape::One,
            Some(b"valid_relation_trigger") => Shape::Trigger,
            _ => shape(Node::Modifier, field, registry, undecided),
        },
        Node::EquipmentBonuses => Shape::Object(Node::EquipmentBonus),
        Node::EquipmentBonus if field.keyword_is("instant") => Shape::One,
        Node::EquipmentBonus => decide("140625D00", UNKNOWN),
        Node::ResearchBonus => Shape::One,
    }
}

/// The name 1406042A0 looks a modifier up by, as text: a keyword's or a
/// quoted token's (FindTokenFromSsoString 1424A7060). Punctuation and number
fn modifier_name<'a>(item: &'a Item, unheld: &mut Unheld) -> Option<&'a str> {
    if item.keyword().is_none() && !item.key_quoted {
        return None;
    }
    unheld.keep("a modifier key", item.key_str())
}

/// A modifier value from one entry, when its key can name one.
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
    /// One entry of a CModifier block as 1406031A0 dispatched it.
    pub fn read(&mut self, item: &Item) {
        match item.keyword().map(|keyword| keyword.folded()).as_deref() {
            // The modifier's own fields, which no tooltip shows.
            Some(b"name" | b"data") => {}
            // ReadStringInto(..., 0) (1424ABB50), its FNV-1a (no folding)
            // the table's hash.
            Some(b"custom_modifier_tooltip") => {
                if let Some(text) = self
                    .unheld
                    .keep("custom_modifier_tooltip", item.value.read_str())
                {
                    self.tooltips.insert(text.to_owned());
                }
            }
            // A `{` value is the loop; any other RecordError (1424ACF90),
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

    /// Every entry of another block read into this CModifier (a reader that
    /// reads `modifier` again reads into the same object).
    pub fn read_all(&mut self, items: &[Item]) {
        for item in items {
            self.read(item);
        }
    }

    /// 1406042A0's insert: a new modifier is added; a known one takes the
    /// value ("specified multiple time, last value will be used").
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

    /// The custom tooltip keys, one per text, in the order FormatDescription
    /// (14054E050) lists them: the table's slots from the first.
    pub fn tooltips(&self) -> Vec<String> {
        self.tooltips.keys().map(str::to_owned).collect()
    }

    /// The custom tooltip set itself, for a reader that copies it.
    pub fn tooltip_table(&self) -> &TooltipTable {
        &self.tooltips
    }

    /// Whether the tooltip lists a modifier: FormatLine (14054F6D0) skips a
    /// hidden one.
    pub fn listed(&self) -> impl Iterator<Item = &Modifier> {
        self.values
            .iter()
            .filter(|modifier| !self.hidden.contains(&modifier.name))
    }
}

/// One slot of CPdxRobinHoodTable (pdx_robin_hood_table.h): the key's
/// hash, its probe distance plus one (0 empty, 0xFF the end sentinel) and
/// the key.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Slot {
    hash: u32,
    distance: u8,
    key: String,
}

/// The accumulator's load factor, 0.9 as f32 (InitializeModifierAccumulator
/// 140549470; the copy 1405492D0 keeps the source's).
const LOAD: f32 = f32::from_bits(0x3f66_6666);

/// CPdxRobinHoodTable_String as CModifier's `custom_modifier_tooltip` set
/// (+0x68) holds it: FindOrInsert 140546110 / 140546E60, Grow 1405521B0,
/// Rehash 140604920, and the copy ModifierAccumulator_CopyConstruct
/// 1405492D0 -> 14054BA50 ([`Self::copied`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TooltipTable {
    /// `2^n` buckets, `n + 2` overflow slots, then the sentinel.
    slots: Vec<Slot>,
    count: i32,
    mask: u32,
    /// The longest distance plus one a slot may take (+0x18): `n + 2`.
    overflow: u8,
    /// Still the shared static table of InitializeModifierAccumulator.
    shared: bool,
}

impl Default for TooltipTable {
    /// InitializeModifierAccumulator (140549470): the shared static table,
    /// an empty slot then the sentinel, mask 0.
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
    /// The keys in slot order, as FormatDescription (14054E050) walks them.
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

    /// The table ModifierAccumulator_CopyConstruct (1405492D0) builds
    /// through 14054BA50: the default table for an empty source, else
    /// Rehash to 2^ceil(log2(int(max(1.9, 1/load) * count + 1))) buckets
    /// without reinserting, then the source's keys in its slot order with
    /// their stored hashes.
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

    /// FindOrInsert with the key's FNV-1a (1406031A0, no folding).
    fn insert(&mut self, key: String) {
        let hash = key.bytes().fold(0x811c_9dc5_u32, |hash, byte| {
            (hash ^ u32::from(byte)).wrapping_mul(0x0100_0193)
        });
        self.insert_hashed(hash, key);
    }

    /// 140546110: the probe from `hash & mask` compares keys while the
    /// distance stays within the occupant's; the table grows first when the
    /// distance passes the overflow (+0x18) or `(count + 1) / mask` (f32, a
    /// zero mask dividing to infinity) passes the load. Robin Hood then: the
    /// key takes the slot and the occupant is carried on, swapping with
    /// any occupant nearer its home. A carried distance past the overflow
    /// puts the carried entry in the key's slot, takes the key back out,
    /// grows and inserts it again (140546E60).
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

    /// Grow (1405521B0): from a mask of 0, 1 or 3 to 2^3 buckets, else
    /// double.
    fn grow(&mut self) {
        let power = match self.mask {
            0 | 1 | 3 => 3,
            mask => (mask + 1).trailing_zeros() + 1,
        };
        self.rehash(power);
    }

    /// Rehash (140604920): 2^power buckets, power + 2 overflow slots and
    /// the sentinel; the old occupied slots, up to the count, are inserted
    /// again in slot order with their hashes. Leaving the shared static
    /// table inserts nothing. A power outside 1..=30 logs "Invalid rehash
    /// requested" and keeps the table.
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

/// Every entry of a CModifier block, in document order.
pub fn values(items: &[Item]) -> Modifiers {
    let mut result = Modifiers::default();
    result.read_all(items);
    result
}

/// A `targeted_modifier` block as CStaticModifier (1406039B0) read it.
pub fn targeted(items: &[Item]) -> TargetedModifier {
    let mut tag = None;
    let mut modifiers = Modifiers::default();
    for item in items {
        match item.keyword().map(|keyword| keyword.folded()).as_deref() {
            // ReadStringInto, then SsoStringToUpperInPlace (1424B87F0)
            // before the tag is parsed (140BA4F40).
            Some(b"key" | b"tag") => {
                tag = modifiers
                    .unheld
                    .keep("a targeted modifier's tag", item.value.read_str())
                    .map(str::to_ascii_uppercase);
            }
            Some(b"valid_relation_trigger") => {}
            _ => modifiers.read(item),
        }
    }
    TargetedModifier {
        // `event_target:…`, `var:…`, `[…]` and an undefined `@…` name a
        // country only at run time (HasCountrySpecifier 140BA6220 fails and
        // the text becomes a scoped variable, 140539AF0).
        tag: tag.filter(|tag| !tag.contains(['[', '@', ':'])),
        modifiers,
    }
}

/// An `equipment_bonus` block: each entry an equipment type or archetype
/// (its key's text) with `instant` and its stats.
pub fn equipment_bonuses(items: &[Item], unheld: &mut Unheld) -> Vec<EquipmentBonus> {
    items
        .iter()
        .filter_map(|item| {
            let equipment = unheld.keep("an equipment bonus's equipment", item.key_str())?;
            let mut instant = false;
            let mut values: Vec<Modifier> = Vec::new();
            for field in item.value.items().unwrap_or_default() {
                if field.keyword_is("instant") {
                    // read_bool_field (1424ABCA0 -> read_bool_token
                    // 1424B06E0): exactly `yes` or `no`; a failure leaves it
                    // as it was.
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
                // ReadAndCombineStatToken (140625D00): a stat read again
                // adds to the one stored ("combining values").
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

/// A `research_bonus` block: each entry a new bonus (140FBF9C0 allocates
/// one per entry, its value +0x38 zeroed) read by ReadFixed5.
pub fn research_bonuses(items: &[Item], unheld: &mut Unheld) -> Vec<Modifier> {
    items
        .iter()
        .filter_map(|item| value(item, unheld))
        .collect()
}

/// Failed native number conversions, named and counted without changing
/// the stored value the game retains (ReadFixed5 1424ABB10, ReadInt 1424AB970).
#[derive(Default)]
pub(crate) struct ScanFailures {
    count: usize,
    examples: Vec<String>,
    distinct: Option<BTreeSet<(String, String, String)>>,
}

impl ScanFailures {
    /// Character alternatives repeat their common body; count each problem once.
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
        // Shape::One keeps the opening token as scalar text; the reader already
        // names this block as a misread, rather than a scalar conversion failure.
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

    pub(crate) fn warning(&self) -> Option<String> {
        let more = match self.count - self.examples.len() {
            0 => String::new(),
            more => format!("; and {more} more"),
        };
        (self.count > 0).then(|| {
            format!(
                "{} content values failed native numeric conversion; the game's stored \
                 destinations are retained: {}{more}.",
                self.count,
                self.examples.join("; "),
            )
        })
    }
}

#[cfg(test)]
mod tests;

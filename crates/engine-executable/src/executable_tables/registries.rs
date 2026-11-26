use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    fmt::Write as _,
    io,
};

use iced_x86::{
    Code, Decoder, DecoderOptions, FlowControl, Instruction, Mnemonic, OpKind, Register,
};
use sha2::{Digest, Sha256};

use super::{
    keywords::KeywordTables,
    leak_string,
    pe::{Anchor, Hashed, PeImage},
};

mod pinned;

const ENTRY_FACTORY: u32 = 8;
const TRIGGER_VALUE_READER: u32 = 40;
const TRIGGER_KEY_READER: u32 = 48;
const TRIGGER_SCOPE_MASK: u32 = 112;
const EFFECT_VALUE_READER: u32 = 24;
const EFFECT_KEY_READER: u32 = 32;

const TRIGGER_ENTRY: Anchor = Anchor {
    what: "the trigger registry entry template",
    text: Hashed {
        length: 20,
        fnv: 0xa663d7d6fe7f1da5,
    },
};
const EFFECT_ENTRY: Anchor = Anchor {
    what: "the effect registry entry template",
    text: Hashed {
        length: 19,
        fnv: 0x31c287a54eef8cbe,
    },
};
const TRIGGER_BASE: Anchor = Anchor {
    what: "the trigger base class",
    text: Hashed {
        length: 14,
        fnv: 0xca718c196690dae2,
    },
};
const ASSIGN_TRIGGER: Anchor = Anchor {
    what: "the assign trigger class",
    text: Hashed {
        length: 26,
        fnv: 0xe0a7399faa73c7af,
    },
};
const COMPARISON_TRIGGERS: [Anchor; 2] = [
    Anchor {
        what: "the integer comparison trigger class",
        text: Hashed {
            length: 17,
            fnv: 0xae16fcd95957b39d,
        },
    },
    Anchor {
        what: "the value comparison trigger class",
        text: Hashed {
            length: 19,
            fnv: 0x666727962e04ae8d,
        },
    },
];
const EFFECT_BASE: Anchor = Anchor {
    what: "the effect base class",
    text: Hashed {
        length: 13,
        fnv: 0x460da4801231ad57,
    },
};
const CONTEXT_EFFECTS: [Anchor; 2] = [
    Anchor {
        what: "the context effect class",
        text: Hashed {
            length: 20,
            fnv: 0xdd2d190e7137c56c,
        },
    },
    Anchor {
        what: "the scripted effect class",
        text: Hashed {
            length: 21,
            fnv: 0xbb1b503a9fc6ec09,
        },
    },
];

pub const UNMATCHED_IDENTITY: u64 = 1 << 63;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TriggerReader {
    Assign,
    Comparison,
    Body,
    Own(u64),
}

#[derive(Debug, Default)]
pub struct RegistryTables {
    pub triggers: Vec<(u32, &'static str)>,
    pub trigger_classes: Vec<(u32, u32, TriggerReader)>,
    pub trigger_key_readers: Vec<(u32, u64)>,
    pub effects: Vec<(u32, &'static str)>,
    pub effect_classes: Vec<(u32, u64, u64)>,
    pub effect_identities: Vec<(u32, &'static str, u64)>,
    pub(super) context_effect_names: BTreeMap<u64, &'static str>,
    pub(super) unmatched: BTreeMap<u64, u64>,
}

impl RegistryTables {
    pub fn content_digest(&self) -> [u8; 32] {
        let identity = |value: u64| match self.unmatched.get(&value) {
            Some(stand_in) => format!("?{stand_in:016x}"),
            None => format!("{value:x}"),
        };
        let mut text = String::new();
        for ((&(token, name), &(_, mask, reader)), &(_, key)) in self
            .triggers
            .iter()
            .zip(&self.trigger_classes)
            .zip(&self.trigger_key_readers)
        {
            let reader = match reader {
                TriggerReader::Assign => "assign".to_owned(),
                TriggerReader::Comparison => "comparison".to_owned(),
                TriggerReader::Body => "body".to_owned(),
                TriggerReader::Own(value) => identity(value),
            };
            writeln!(
                text,
                "trigger\t{token}\t{name}\t{mask}\t{reader}\t{}",
                identity(key)
            )
            .unwrap();
        }
        for ((&(token, name), &(_, value, key)), &(_, class, vtable)) in self
            .effects
            .iter()
            .zip(&self.effect_classes)
            .zip(&self.effect_identities)
        {
            writeln!(
                text,
                "effect\t{token}\t{name}\t{class}\t{}\t{}\t{}",
                identity(vtable),
                identity(value),
                identity(key)
            )
            .unwrap();
        }
        for (&vtable, name) in &self.context_effect_names {
            writeln!(text, "context\t{name}\t{}", identity(vtable)).unwrap();
        }
        Sha256::digest(text.as_bytes()).into()
    }

    pub fn effect_class_name(&self, vtable: u64) -> Option<&'static str> {
        self.effect_identities
            .iter()
            .find_map(|&(_, name, identity)| (identity == vtable).then_some(name))
            .or_else(|| self.context_effect_names.get(&vtable).copied())
    }

    pub fn identities_matched(&self) -> bool {
        self.unmatched.is_empty()
    }

    pub fn unmatched_identities(&self) -> usize {
        self.unmatched.len()
    }

    pub(super) fn decode(image: &PeImage, keywords: &KeywordTables) -> io::Result<Self> {
        let raw = Raw::decode(image, keywords)?;
        let mut code = Fingerprints::new(image);
        for (table, vtable, expected) in [
            ("triggers", raw.trigger_base, pinned::TRIGGER_LAYOUT),
            ("effects", raw.effect_base, pinned::EFFECT_LAYOUT),
        ] {
            if code.layout(vtable)? != expected {
                return Err(invalid(format!(
                    "{table}: the base class's virtual functions differ from the recognised build, so its reader slots are unconfirmed"
                )));
            }
        }
        let mut identities = Identities {
            code,
            ambiguous: &raw.ambiguous,
            unmatched: BTreeMap::new(),
        };
        for (class, reader) in raw
            .reader_classes
            .into_iter()
            .zip([raw.assign, raw.comparison, raw.body])
        {
            identities.function(class, TRIGGER_VALUE_READER, reader);
        }
        let mut result = Self::default();
        for row in &raw.triggers {
            if let Ok(index) =
                pinned::SCOPE_MASKS.binary_search_by_key(&row.token, |&(token, _)| token)
                && pinned::SCOPE_MASKS[index].1 != row.mask
            {
                return Err(invalid(format!(
                    "trigger classes: keyword {}: scope mask {:#x} differs from the recognised build's {:#x}, whose scope bits readers assume",
                    row.token,
                    row.mask,
                    pinned::SCOPE_MASKS[index].1
                )));
            }
            let reader = if row.value == raw.assign {
                TriggerReader::Assign
            } else if row.value == raw.comparison {
                TriggerReader::Comparison
            } else if row.value == raw.body {
                TriggerReader::Body
            } else {
                TriggerReader::Own(identities.function(row.class, TRIGGER_VALUE_READER, row.value))
            };
            let key = identities.function(row.class, TRIGGER_KEY_READER, row.key);
            result.triggers.push((row.token, row.name));
            result.trigger_classes.push((row.token, row.mask, reader));
            result.trigger_key_readers.push((row.token, key));
        }
        for row in &raw.effects {
            let value = identities.function(row.class, EFFECT_VALUE_READER, row.value);
            let key = identities.function(row.class, EFFECT_KEY_READER, row.key);
            let class = identities.class(row.class, row.vtable);
            result.effects.push((row.token, row.name));
            result.effect_classes.push((row.token, value, key));
            result.effect_identities.push((
                row.token,
                leak_string(project_class_name(&canonical(row.class))?),
                class,
            ));
        }
        for &(decorated, vtable) in &raw.contexts {
            let class = identities.class(decorated, vtable);
            result.context_effect_names.insert(
                class,
                leak_string(project_class_name(&canonical(decorated))?),
            );
        }
        result.unmatched = identities.unmatched;
        Ok(result)
    }
}

struct TriggerRow<'a> {
    token: u32,
    name: &'static str,
    class: &'a str,
    vtable: u32,
    value: u32,
    key: u32,
    mask: u32,
}

struct EffectRow<'a> {
    token: u32,
    name: &'static str,
    class: &'a str,
    vtable: u32,
    value: u32,
    key: u32,
}

struct Raw<'a> {
    triggers: Vec<TriggerRow<'a>>,
    effects: Vec<EffectRow<'a>>,
    trigger_base: u32,
    effect_base: u32,
    assign: u32,
    comparison: u32,
    body: u32,
    reader_classes: [&'a str; 3],
    contexts: Vec<(&'a str, u32)>,
    ambiguous: BTreeSet<u64>,
}

impl<'a> Raw<'a> {
    fn decode(image: &'a PeImage, keywords: &KeywordTables) -> io::Result<Self> {
        let exact: Vec<Anchor> = [TRIGGER_BASE, ASSIGN_TRIGGER, EFFECT_BASE]
            .into_iter()
            .chain(COMPARISON_TRIGGERS)
            .chain(CONTEXT_EFFECTS)
            .collect();
        let classes = primary_vtables(image, |name| {
            TRIGGER_ENTRY.text.prefixes(name.as_bytes())
                || EFFECT_ENTRY.text.prefixes(name.as_bytes())
                || exact.iter().any(|anchor| anchor.text.matches(name.as_bytes()))
        })?;
        let class = |anchor: Anchor| {
            classes
                .iter()
                .find(|(name, _)| anchor.text.matches(name.as_bytes()))
                .map(|(&name, &vtable)| (name, vtable))
                .ok_or_else(|| invalid(format!("registries: {} has no virtual table", anchor.what)))
        };
        let named = |anchor: Anchor| class(anchor).map(|(_, vtable)| vtable);
        let entries = |prefix: Anchor| -> Vec<u32> {
            let mut entries: Vec<u32> = classes
                .iter()
                .filter(|(name, _)| prefix.text.prefixes(name.as_bytes()))
                .map(|(_, &vtable)| vtable)
                .collect();
            entries.sort_unstable();
            entries
        };
        let trigger_entries = entries(TRIGGER_ENTRY);
        let effect_entries = entries(EFFECT_ENTRY);
        let located = locate_registrations(image, &trigger_entries, &effect_entries)?;
        let [trigger_sites, effect_sites] = call_sites(image, [located.trigger, located.effect])?;
        let triggers = registrations(
            image,
            located.allocation,
            located.trigger,
            trigger_sites,
            "triggers",
        )?;
        let effects = registrations(
            image,
            located.allocation,
            located.effect,
            effect_sites,
            "effects",
        )?;

        let trigger_base = named(TRIGGER_BASE)?;
        let effect_base = named(EFFECT_BASE)?;
        let body = code_rva(image, checked_add(trigger_base, TRIGGER_VALUE_READER)?)?;
        let assign = code_rva(
            image,
            checked_add(named(ASSIGN_TRIGGER)?, TRIGGER_VALUE_READER)?,
        )?;
        let [comparison, other] = [COMPARISON_TRIGGERS[0], COMPARISON_TRIGGERS[1]].map(|name| {
            named(name)
                .and_then(|vtable| code_rva(image, checked_add(vtable, TRIGGER_VALUE_READER)?))
        });
        let comparison = comparison?;
        if comparison != other? {
            return Err(invalid(
                "trigger classes: the comparison classes inherit different readers",
            ));
        }
        if assign == body || comparison == body || comparison == assign {
            return Err(invalid(
                "trigger classes: the assign, comparison and body readers are not distinct",
            ));
        }

        let mut result = Self {
            triggers: Vec::with_capacity(triggers.len()),
            effects: Vec::with_capacity(effects.len()),
            trigger_base,
            effect_base,
            assign,
            comparison,
            body,
            reader_classes: [
                class(ASSIGN_TRIGGER)?.0,
                class(COMPARISON_TRIGGERS[0])?.0,
                class(TRIGGER_BASE)?.0,
            ],
            contexts: Vec::with_capacity(CONTEXT_EFFECTS.len()),
            ambiguous: BTreeSet::new(),
        };
        let mut factories = Factories::new(image, located.allocation);
        for (token, entry) in triggers {
            let name = keyword(keywords, token, "triggers")?;
            let (vtable, class) = registered_class(
                image,
                &mut factories,
                &trigger_entries,
                entry,
                TRIGGER_ENTRY,
                TRIGGER_BASE,
            )
            .map_err(|error| invalid(format!("triggers: keyword {token}: {error}")))?;
            let mask = code_rva(image, checked_add(vtable, TRIGGER_SCOPE_MASK)?)?;
            result.triggers.push(TriggerRow {
                token,
                name,
                class,
                vtable,
                value: code_rva(image, checked_add(vtable, TRIGGER_VALUE_READER)?)?,
                key: code_rva(image, checked_add(vtable, TRIGGER_KEY_READER)?)?,
                mask: constant_return(image, mask)?,
            });
        }
        for (token, entry) in effects {
            let name = keyword(keywords, token, "effects")?;
            let (vtable, class) = registered_class(
                image,
                &mut factories,
                &effect_entries,
                entry,
                EFFECT_ENTRY,
                EFFECT_BASE,
            )
            .map_err(|error| invalid(format!("effects: keyword {token}: {error}")))?;
            result.effects.push(EffectRow {
                token,
                name,
                class,
                vtable,
                value: code_rva(image, checked_add(vtable, EFFECT_VALUE_READER)?)?,
                key: code_rva(image, checked_add(vtable, EFFECT_KEY_READER)?)?,
            });
        }
        for anchor in CONTEXT_EFFECTS {
            let vtable = named(anchor)?;
            if !derives(image, vtable, EFFECT_BASE)? {
                return Err(invalid(format!(
                    "effects: {} does not derive from the effect base class",
                    anchor.what
                )));
            }
            result
                .contexts
                .push((decorated_name(image, vtable)?, vtable));
        }
        let mut vtables: HashMap<u64, BTreeSet<u32>> = HashMap::new();
        for (class, vtable) in (result.triggers.iter().map(|row| (row.class, row.vtable)))
            .chain(result.effects.iter().map(|row| (row.class, row.vtable)))
            .chain(result.contexts.iter().copied())
        {
            vtables.entry(name_hash(class)).or_default().insert(vtable);
        }
        result.ambiguous = vtables
            .into_iter()
            .filter(|(_, vtables)| vtables.len() > 1)
            .map(|(hash, _)| hash)
            .collect();
        Ok(result)
    }
}

fn registered_class<'a>(
    image: &'a PeImage,
    factories: &mut Factories<'_>,
    entries: &[u32],
    entry: u32,
    prefix: Anchor,
    base: Anchor,
) -> io::Result<(u32, &'a str)> {
    if entries.binary_search(&entry).is_err() {
        return Err(invalid("registration entry is not a registry entry class"));
    }
    let decorated = decorated_name(image, entry)?;
    let argument = (prefix.text.prefixes(decorated.as_bytes()))
        .then(|| &decorated[prefix.text.length..])
        .and_then(|name| name.strip_suffix("@@"))
        .ok_or_else(|| invalid("registration entry class names no class"))?;
    let vtable = factories.object(entry)?;
    let class = decorated_name(image, vtable)?;
    if class.strip_prefix(".?A") != Some(argument) {
        return Err(invalid(
            "registry factory returns a class other than the one its entry names",
        ));
    }
    if !derives(image, vtable, base)? {
        return Err(invalid(
            "registered class does not derive from the registry base class",
        ));
    }
    Ok((vtable, class))
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

fn checked_add(rva: u32, offset: u32) -> io::Result<u32> {
    rva.checked_add(offset)
        .ok_or_else(|| invalid("registry address overflow"))
}

fn keyword(keywords: &KeywordTables, token: u32, table: &str) -> io::Result<&'static str> {
    keywords
        .by_id(token)
        .ok_or_else(|| invalid(format!("{table}: unknown keyword {token}")))
}

fn is_code(image: &PeImage, rva: u32) -> bool {
    image
        .executable_ranges()
        .iter()
        .any(|&(start, end)| start <= rva && rva < end)
}

fn code_pointer(image: &PeImage, rva: u32) -> io::Result<u64> {
    let address = image.u64(rva)?;
    if !is_code(image, image.va_to_rva(address)?) {
        return Err(invalid("registry reader is outside executable sections"));
    }
    Ok(address)
}

fn code_rva(image: &PeImage, rva: u32) -> io::Result<u32> {
    image.va_to_rva(code_pointer(image, rva)?)
}

fn is_vtable(image: &PeImage, address: u64) -> bool {
    image
        .va_to_rva(address)
        .and_then(|rva| code_pointer(image, rva))
        .is_ok()
}

fn direct_target(image: &PeImage, instruction: &Instruction) -> io::Result<Option<u32>> {
    if matches!(instruction.mnemonic(), Mnemonic::Call | Mnemonic::Jmp)
        && matches!(
            instruction.op0_kind(),
            OpKind::NearBranch64 | OpKind::NearBranch32
        )
    {
        return image.va_to_rva(instruction.near_branch_target()).map(Some);
    }
    Ok(None)
}

fn name_hash(name: &str) -> u64 {
    truncated(Sha256::digest(canonical(name).as_bytes()).into())
}

fn canonical(decorated: &str) -> String {
    let mut result = String::with_capacity(decorated.len());
    let mut rest = decorated;
    while let Some(at) = rest.find("?A0x") {
        result.push_str(&rest[..at + 2]);
        rest = rest[at + 4..].trim_start_matches(|byte: char| byte.is_ascii_hexdigit());
    }
    result.push_str(rest);
    result
}

fn truncated(digest: [u8; 32]) -> u64 {
    u64::from_le_bytes(digest[..8].try_into().unwrap())
}

fn primary_vtables<'a>(
    image: &'a PeImage,
    wanted: impl Fn(&str) -> bool,
) -> io::Result<BTreeMap<&'a str, u32>> {
    let mut locators = BTreeMap::new();
    let mut result = BTreeMap::new();
    for class in image.classes().iter().filter(|class| class.offset == 0) {
        let Ok(name) = image.type_name(class.descriptor) else {
            continue;
        };
        if !wanted(name) {
            continue;
        }
        if locators
            .insert(name, class.locator)
            .is_some_and(|locator| locator != class.locator)
        {
            return Err(invalid(format!(
                "registries: class identity {name} is not unique"
            )));
        }
        if result.insert(name, class.vtable).is_some() {
            return Err(invalid(format!(
                "registries: class identity {name} has several virtual tables"
            )));
        }
    }
    Ok(result)
}

fn locator(image: &PeImage, vtable: u32) -> io::Result<u32> {
    let pointer = vtable
        .checked_sub(8)
        .ok_or_else(|| invalid("class identity address underflow"))?;
    let locator = image.va_to_rva(image.u64(pointer)?)?;
    image.bytes(locator, 24)?;
    if image.u32(locator)? != 1 || image.u32(checked_add(locator, 20)?)? != locator {
        return Err(invalid("class identity locator is inconsistent"));
    }
    Ok(locator)
}

fn decorated_name(image: &PeImage, vtable: u32) -> io::Result<&str> {
    let locator = locator(image, vtable)?;
    let descriptor = image.u32(checked_add(locator, 12)?)?;
    image.bytes(descriptor, 16)?;
    let hierarchy = image.u32(checked_add(locator, 16)?)?;
    image.bytes(hierarchy, 16)?;
    image.string(checked_add(descriptor, 16)?)
}

fn derives(image: &PeImage, vtable: u32, base: Anchor) -> io::Result<bool> {
    let hierarchy = image.u32(checked_add(locator(image, vtable)?, 16)?)?;
    let count = image.u32(checked_add(hierarchy, 8)?)?;
    let array = image.u32(checked_add(hierarchy, 12)?)?;
    let span = usize::try_from(count)
        .ok()
        .and_then(|count| count.checked_mul(4))
        .ok_or_else(|| invalid("class hierarchy overflow"))?;
    for entry in image.bytes(array, span)?.chunks_exact(4) {
        let descriptor = image.u32(u32::from_le_bytes(entry.try_into().unwrap()))?;
        if base
            .text
            .matches(image.string(checked_add(descriptor, 16)?)?.as_bytes())
        {
            return Ok(true);
        }
    }
    Ok(false)
}

fn vtable_references(image: &PeImage, vtables: &[u32]) -> io::Result<Vec<(u32, u32)>> {
    let (Some(&first), Some(&last)) = (vtables.first(), vtables.last()) else {
        return Ok(Vec::new());
    };
    let mut result = Vec::new();
    for &(start, end) in image.executable_ranges() {
        let bytes = image.bytes(start, (end - start) as usize)?;
        for at in memchr::memchr_iter(0x8D, bytes) {
            if at == 0
                || at + 6 > bytes.len()
                || bytes[at - 1] & 0xFB != 0x48
                || bytes[at + 1] & 0xC7 != 0x05
            {
                continue;
            }
            let site = start + at as u32 - 1;
            let displacement = i32::from_le_bytes(bytes[at + 2..at + 6].try_into().unwrap());
            let target = i64::from(site) + 7 + i64::from(displacement);
            let Ok(target) = u32::try_from(target) else {
                continue;
            };
            if target < first || target > last || vtables.binary_search(&target).is_err() {
                continue;
            }
            let mut decoder = Decoder::with_ip(
                64,
                &bytes[at - 1..at + 6],
                image.base + u64::from(site),
                DecoderOptions::NONE,
            );
            let instruction = decoder.decode();
            if instruction.mnemonic() == Mnemonic::Lea
                && instruction.len() == 7
                && instruction.ip_rel_memory_address() == image.base + u64::from(target)
            {
                result.push((site, target));
            }
        }
    }
    Ok(result)
}

struct Located {
    trigger: u32,
    effect: u32,
    allocation: u32,
}

fn locate_registrations(
    image: &PeImage,
    trigger_entries: &[u32],
    effect_entries: &[u32],
) -> io::Result<Located> {
    let mut entries: Vec<u32> = trigger_entries
        .iter()
        .chain(effect_entries)
        .copied()
        .collect();
    entries.sort_unstable();
    let mut triggers = BTreeSet::new();
    let mut effects = BTreeSet::new();
    let mut allocations = BTreeSet::new();
    for (site, vtable) in vtable_references(image, &entries)? {
        let Some((registration, allocation)) = initializer(image, site)? else {
            continue;
        };
        if trigger_entries.binary_search(&vtable).is_ok() {
            triggers.insert(registration);
        } else {
            effects.insert(registration);
        }
        allocations.insert(allocation);
    }
    let one = |found: BTreeSet<u32>, what: &str| match (found.len(), found.first()) {
        (1, Some(&found)) => Ok(found),
        (count, _) => Err(invalid(format!(
            "{what}: the entry initializers name {count} candidates, not one"
        ))),
    };
    let located = Located {
        trigger: one(triggers, "trigger registration")?,
        effect: one(effects, "effect registration")?,
        allocation: one(allocations, "registry allocation")?,
    };
    if located.trigger == located.effect {
        return Err(invalid(
            "trigger and effect registrations are the same function",
        ));
    }
    Ok(located)
}

fn initializer(image: &PeImage, site: u32) -> io::Result<Option<(u32, u32)>> {
    let Ok((start, end)) = image.function(site) else {
        return Ok(None);
    };
    let mut allocation = None;
    let mut tail = None;
    let mut previous: Option<&Instruction> = None;
    let Ok(instructions) = image.instructions(start, end) else {
        return Ok(None);
    };
    for instruction in &instructions {
        if let Ok(Some(target)) = direct_target(image, instruction)
            && !(start..end).contains(&target)
        {
            if instruction.mnemonic() == Mnemonic::Call
                && previous.is_some_and(|previous| {
                    previous.mnemonic() == Mnemonic::Mov
                        && previous.op0_kind() == OpKind::Register
                        && previous.op0_register() == Register::ECX
                        && previous.op1_kind() == OpKind::Immediate32
                })
                && allocation.replace(target).is_some()
            {
                return Ok(None);
            }
            tail = Some(target);
        }
        previous = Some(instruction);
    }
    Ok(allocation
        .zip(tail)
        .map(|(allocation, tail)| (tail, allocation)))
}

fn call_sites<const N: usize>(
    image: &PeImage,
    targets: [u32; N],
) -> io::Result<[BTreeSet<u32>; N]> {
    let mut sites = std::array::from_fn(|_| BTreeSet::new());
    for &(start, end) in image.executable_ranges() {
        let bytes = image.bytes(start, (end - start) as usize)?;
        let Some(last) = bytes.len().checked_sub(5) else {
            continue;
        };
        for offset in memchr::memchr2_iter(0xE8, 0xE9, &bytes[..=last]) {
            let site = checked_add(
                start,
                u32::try_from(offset).map_err(|_| invalid("registry span overflow"))?,
            )?;
            let displacement =
                i32::from_le_bytes(bytes[offset + 1..offset + 5].try_into().unwrap());
            let destination = i64::from(site) + 5 + i64::from(displacement);
            for (&target, found) in targets.iter().zip(&mut sites) {
                if destination == i64::from(target) {
                    found.insert(site);
                }
            }
        }
    }
    Ok(sites)
}

fn registrations(
    image: &PeImage,
    allocation: u32,
    target: u32,
    sites: BTreeSet<u32>,
    table: &str,
) -> io::Result<BTreeMap<u32, u32>> {
    let mut result = BTreeMap::new();
    let mut refused = 0;
    for site in sites {
        let row = (|| {
            let (start, end) = image.function(site)?;
            let instructions = image.instructions(start, end)?;
            let mut state = State::new();
            let mut boundary = false;
            for instruction in &instructions {
                let rva = image.va_to_rva(instruction.ip())?;
                if rva == site {
                    if direct_target(image, instruction)? != Some(target) {
                        return Err(invalid("registration is not an instruction boundary"));
                    }
                    boundary = true;
                    break;
                }
                state.simple(image, instruction)?;
                if let Some(call) = direct_target(image, instruction)?
                    && instruction.mnemonic() == Mnemonic::Call
                {
                    state.clobber();
                    if call == allocation {
                        state.allocate(rva);
                    }
                }
            }
            if !boundary {
                return Err(invalid("registration is not an instruction boundary"));
            }
            let Some(Value::Number(token)) = state.registers.get(&Register::RDX).copied() else {
                return Err(invalid("registration has no keyword argument"));
            };
            let Some(Value::Object(root, offset)) = state.registers.get(&Register::R8).copied()
            else {
                return Err(invalid("registration has no entry object"));
            };
            let entry = state
                .tables
                .get(&(root, offset))
                .copied()
                .ok_or_else(|| invalid("registration has no entry identity"))?;
            let token =
                u32::try_from(token).map_err(|_| invalid("registration keyword overflow"))?;
            Ok((token, entry))
        })();
        match row {
            Ok((token, entry)) => {
                if result
                    .insert(token, entry)
                    .is_some_and(|previous| previous != entry)
                {
                    return Err(invalid(format!(
                        "{table}: conflicting registration for keyword {token}"
                    )));
                }
            }
            Err(_) => refused += 1,
        }
    }
    if refused != 0 {
        return Err(invalid(format!(
            "{table}: {refused} unsupported registration occurrences"
        )));
    }
    if result.is_empty() {
        return Err(invalid(format!("{table}: no registrations")));
    }
    Ok(result)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Value {
    Number(u64),
    Address(u64),
    Object(u32, i64),
}

#[derive(Clone, Default, PartialEq, Eq, PartialOrd, Ord)]
struct State {
    registers: BTreeMap<Register, Value>,
    tables: BTreeMap<(u32, i64), u32>,
    stack: BTreeMap<i64, Value>,
    stack_offset: i64,
}

impl State {
    fn new() -> Self {
        let mut result = Self::default();
        result.registers.insert(Register::RCX, Value::Object(0, 0));
        result
    }

    fn allocate(&mut self, root: u32) {
        self.registers.insert(Register::RAX, Value::Object(root, 0));
    }

    fn clobber(&mut self) {
        for register in [
            Register::RAX,
            Register::RCX,
            Register::RDX,
            Register::R8,
            Register::R9,
            Register::R10,
            Register::R11,
        ] {
            self.registers.remove(&register);
        }
    }

    fn operand(&self, instruction: &Instruction, operand: u32) -> Option<Value> {
        match instruction.op_kind(operand) {
            OpKind::Register => {
                let source = instruction.op_register(operand);
                match self.registers.get(&register(source)).copied()? {
                    Value::Number(number) => {
                        let bits = source.size() * 8;
                        Some(Value::Number(if bits < 64 {
                            number & ((1u64 << bits) - 1)
                        } else {
                            number
                        }))
                    }
                    value if is_pointer_register(source) => Some(value),
                    _ => None,
                }
            }
            OpKind::Immediate8 => Some(Value::Number(u64::from(instruction.immediate8()))),
            OpKind::Immediate16 => Some(Value::Number(u64::from(instruction.immediate16()))),
            OpKind::Immediate32 => Some(Value::Number(u64::from(instruction.immediate32()))),
            OpKind::Immediate64 => Some(Value::Number(instruction.immediate64())),
            OpKind::Immediate8to32 => {
                Some(Value::Number(instruction.immediate8to32() as u32 as u64))
            }
            OpKind::Immediate8to64 => Some(Value::Number(instruction.immediate8to64() as u64)),
            OpKind::Immediate32to64 => Some(Value::Number(instruction.immediate32to64() as u64)),
            OpKind::Memory
                if instruction.memory_base() == Register::RSP
                    && instruction.memory_index() == Register::None =>
            {
                self.stack
                    .get(&(self.stack_offset + instruction.memory_displacement64() as i64))
                    .copied()
            }
            _ => None,
        }
    }

    fn simple(&mut self, image: &PeImage, instruction: &Instruction) -> io::Result<()> {
        let mnemonic = instruction.mnemonic();
        if matches!(mnemonic, Mnemonic::Push | Mnemonic::Pop) {
            self.stack_offset += if mnemonic == Mnemonic::Push { -8 } else { 8 };
            if mnemonic == Mnemonic::Pop && instruction.op0_kind() == OpKind::Register {
                self.registers.remove(&register(instruction.op0_register()));
            }
            return Ok(());
        }
        if matches!(mnemonic, Mnemonic::Add | Mnemonic::Sub)
            && instruction.op0_kind() == OpKind::Register
            && instruction.op0_register() == Register::RSP
        {
            if let Some(Value::Number(amount)) = self.operand(instruction, 1) {
                let amount =
                    i64::try_from(amount).map_err(|_| invalid("constructor stack overflow"))?;
                self.stack_offset = self
                    .stack_offset
                    .checked_add(if mnemonic == Mnemonic::Add {
                        amount
                    } else {
                        -amount
                    })
                    .ok_or_else(|| invalid("constructor stack overflow"))?;
            }
            return Ok(());
        }
        if mnemonic == Mnemonic::Mov && instruction.op0_kind() == OpKind::Memory {
            let value = self.operand(instruction, 1);
            if instruction.memory_base() == Register::RSP
                && instruction.memory_index() == Register::None
            {
                let offset = self.stack_offset + instruction.memory_displacement64() as i64;
                if let Some(value) = value {
                    self.stack.insert(offset, value);
                } else {
                    self.stack.remove(&offset);
                }
            } else if instruction.memory_index() == Register::None
                && let Some(Value::Address(address)) = value
                && instruction.op1_kind() == OpKind::Register
                && is_pointer_register(instruction.op1_register())
                && is_vtable(image, address)
            {
                let Some(Value::Object(root, offset)) = self
                    .registers
                    .get(&register(instruction.memory_base()))
                    .copied()
                else {
                    return Err(invalid("constructor identity has an unknown destination"));
                };
                let offset = offset
                    .checked_add(instruction.memory_displacement64() as i64)
                    .ok_or_else(|| invalid("constructor object offset overflow"))?;
                self.tables
                    .insert((root, offset), image.va_to_rva(address)?);
            }
            return Ok(());
        }
        if instruction.op_count() == 0 || instruction.op0_kind() != OpKind::Register {
            return Ok(());
        }
        let destination = register(instruction.op0_register());
        let value = match mnemonic {
            Mnemonic::Lea if instruction.op1_kind() == OpKind::Memory => {
                if instruction.is_ip_rel_memory_operand() {
                    Some(Value::Address(instruction.ip_rel_memory_address()))
                } else if instruction.memory_index() == Register::None {
                    match self
                        .registers
                        .get(&register(instruction.memory_base()))
                        .copied()
                    {
                        Some(Value::Object(root, offset)) => offset
                            .checked_add(instruction.memory_displacement64() as i64)
                            .map(|offset| Value::Object(root, offset)),
                        _ => None,
                    }
                } else {
                    None
                }
            }
            Mnemonic::Mov => self.operand(instruction, 1),
            Mnemonic::Xor
                if instruction.op1_kind() == OpKind::Register
                    && instruction.op0_register() == instruction.op1_register() =>
            {
                Some(Value::Number(0))
            }
            Mnemonic::Cmp | Mnemonic::Test => return Ok(()),
            _ => None,
        };
        let value = value.filter(|value| {
            matches!(value, Value::Number(_)) || is_pointer_register(instruction.op0_register())
        });
        if let Some(value) = value {
            self.registers.insert(destination, value);
        } else {
            self.registers.remove(&destination);
        }
        Ok(())
    }
}

fn is_pointer_register(register: Register) -> bool {
    matches!(
        register,
        Register::RAX
            | Register::RBX
            | Register::RCX
            | Register::RDX
            | Register::RSI
            | Register::RDI
            | Register::RSP
            | Register::RBP
            | Register::R8
            | Register::R9
            | Register::R10
            | Register::R11
            | Register::R12
            | Register::R13
            | Register::R14
            | Register::R15
    )
}

fn register(value: Register) -> Register {
    use Register::*;
    match value {
        AL | AH | AX | EAX => RAX,
        BL | BH | BX | EBX => RBX,
        CL | CH | CX | ECX => RCX,
        DL | DH | DX | EDX => RDX,
        SIL | SI | ESI => RSI,
        DIL | DI | EDI => RDI,
        SPL | SP | ESP => RSP,
        BPL | BP | EBP => RBP,
        R8L | R8W | R8D => R8,
        R9L | R9W | R9D => R9,
        R10L | R10W | R10D => R10,
        R11L | R11W | R11D => R11,
        R12L | R12W | R12D => R12,
        R13L | R13W | R13D => R13,
        R14L | R14W | R14D => R14,
        R15L | R15W | R15D => R15,
        _ => value,
    }
}

#[derive(Clone, Copy, Default)]
struct Summary {
    receiver: Option<u32>,
    returned: Option<(u32, bool)>,
}

struct Factories<'a> {
    image: &'a PeImage,
    allocation: u32,
    cache: BTreeMap<u32, Summary>,
    visiting: BTreeSet<u32>,
}

impl<'a> Factories<'a> {
    fn new(image: &'a PeImage, allocation: u32) -> Self {
        Self {
            image,
            allocation,
            cache: BTreeMap::new(),
            visiting: BTreeSet::new(),
        }
    }

    fn object(&mut self, entry: u32) -> io::Result<u32> {
        let factory = code_pointer(self.image, checked_add(entry, ENTRY_FACTORY)?)?;
        let summary = self.summary(self.image.va_to_rva(factory)?)?;
        let (vtable, _) = summary
            .returned
            .ok_or_else(|| invalid("registry factory has no returned object identity"))?;
        class_name(self.image, vtable)?;
        Ok(vtable)
    }

    fn summary(&mut self, function: u32) -> io::Result<Summary> {
        if let Some(summary) = self.cache.get(&function) {
            return Ok(*summary);
        }
        if !self.visiting.insert(function) {
            return Err(invalid("registry constructor graph contains a cycle"));
        }
        let result = self.read_summary(function);
        self.visiting.remove(&function);
        if let Ok(summary) = result {
            self.cache.insert(function, summary);
        }
        result
    }

    fn read_summary(&mut self, function: u32) -> io::Result<Summary> {
        let instructions = function_body(self.image, function)?;
        if instructions.is_empty() {
            return Err(invalid("registry function has no instructions"));
        }
        let boundaries: BTreeMap<_, _> = instructions
            .iter()
            .enumerate()
            .map(|(index, instruction)| (instruction.ip(), index))
            .collect();
        let mut pending = vec![(0, State::new(), None)];
        let mut visited = BTreeSet::new();
        let mut result = Summary::default();
        while let Some((mut index, mut state, mut previous)) = pending.pop() {
            while let Some(instruction) = instructions.get(index) {
                if !visited.insert((index, state.clone(), previous)) {
                    break;
                }
                let mut next = index + 1;
                if instruction.flow_control() == FlowControl::ConditionalBranch {
                    let target = boundaries
                        .get(&instruction.near_branch_target())
                        .copied()
                        .ok_or_else(|| {
                            invalid("registry conditional branch leaves its function extent")
                        })?;
                    match branch_taken(
                        &state,
                        instruction,
                        previous.map(|index| &instructions[index]),
                    ) {
                        Some(true) => next = target,
                        Some(false) => {}
                        None => pending.push((target, state.clone(), None)),
                    }
                } else if let Some(target) = direct_target(self.image, instruction)? {
                    if instruction.mnemonic() == Mnemonic::Jmp
                        && let Some(&target) = boundaries.get(&instruction.near_branch_target())
                    {
                        previous = None;
                        index = target;
                        continue;
                    }
                    let allocation_root = self.image.va_to_rva(instruction.ip())?;
                    let receiver = state.registers.get(&Register::RCX).copied();
                    let summary = if target != self.allocation
                        && matches!(receiver, Some(Value::Object(_, 0)))
                    {
                        Some(self.summary(target)?)
                    } else {
                        None
                    };
                    state.clobber();
                    if target == self.allocation {
                        state.allocate(allocation_root);
                    } else if let Some(summary) = summary
                        && let Some(Value::Object(root, offset)) = receiver
                    {
                        if let Some(vtable) = summary.receiver {
                            state.tables.insert((root, offset), vtable);
                        }
                        if let Some((vtable, returned_receiver)) = summary.returned {
                            if returned_receiver {
                                state
                                    .registers
                                    .insert(Register::RAX, Value::Object(root, offset));
                            } else {
                                state.allocate(allocation_root);
                                if let Some(Value::Object(root, offset)) =
                                    state.registers.get(&Register::RAX).copied()
                                {
                                    state.tables.insert((root, offset), vtable);
                                }
                            }
                        }
                    }
                    if instruction.mnemonic() == Mnemonic::Jmp {
                        record_return(&state, &mut result)?;
                        break;
                    }
                } else if instruction.mnemonic() == Mnemonic::Call {
                    state.clobber();
                } else if instruction.mnemonic() == Mnemonic::Ret {
                    record_return(&state, &mut result)?;
                    break;
                } else {
                    state.simple(self.image, instruction)?;
                }
                previous = Some(index);
                index = next;
            }
        }
        Ok(result)
    }
}

fn branch_taken(
    state: &State,
    branch: &Instruction,
    previous: Option<&Instruction>,
) -> Option<bool> {
    if !matches!(branch.mnemonic(), Mnemonic::Je | Mnemonic::Jne) {
        return None;
    }
    let previous = previous?;
    if !matches!(previous.mnemonic(), Mnemonic::Test | Mnemonic::Cmp) {
        return None;
    }
    let left = state.operand(previous, 0)?;
    let right = state.operand(previous, 1)?;
    let zero = match (previous.mnemonic(), left, right) {
        (Mnemonic::Test, Value::Number(left), Value::Number(right)) => left & right == 0,
        (Mnemonic::Cmp, Value::Number(left), Value::Number(right)) => left == right,
        (Mnemonic::Test, Value::Object(root, 0), Value::Object(other, 0))
            if root == other
                && is_pointer_register(previous.op0_register())
                && is_pointer_register(previous.op1_register()) =>
        {
            false
        }
        (Mnemonic::Cmp, Value::Object(_, 0), Value::Number(0))
            if is_pointer_register(previous.op0_register()) =>
        {
            false
        }
        _ => return None,
    };
    Some(if branch.mnemonic() == Mnemonic::Je {
        zero
    } else {
        !zero
    })
}

fn record_return(state: &State, summary: &mut Summary) -> io::Result<()> {
    if let Some(vtable) = state.tables.get(&(0, 0)).copied() {
        if summary.receiver.is_some_and(|previous| previous != vtable) {
            return Err(invalid(
                "registry constructor leaves ambiguous receiver identities",
            ));
        }
        summary.receiver = Some(vtable);
    }
    if let Some(Value::Object(root, offset)) = state.registers.get(&Register::RAX).copied()
        && let Some(vtable) = state.tables.get(&(root, offset)).copied()
    {
        let returned = (vtable, root == 0 && offset == 0);
        if summary
            .returned
            .is_some_and(|previous| previous != returned)
        {
            return Err(invalid(
                "registry factory returns ambiguous object identities",
            ));
        }
        summary.returned = Some(returned);
    }
    Ok(())
}

fn extent(image: &PeImage, start: u32) -> io::Result<(u32, u32)> {
    if let Ok((begin, end)) = image.function(start) {
        if begin != start {
            return Err(invalid("registry function pointer is not a function entry"));
        }
        return Ok((begin, end));
    }
    let mut end = image
        .executable_ranges()
        .iter()
        .find_map(|&(begin, end)| (begin <= start && start < end).then_some(end))
        .ok_or_else(|| invalid("registry function is outside executable sections"))?;
    let functions = image.functions();
    if let Some(&(next, _)) = functions.get(functions.partition_point(|&(begin, _)| begin <= start))
    {
        end = end.min(next);
    }
    Ok((start, end))
}

fn function_body(image: &PeImage, start: u32) -> io::Result<Vec<Instruction>> {
    if image.function(start).is_ok() {
        let (begin, end) = extent(image, start)?;
        return image.instructions(begin, end);
    }
    let (_, end) = extent(image, start)?;
    let bytes = image.bytes(start, (end - start) as usize)?;
    let mut decoder = Decoder::with_ip(
        64,
        bytes,
        image.base + u64::from(start),
        DecoderOptions::NONE,
    );
    let mut result = Vec::new();
    while decoder.can_decode() {
        let instruction = decoder.decode();
        if instruction.is_invalid() {
            return Err(invalid("invalid instruction in registry leaf function"));
        }
        if matches!(
            instruction.op0_kind(),
            OpKind::NearBranch64 | OpKind::NearBranch32
        ) && !matches!(instruction.mnemonic(), Mnemonic::Call | Mnemonic::Jmp)
        {
            return Err(invalid(
                "registry leaf function has an unbounded conditional branch",
            ));
        }
        let terminal = instruction.mnemonic() == Mnemonic::Ret
            || (instruction.mnemonic() == Mnemonic::Jmp
                && direct_target(image, &instruction)?.is_some());
        result.push(instruction);
        if terminal {
            return Ok(result);
        }
    }
    Err(invalid("registry leaf function has no bounded terminator"))
}

struct Masked {
    bytes: [u8; 15],
    len: u8,
    outside: Option<u32>,
    cases: Vec<u32>,
    data: Vec<u8>,
}

fn masked_body(image: &PeImage, start: u32, data: bool) -> io::Result<Vec<Masked>> {
    let (low, high) = extent(image, start)?;
    let bytes = image.bytes(low, (high - low) as usize)?;
    let mut decoder =
        Decoder::with_ip(64, bytes, image.base + u64::from(low), DecoderOptions::NONE);
    let mut decoded = BTreeMap::new();
    let mut pending = vec![start];
    let inside = |rva: u32| (low..high).contains(&rva);
    let image_rva = |value: u64| {
        value >= 0x1000
            && image
                .base
                .checked_add(value)
                .is_some_and(|address| image.va_to_rva(address).is_ok())
    };
    while let Some(mut at) = pending.pop() {
        while inside(at) && !decoded.contains_key(&at) {
            decoder
                .set_position((at - low) as usize)
                .map_err(|_| invalid("fingerprint position is outside its function"))?;
            decoder.set_ip(image.base + u64::from(at));
            let instruction = decoder.decode();
            if instruction.is_invalid() {
                return Err(invalid("fingerprinted function has an invalid instruction"));
            }
            let offsets = decoder.get_constant_offsets(&instruction);
            let length = instruction.len();
            let mut masked = Masked {
                bytes: [0; 15],
                len: length as u8,
                outside: None,
                cases: Vec::new(),
                data: Vec::new(),
            };
            masked.bytes[..length]
                .copy_from_slice(&bytes[(at - low) as usize..(at - low) as usize + length]);
            if offsets.has_displacement() {
                let offset = offsets.displacement_offset();
                let size = match (offsets.displacement_size(), instruction.code()) {
                    (
                        8,
                        Code::Mov_AL_moffs8
                        | Code::Mov_AX_moffs16
                        | Code::Mov_EAX_moffs32
                        | Code::Mov_RAX_moffs64
                        | Code::Mov_moffs8_AL
                        | Code::Mov_moffs16_AX
                        | Code::Mov_moffs32_EAX
                        | Code::Mov_moffs64_RAX,
                    ) => 8,
                    (8, _) => 4,
                    (size, _) => size,
                };
                let displacement = instruction.memory_displacement64();
                if instruction.is_ip_rel_memory_operand() || size == 8 {
                    masked.bytes[offset..offset + size].fill(0);
                    if data && instruction.is_ip_rel_memory_operand() {
                        masked.data = referenced_data(image, &instruction)?;
                    }
                } else if size == 4
                    && inside(displacement as u32)
                    && displacement <= u64::from(u32::MAX)
                {
                    let relative = displacement as u32 - low;
                    masked.bytes[offset..offset + 4].copy_from_slice(&relative.to_le_bytes());
                    if instruction.memory_index() != Register::None
                        && instruction.memory_index_scale() == 4
                    {
                        let mut entry = displacement as u32;
                        while entry.checked_add(4).is_some_and(|end| end <= high) {
                            let target = image.u32(entry)?;
                            if !inside(target) {
                                break;
                            }
                            pending.push(target);
                            masked.cases.push(target - low);
                            entry += 4;
                        }
                    }
                } else if size == 4
                    && image_rva(displacement & u64::from(u32::MAX))
                    && displacement <= u64::from(u32::MAX)
                {
                    masked.bytes[offset..offset + 4].fill(0);
                }
            }
            let mut next = Some(at + length as u32);
            if matches!(instruction.op0_kind(), OpKind::NearBranch64) {
                let target = image.va_to_rva(instruction.near_branch_target())?;
                if inside(target) {
                    match instruction.flow_control() {
                        FlowControl::UnconditionalBranch => next = Some(target),
                        FlowControl::ConditionalBranch => pending.push(target),
                        _ => {}
                    }
                } else {
                    let offset = offsets.immediate_offset();
                    masked.bytes[offset..offset + offsets.immediate_size()].fill(0);
                    masked.outside = Some(target);
                    if instruction.flow_control() == FlowControl::UnconditionalBranch {
                        next = None;
                    }
                }
            } else if offsets.has_immediate()
                && offsets.immediate_size() == 8
                && instruction
                    .immediate64()
                    .checked_sub(image.base)
                    .is_some_and(image_rva)
            {
                let offset = offsets.immediate_offset();
                masked.bytes[offset..offset + 8].fill(0);
            }
            if matches!(
                instruction.flow_control(),
                FlowControl::Return
                    | FlowControl::Interrupt
                    | FlowControl::Exception
                    | FlowControl::IndirectBranch
            ) {
                next = None;
            }
            decoded.insert(at, masked);
            match next {
                Some(next) => at = next,
                None => break,
            }
        }
    }
    Ok(decoded.into_values().collect())
}

fn referenced_data(image: &PeImage, instruction: &Instruction) -> io::Result<Vec<u8>> {
    let target = image.va_to_rva(instruction.ip_rel_memory_address())?;
    if instruction.mnemonic() == Mnemonic::Lea {
        return Ok(image
            .string(target)
            .ok()
            .filter(|text| {
                !text.is_empty()
                    && text.bytes().all(|byte| byte.is_ascii_graphic() || byte == b' ')
                    && image.is_read_only(target, text.len() + 1)
            })
            .map(|text| text.as_bytes().to_vec())
            .unwrap_or_default());
    }
    let size = instruction.memory_size().size();
    if size == 0 || size > 16 || !image.is_read_only(target, size) {
        return Ok(Vec::new());
    }
    let bytes = image.bytes(target, size)?;
    if size == 8 {
        let value = u64::from_le_bytes(bytes.try_into().unwrap());
        if image.va_to_rva(value).is_ok() {
            return Ok(Vec::new());
        }
    }
    Ok(bytes.to_vec())
}

struct Fingerprints<'a> {
    image: &'a PeImage,
    memo: HashMap<(u32, bool), Option<u64>>,
}

impl<'a> Fingerprints<'a> {
    fn new(image: &'a PeImage) -> Self {
        Self {
            image,
            memo: HashMap::new(),
        }
    }

    fn fingerprint(&mut self, start: u32, callees: bool) -> Option<u64> {
        if let Some(&found) = self.memo.get(&(start, callees)) {
            return found;
        }
        let found = match masked_body(self.image, start, callees) {
            Ok(body) => {
                let mut hasher = Sha256::new();
                for instruction in &body {
                    hasher.update([instruction.len]);
                    hasher.update(&instruction.bytes[..usize::from(instruction.len)]);
                    hasher.update((instruction.data.len() as u32).to_le_bytes());
                    hasher.update(&instruction.data);
                    hasher.update((instruction.cases.len() as u32).to_le_bytes());
                    for case in &instruction.cases {
                        hasher.update(case.to_le_bytes());
                    }
                    if callees && let Some(target) = instruction.outside {
                        let callee = self.fingerprint(target, false).unwrap_or(u64::MAX);
                        hasher.update(callee.to_le_bytes());
                    }
                }
                Some(truncated(hasher.finalize().into()))
            }
            Err(_) => None,
        };
        self.memo.insert((start, callees), found);
        found
    }

    fn layout(&mut self, vtable: u32) -> io::Result<u64> {
        let mut hasher = Sha256::new();
        let mut slots = 0u32;
        while let Some(slot) = vtable.checked_add(slots * 8)
            && let Ok(address) = self.image.u64(slot)
            && let Ok(rva) = self.image.va_to_rva(address)
            && is_code(self.image, rva)
        {
            hasher.update(
                self.fingerprint(rva, true)
                    .unwrap_or(u64::MAX)
                    .to_le_bytes(),
            );
            slots += 1;
        }
        if slots == 0 {
            return Err(invalid("registry base class has no virtual functions"));
        }
        hasher.update(slots.to_le_bytes());
        Ok(truncated(hasher.finalize().into()))
    }
}

struct Identities<'a> {
    code: Fingerprints<'a>,
    ambiguous: &'a BTreeSet<u64>,
    unmatched: BTreeMap<u64, u64>,
}

impl Identities<'_> {
    fn function(&mut self, class: &str, slot: u32, rva: u32) -> u64 {
        let fingerprint = self.code.fingerprint(rva, true);
        let matched = fingerprint.and_then(|fingerprint| {
            let first = pinned::FUNCTIONS.partition_point(|&(found, _)| found < fingerprint);
            let candidates: Vec<u64> = pinned::FUNCTIONS[first..]
                .iter()
                .take_while(|&&(found, _)| found == fingerprint)
                .map(|&(_, address)| address)
                .collect();
            match candidates[..] {
                [] => None,
                [address] => Some(address),
                _ => Some(name_hash(class))
                    .filter(|hash| !self.ambiguous.contains(hash))
                    .and_then(|hash| {
                        pinned::COLLIDING
                            .binary_search_by_key(&(hash, slot), |&(class, slot, _)| (class, slot))
                            .ok()
                    })
                    .map(|index| pinned::COLLIDING[index].2)
                    .filter(|address| candidates.contains(address)),
            }
        });
        matched.unwrap_or_else(|| {
            let identity = UNMATCHED_IDENTITY | u64::from(rva);
            let stand_in = fingerprint.unwrap_or_else(|| name_hash(class) ^ u64::from(slot));
            self.unmatched.insert(identity, stand_in);
            identity
        })
    }

    fn class(&mut self, decorated: &str, vtable: u32) -> u64 {
        let hash = name_hash(decorated);
        match pinned::CLASSES.binary_search_by_key(&hash, |&(found, _)| found) {
            Ok(index) if !self.ambiguous.contains(&hash) => pinned::CLASSES[index].1,
            _ => {
                let identity = UNMATCHED_IDENTITY | u64::from(vtable);
                self.unmatched.insert(identity, hash);
                identity
            }
        }
    }
}

pub(super) fn pinned_source(image: &PeImage, keywords: &KeywordTables) -> io::Result<String> {
    let raw = Raw::decode(image, keywords)?;
    if !raw.ambiguous.is_empty() {
        return Err(invalid(
            "registries: several classes share a canonical name",
        ));
    }
    let mut code = Fingerprints::new(image);
    let mut functions: BTreeMap<u32, (u64, BTreeSet<(u64, u32)>)> = BTreeMap::new();
    let mut uses: Vec<_> = raw
        .reader_classes
        .into_iter()
        .zip([raw.assign, raw.comparison, raw.body])
        .map(|(class, reader)| (class, TRIGGER_VALUE_READER, reader))
        .collect();
    for row in &raw.triggers {
        uses.push((row.class, TRIGGER_VALUE_READER, row.value));
        uses.push((row.class, TRIGGER_KEY_READER, row.key));
    }
    for row in &raw.effects {
        uses.push((row.class, EFFECT_VALUE_READER, row.value));
        uses.push((row.class, EFFECT_KEY_READER, row.key));
    }
    for (class, slot, rva) in uses {
        let fingerprint = code
            .fingerprint(rva, true)
            .ok_or_else(|| invalid(format!("{class} slot {slot}: reader has no fingerprint")))?;
        functions
            .entry(rva)
            .or_insert_with(|| (fingerprint, BTreeSet::new()))
            .1
            .insert((name_hash(class), slot));
    }
    let mut counts = HashMap::new();
    for (fingerprint, _) in functions.values() {
        *counts.entry(*fingerprint).or_insert(0) += 1;
    }
    let address = |rva: u32| image.base + u64::from(rva);
    let mut by_fingerprint: Vec<_> = functions
        .iter()
        .map(|(&rva, (fingerprint, _))| (*fingerprint, address(rva)))
        .collect();
    by_fingerprint.sort_unstable();
    let mut colliding: Vec<_> = functions
        .iter()
        .filter(|(_, (fingerprint, _))| counts[fingerprint] > 1)
        .flat_map(|(&rva, (_, uses))| {
            uses.iter()
                .map(move |&(class, slot)| (class, slot, address(rva)))
        })
        .collect();
    colliding.sort_unstable();
    let mut classes: Vec<_> = raw
        .effects
        .iter()
        .map(|row| (row.class, row.vtable))
        .chain(raw.contexts.iter().copied())
        .map(|(class, vtable)| (name_hash(class), address(vtable)))
        .collect();
    classes.sort_unstable();
    classes.dedup();
    let mut masks: Vec<_> = raw
        .triggers
        .iter()
        .map(|row| (row.token, row.mask))
        .collect();
    masks.sort_unstable();

    let mut text = String::from(
        "//! Recognised-build identities that other builds' registries map onto; written by `executable-tables-check --pin`.\n\n",
    );
    writeln!(
        text,
        "pub(super) const TRIGGER_LAYOUT: u64 = {:#018x};",
        code.layout(raw.trigger_base)?
    )
    .unwrap();
    writeln!(
        text,
        "pub(super) const EFFECT_LAYOUT: u64 = {:#018x};",
        code.layout(raw.effect_base)?
    )
    .unwrap();
    text.push_str("\n/// Reader fingerprints and addresses, by fingerprint.\n#[rustfmt::skip]\npub(super) const FUNCTIONS: &[(u64, u64)] = &[\n");
    for (fingerprint, address) in by_fingerprint {
        writeln!(text, "    ({fingerprint:#018x}, {address:#x}),").unwrap();
    }
    text.push_str("];\n\n/// Class-name hash and slot of each use of a fingerprint shared by several readers.\n#[rustfmt::skip]\npub(super) const COLLIDING: &[(u64, u32, u64)] = &[\n");
    for (class, slot, address) in colliding {
        writeln!(text, "    ({class:#018x}, {slot}, {address:#x}),").unwrap();
    }
    text.push_str("];\n\n/// Class-name hashes and virtual tables of the effect classes.\n#[rustfmt::skip]\npub(super) const CLASSES: &[(u64, u64)] = &[\n");
    write_class_rows(&mut text, classes);
    text.push_str("];\n\n/// Trigger scope masks, by keyword.\n#[rustfmt::skip]\npub(super) const SCOPE_MASKS: &[(u32, u32)] = &[\n");
    for (token, mask) in masks {
        writeln!(text, "    ({token:#x}, {mask:#x}),").unwrap();
    }
    text.push_str("];\n");
    Ok(text)
}

fn write_class_rows(text: &mut String, classes: impl IntoIterator<Item = (u64, u64)>) {
    for (class, address) in classes {
        let identity = match address {
            super::effect_identity::CONTEXT => "super::super::effect_identity::CONTEXT".to_owned(),
            super::effect_identity::SCRIPTED => "super::super::effect_identity::SCRIPTED".to_owned(),
            _ => format!("{address:#x}"),
        };
        writeln!(text, "    ({class:#018x}, {identity}),").unwrap();
    }
}

fn constant_return(image: &PeImage, function: u32) -> io::Result<u32> {
    let mut result = None;
    for instruction in function_body(image, function)? {
        match instruction.mnemonic() {
            Mnemonic::Mov
                if instruction.op0_kind() == OpKind::Register
                    && instruction.op0_register() == Register::EAX
                    && instruction.op1_kind() == OpKind::Immediate32 =>
            {
                result = Some(instruction.immediate32());
            }
            Mnemonic::Xor
                if instruction.op0_register() == Register::EAX
                    && instruction.op1_kind() == OpKind::Register
                    && instruction.op1_register() == Register::EAX =>
            {
                result = Some(0)
            }
            Mnemonic::Ret => {
                return result.ok_or_else(|| invalid("trigger scope mask has no constant result"));
            }
            Mnemonic::Nop | Mnemonic::Int3 => {}
            _ => return Err(invalid("trigger scope mask has an unsupported getter")),
        }
    }
    Err(invalid("trigger scope mask has no return"))
}

pub(crate) fn class_name(image: &PeImage, vtable: u32) -> io::Result<String> {
    project_class_name(decorated_name(image, vtable)?)
}

fn project_class_name(decorated: &str) -> io::Result<String> {
    let raw = decorated
        .strip_prefix(".?AV")
        .or_else(|| decorated.strip_prefix(".?AU"))
        .and_then(|name| name.strip_suffix("@@"))
        .filter(|name| {
            !name.is_empty() && name.is_ascii() && !name.bytes().any(|byte| byte.is_ascii_control())
        })
        .ok_or_else(|| invalid("unsupported class identity decoration"))?;
    if let Some(template) = raw.strip_prefix("?$") {
        let (name, _) = template
            .split_once('@')
            .ok_or_else(|| invalid("invalid template class identity"))?;
        if !identifier(name) {
            return Err(invalid("invalid template class identity"));
        }
        return Ok(format!("{name}<>"));
    }
    let parts: Vec<_> = raw.split('@').collect();
    if parts.iter().all(|part| identifier(part)) {
        Ok(parts.into_iter().rev().collect::<Vec<_>>().join("::"))
    } else {
        Ok(raw.to_owned())
    }
}

fn identifier(value: &str) -> bool {
    let mut bytes = value.bytes();
    bytes
        .next()
        .is_some_and(|byte| byte.is_ascii_alphabetic() || byte == b'_')
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

#[cfg(test)]
mod tests {
    use super::*;

    const TRIGGER_REGISTRATION: u32 = 0x0059_7b90;
    const EFFECT_REGISTRATION: u32 = 0x0058_88e0;
    const OBJECT_ALLOCATION: u32 = 0x025a_009c;

    const CODE_START: u32 = 0x1000;
    const FACTORY_START: u32 = 0x1100;
    const CONSTRUCTOR_START: u32 = 0x1200;
    const ENTRY_TABLE: u32 = 0x3100;
    const OBJECT_TABLE: u32 = 0x3208;
    const HELPER_TABLE: u32 = 0x3308;

    fn write_u16(bytes: &mut [u8], at: usize, value: u16) {
        bytes[at..at + 2].copy_from_slice(&value.to_le_bytes());
    }

    fn write_u32(bytes: &mut [u8], at: usize, value: u32) {
        bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }

    fn write_u64(bytes: &mut [u8], at: usize, value: u64) {
        bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
    }

    fn offset(rva: u32) -> usize {
        if rva < 0x3000 {
            (rva - 0x1000 + 0x200) as usize
        } else {
            (rva - 0x3000 + 0x1200) as usize
        }
    }

    fn fixture_bytes() -> Vec<u8> {
        let mut bytes = vec![0; 0x2200];
        bytes[..2].copy_from_slice(b"MZ");
        write_u32(&mut bytes, 0x3c, 0x80);
        bytes[0x80..0x84].copy_from_slice(b"PE\0\0");
        write_u16(&mut bytes, 0x84, 0x8664);
        write_u16(&mut bytes, 0x86, 2);
        write_u16(&mut bytes, 0x94, 240);
        write_u16(&mut bytes, 0x98, 0x20b);
        write_u64(&mut bytes, 0xb0, super::super::pe::PREFERRED_IMAGE_BASE);
        write_u32(&mut bytes, 0xb8, 0x1000);
        write_u32(&mut bytes, 0xbc, 0x200);
        write_u32(&mut bytes, 0xd0, 0x0260_0000);
        write_u32(&mut bytes, 0xd4, 0x200);
        write_u32(&mut bytes, 0x104, 16);
        write_u32(&mut bytes, 0x120, 0x3700);
        write_u32(&mut bytes, 0x124, 12);
        for (section, rva, raw, flags) in [
            (0x188, 0x1000, 0x200, 0x6000_0020),
            (0x1b0, 0x3000, 0x1200, 0x4000_0040),
        ] {
            write_u32(&mut bytes, section + 8, 0x1000);
            write_u32(&mut bytes, section + 12, rva);
            write_u32(&mut bytes, section + 16, 0x1000);
            write_u32(&mut bytes, section + 20, raw);
            write_u32(&mut bytes, section + 36, flags);
        }
        bytes[offset(CODE_START)] = 0xc3;
        write_u32(&mut bytes, offset(0x3700), CODE_START);
        write_u32(&mut bytes, offset(0x3704), CODE_START + 1);
        write_u32(&mut bytes, offset(0x3708), 0x3800);
        let base = super::super::pe::PREFERRED_IMAGE_BASE;
        write_u64(
            &mut bytes,
            offset(ENTRY_TABLE),
            base + u64::from(CODE_START),
        );
        write_u64(
            &mut bytes,
            offset(ENTRY_TABLE + ENTRY_FACTORY),
            base + u64::from(FACTORY_START),
        );
        for (vtable, locator, descriptor, name) in [
            (OBJECT_TABLE, 0x3400, 0x3500, ".?AVSyntheticRoot@@"),
            (HELPER_TABLE, 0x3420, 0x3540, ".?AVSyntheticHelper@@"),
        ] {
            write_u64(&mut bytes, offset(vtable - 8), base + u64::from(locator));
            write_u64(&mut bytes, offset(vtable), base + u64::from(CODE_START));
            write_u64(&mut bytes, offset(vtable + 8), base + u64::from(CODE_START));
            write_u32(&mut bytes, offset(locator), 1);
            write_u32(&mut bytes, offset(locator + 12), descriptor);
            write_u32(&mut bytes, offset(locator + 16), 0x3600);
            write_u32(&mut bytes, offset(locator + 20), locator);
            let at = offset(descriptor + 16);
            bytes[at..at + name.len()].copy_from_slice(name.as_bytes());
        }
        bytes
    }

    fn relative(code: &mut Vec<u8>, start: u32, opcode: &[u8], target: u32) {
        code.extend_from_slice(opcode);
        let next = start + code.len() as u32 + 4;
        let displacement = i32::try_from(i64::from(target) - i64::from(next)).unwrap();
        code.extend_from_slice(&displacement.to_le_bytes());
    }

    fn install_code(bytes: &mut [u8], start: u32, code: &[u8]) {
        let at = offset(start);
        bytes[at..at + code.len()].copy_from_slice(code);
    }

    fn allocation(code: &mut Vec<u8>) {
        code.extend_from_slice(&[0xb9, 0x40, 0, 0, 0]);
        relative(code, FACTORY_START, &[0xe8], OBJECT_ALLOCATION);
        code.extend_from_slice(&[0x48, 0x89, 0xc3]);
    }

    fn registration_fixture(clobber_arguments: bool) -> PeImage {
        let mut bytes = fixture_bytes();
        let mut code = Vec::new();
        allocation(&mut code);
        relative(&mut code, FACTORY_START, &[0x48, 0x8d, 0x0d], ENTRY_TABLE);
        code.extend_from_slice(&[0x48, 0x89, 0x08, 0x49, 0x89, 0xc0, 0xba, 42, 0, 0, 0]);
        if clobber_arguments {
            relative(&mut code, FACTORY_START, &[0xe8], CODE_START);
            code.extend_from_slice(&[0x49, 0x89, 0xd8]);
        }
        relative(&mut code, FACTORY_START, &[0xe9], TRIGGER_REGISTRATION);
        install_code(&mut bytes, FACTORY_START, &code);
        write_u32(&mut bytes, offset(0x3700), FACTORY_START);
        write_u32(
            &mut bytes,
            offset(0x3704),
            FACTORY_START + code.len() as u32,
        );
        PeImage::parse(bytes).unwrap()
    }

    fn trigger_registrations(image: &PeImage) -> io::Result<BTreeMap<u32, u32>> {
        let [sites] = call_sites(image, [TRIGGER_REGISTRATION])?;
        registrations(
            image,
            OBJECT_ALLOCATION,
            TRIGGER_REGISTRATION,
            sites,
            "synthetic",
        )
    }

    #[test]
    fn registration_requires_the_entry_object_and_current_arguments() {
        let image = registration_fixture(false);
        assert_eq!(
            trigger_registrations(&image).unwrap(),
            BTreeMap::from([(42, ENTRY_TABLE)])
        );
        let image = registration_fixture(true);
        let error = trigger_registrations(&image).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("1 unsupported registration occurrences")
        );
    }

    #[test]
    fn one_pass_finds_each_targets_calls_and_jumps() {
        let image = registration_fixture(true);
        let [allocations, helpers, registrations, unused] = call_sites(
            &image,
            [
                OBJECT_ALLOCATION,
                CODE_START,
                TRIGGER_REGISTRATION,
                EFFECT_REGISTRATION,
            ],
        )
        .unwrap();
        assert_eq!(allocations, BTreeSet::from([FACTORY_START + 5]));
        assert_eq!(helpers, BTreeSet::from([FACTORY_START + 31]));
        assert_eq!(registrations, BTreeSet::from([FACTORY_START + 39]));
        assert!(unused.is_empty());
    }

    #[test]
    fn factory_tracks_the_returned_object_past_an_embedded_helper() {
        let mut bytes = fixture_bytes();
        let mut code = Vec::new();
        allocation(&mut code);
        relative(&mut code, FACTORY_START, &[0x48, 0x8d, 0x0d], OBJECT_TABLE);
        code.extend_from_slice(&[0x48, 0x89, 0x0b, 0x48, 0x8d, 0x53, 0x10]);
        relative(&mut code, FACTORY_START, &[0x48, 0x8d, 0x05], HELPER_TABLE);
        code.extend_from_slice(&[0x48, 0x89, 0x02, 0x48, 0x89, 0xd8, 0xc3]);
        install_code(&mut bytes, FACTORY_START, &code);
        let image = PeImage::parse(bytes).unwrap();
        assert_eq!(
            Factories::new(&image, OBJECT_ALLOCATION)
                .object(ENTRY_TABLE)
                .unwrap(),
            OBJECT_TABLE
        );
    }

    #[test]
    fn constructor_calls_preserve_the_receiver_and_derived_override() {
        let mut bytes = fixture_bytes();
        let mut code = Vec::new();
        allocation(&mut code);
        code.extend_from_slice(&[0x48, 0x89, 0xc1]);
        relative(&mut code, FACTORY_START, &[0xe8], CONSTRUCTOR_START);
        relative(&mut code, FACTORY_START, &[0x48, 0x8d, 0x05], OBJECT_TABLE);
        code.extend_from_slice(&[0x48, 0x89, 0x03, 0x48, 0x89, 0xd8, 0xc3]);
        install_code(&mut bytes, FACTORY_START, &code);
        let mut constructor = Vec::new();
        relative(
            &mut constructor,
            CONSTRUCTOR_START,
            &[0x48, 0x8d, 0x05],
            HELPER_TABLE,
        );
        constructor.extend_from_slice(&[0x48, 0x89, 0x01, 0x48, 0x89, 0xc8, 0xc3]);
        install_code(&mut bytes, CONSTRUCTOR_START, &constructor);
        let image = PeImage::parse(bytes).unwrap();
        assert_eq!(
            Factories::new(&image, OBJECT_ALLOCATION)
                .object(ENTRY_TABLE)
                .unwrap(),
            OBJECT_TABLE
        );
    }

    #[test]
    fn factory_keeps_the_successful_return_separate_from_allocation_failure() {
        let mut bytes = fixture_bytes();
        let mut code = Vec::new();
        allocation(&mut code);
        code.extend_from_slice(&[0x48, 0x85, 0xc0]);
        let failure_branch = code.len();
        relative(&mut code, FACTORY_START, &[0x0f, 0x84], FACTORY_START);
        relative(&mut code, FACTORY_START, &[0x48, 0x8d, 0x0d], OBJECT_TABLE);
        code.extend_from_slice(&[0x48, 0x89, 0x0b]);
        let success_branch = code.len();
        relative(&mut code, FACTORY_START, &[0xe9], FACTORY_START);
        let failure = code.len();
        code.extend_from_slice(&[0x31, 0xdb]);
        let finish = code.len();
        code.extend_from_slice(&[0x48, 0x89, 0xd8, 0xc3]);
        let failure_displacement = i32::try_from(failure - (failure_branch + 6)).unwrap();
        code[failure_branch + 2..failure_branch + 6]
            .copy_from_slice(&failure_displacement.to_le_bytes());
        let success_displacement = i32::try_from(finish - (success_branch + 5)).unwrap();
        code[success_branch + 1..success_branch + 5]
            .copy_from_slice(&success_displacement.to_le_bytes());
        install_code(&mut bytes, FACTORY_START, &code);
        write_u32(&mut bytes, offset(0x3700), FACTORY_START);
        write_u32(
            &mut bytes,
            offset(0x3704),
            FACTORY_START + code.len() as u32,
        );
        let image = PeImage::parse(bytes).unwrap();
        assert_eq!(
            Factories::new(&image, OBJECT_ALLOCATION)
                .object(ENTRY_TABLE)
                .unwrap(),
            OBJECT_TABLE
        );
    }

    #[test]
    fn factory_refuses_different_returned_identities_on_unknown_branches() {
        let mut bytes = fixture_bytes();
        let mut code = Vec::new();
        allocation(&mut code);
        code.extend_from_slice(&[0x85, 0xd2]);
        let alternate_branch = code.len();
        relative(&mut code, FACTORY_START, &[0x0f, 0x84], FACTORY_START);
        relative(&mut code, FACTORY_START, &[0x48, 0x8d, 0x0d], OBJECT_TABLE);
        code.extend_from_slice(&[0x48, 0x89, 0x0b, 0x48, 0x89, 0xd8, 0xc3]);
        let alternate = code.len();
        relative(&mut code, FACTORY_START, &[0x48, 0x8d, 0x0d], HELPER_TABLE);
        code.extend_from_slice(&[0x48, 0x89, 0x0b, 0x48, 0x89, 0xd8, 0xc3]);
        let displacement = i32::try_from(alternate - (alternate_branch + 6)).unwrap();
        code[alternate_branch + 2..alternate_branch + 6]
            .copy_from_slice(&displacement.to_le_bytes());
        install_code(&mut bytes, FACTORY_START, &code);
        write_u32(&mut bytes, offset(0x3700), FACTORY_START);
        write_u32(
            &mut bytes,
            offset(0x3704),
            FACTORY_START + code.len() as u32,
        );
        let image = PeImage::parse(bytes).unwrap();
        let error = Factories::new(&image, OBJECT_ALLOCATION)
            .object(ENTRY_TABLE)
            .unwrap_err();
        assert!(error.to_string().contains("ambiguous object identities"));
    }

    #[test]
    fn constructor_cycles_and_inconsistent_class_locators_are_refused() {
        let mut bytes = fixture_bytes();
        let mut code = Vec::new();
        relative(&mut code, FACTORY_START, &[0xe9], FACTORY_START);
        install_code(&mut bytes, FACTORY_START, &code);
        let image = PeImage::parse(bytes).unwrap();
        assert!(
            Factories::new(&image, OBJECT_ALLOCATION)
                .object(ENTRY_TABLE)
                .is_err()
        );
        let mut bytes = fixture_bytes();
        write_u32(&mut bytes, offset(0x3400 + 20), 0x3420);
        let image = PeImage::parse(bytes).unwrap();
        assert!(class_name(&image, OBJECT_TABLE).is_err());
    }

    #[test]
    fn reader_scope_mask_requires_a_constant_return() {
        let mut bytes = fixture_bytes();
        install_code(&mut bytes, FACTORY_START, &[0xb8, 0x24, 0, 0, 0, 0xc3]);
        let image = PeImage::parse(bytes).unwrap();
        assert_eq!(constant_return(&image, FACTORY_START).unwrap(), 0x24);
        let mut bytes = fixture_bytes();
        install_code(&mut bytes, FACTORY_START, &[0x83, 0xf8, 0, 0xc3]);
        let image = PeImage::parse(bytes).unwrap();
        assert!(constant_return(&image, FACTORY_START).is_err());
    }

    #[test]
    fn class_name_projection_keeps_specializations_and_namespaces() {
        assert_eq!(
            project_class_name(".?AVSynthetic@tools@@").unwrap(),
            "tools::Synthetic"
        );
        assert_eq!(
            project_class_name(".?AU?$Synthetic@H@@").unwrap(),
            "Synthetic<>"
        );
        assert_eq!(
            project_class_name(".?AVSynthetic@?A123@@").unwrap(),
            "Synthetic@?A123"
        );
        assert!(project_class_name("Synthetic").is_err());
    }

    #[test]
    fn fingerprints_ignore_addresses_but_not_code() {
        let mut bytes = fixture_bytes();
        let mut first = Vec::new();
        relative(&mut first, FACTORY_START, &[0x48, 0x8d, 0x05], OBJECT_TABLE);
        first.extend_from_slice(&[0xb8, 1, 0, 0, 0]);
        relative(&mut first, FACTORY_START, &[0xe9], CODE_START);
        install_code(&mut bytes, FACTORY_START, &first);
        let mut second = Vec::new();
        relative(
            &mut second,
            CONSTRUCTOR_START,
            &[0x48, 0x8d, 0x05],
            HELPER_TABLE,
        );
        second.extend_from_slice(&[0xb8, 1, 0, 0, 0]);
        relative(&mut second, CONSTRUCTOR_START, &[0xe9], CODE_START + 1);
        install_code(&mut bytes, CONSTRUCTOR_START, &second);
        install_code(&mut bytes, CODE_START + 0x80, &[0xb8, 2, 0, 0, 0, 0xc3]);
        let image = PeImage::parse(bytes).unwrap();
        let mut code = Fingerprints::new(&image);
        assert!(code.fingerprint(FACTORY_START, false).is_some());
        assert_eq!(
            code.fingerprint(FACTORY_START, false),
            code.fingerprint(CONSTRUCTOR_START, false)
        );
        assert_ne!(
            code.fingerprint(FACTORY_START, false),
            code.fingerprint(CODE_START + 0x80, false)
        );
    }
}

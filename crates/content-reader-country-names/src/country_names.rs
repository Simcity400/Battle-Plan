
use std::collections::{BTreeMap, BTreeSet};

use std::borrow::Cow;

use replay_data::rows::{
    Recorded,
    AutonomyRow, CountryAppearanceRow, CountryLinkRow, CountryRow, PoliticalPartyRow,
    PoliticsRow, ReservedCountryNamingRow, RowCodec, TableId, TableSchema, mask, tables,
};
#[cfg(test)]
use replay_frame::frame::DecodedFrame;
use replay_frame::frame::{Rowset, Tables};

use crate::localisation::{Argument, Localisation};

#[path = "country_names/tables.rs"]
mod table_reads;
pub use table_reads::original_or_own_tag as original_or_own_tag_tables;

pub const TABLES: [TableId; 7] = [
    TableId::Countries,
    TableId::CountryLinks,
    TableId::CountryAppearance,
    TableId::Politics,
    TableId::PoliticalParties,
    TableId::Autonomy,
    TableId::ReservedCountryNaming,
];

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
#[derive(replay_spool::Spooled)]
pub struct Operands {
    pub tag: String,
    pub cosmetic: String,
    pub overlord: Option<u32>,
    overlord_suffix: String,
    subtype_suffix: String,
    group_suffix: String,
    autonomy_suffix: String,
}

impl Operands {
    pub fn heap_bytes(&self) -> usize {
        [
            &self.tag,
            &self.cosmetic,
            &self.overlord_suffix,
            &self.subtype_suffix,
            &self.group_suffix,
            &self.autonomy_suffix,
        ]
        .into_iter()
        .map(String::capacity)
        .sum()
    }

    pub fn parts(&self) -> (Option<u32>, [&str; 6]) {
        (
            self.overlord,
            [
                &self.tag,
                &self.cosmetic,
                &self.overlord_suffix,
                &self.subtype_suffix,
                &self.group_suffix,
                &self.autonomy_suffix,
            ],
        )
    }

    pub fn from_parts(
        overlord: Option<u32>,
        [
            tag,
            cosmetic,
            overlord_suffix,
            subtype_suffix,
            group_suffix,
            autonomy_suffix,
        ]: [String; 6],
    ) -> Self {
        Self {
            tag,
            cosmetic,
            overlord,
            overlord_suffix,
            subtype_suffix,
            group_suffix,
            autonomy_suffix,
        }
    }

    pub fn for_tag(tag: &str, group: Option<&str>) -> Self {
        Self {
            tag: tag.to_owned(),
            group_suffix: group.map(|group| format!("_{group}")).unwrap_or_default(),
            ..Self::default()
        }
    }

    pub fn name_key(&self, suffix: &str, exists: impl Fn(&str) -> bool) -> Option<String> {
        (!self.cosmetic.is_empty())
            .then(|| self.choose(&self.cosmetic, suffix, &exists))
            .flatten()
            .or_else(|| self.choose(&self.tag, suffix, exists))
    }

    pub fn candidate(
        &self,
        base: &str,
        suffix: &str,
        localisation: &Localisation,
    ) -> Option<String> {
        self.choose(base, suffix, |key| localisation.get(key).is_some())
    }

    fn keys(&self, base: &str, suffix: &str) -> Vec<String> {
        let (o, t, g, a) = (
            &self.overlord_suffix,
            &self.subtype_suffix,
            &self.group_suffix,
            &self.autonomy_suffix,
        );
        let mut keys = Vec::new();
        if !o.is_empty() || !a.is_empty() {
            if !t.is_empty() {
                keys.push(format!("{base}{o}{t}{a}{suffix}"));
                keys.push(format!("{base}{o}{t}_subject{suffix}"));
            }
            keys.extend([
                format!("{base}{o}{g}{a}{suffix}"),
                format!("{base}{o}{g}_subject{suffix}"),
                format!("{base}{o}{a}{suffix}"),
                format!("COUNTRY{o}{a}{suffix}"),
                format!("{base}{o}_subject{suffix}"),
            ]);
            if !t.is_empty() {
                keys.push(format!("{base}{t}{a}{suffix}"));
                keys.push(format!("{base}{t}_subject{suffix}"));
            }
            keys.extend([
                format!("{base}{g}{a}{suffix}"),
                format!("{base}{g}_subject{suffix}"),
                format!("{base}{a}{suffix}"),
                format!("COUNTRY{a}{suffix}"),
                format!("{base}_subject{suffix}"),
            ]);
        }
        if !t.is_empty() {
            keys.push(format!("{base}{t}{suffix}"));
        }
        keys.extend([format!("{base}{g}{suffix}"), format!("{base}{suffix}")]);
        keys
    }

    fn choose(&self, base: &str, suffix: &str, exists: impl Fn(&str) -> bool) -> Option<String> {
        let search = |suffix: &str| self.keys(base, suffix).into_iter().find(|k| exists(k));
        search(suffix).or_else(|| {
            let lower = suffix.to_ascii_lowercase();
            if lower != suffix {
                search(&lower)
            } else {
                let upper = suffix.to_ascii_uppercase();
                (upper != suffix).then(|| search(&upper)).flatten()
            }
        })
    }
}

const SUBJECT_ARGUMENTS: [&str; 5] = [
    "OVERLORDADJ",
    "OVERLORDNAMEDEF",
    "NONIDEOLOGY",
    "NONIDEOLOGYDEF",
    "NONIDEOLOGYADJ",
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CountryName {
    Plain(String),
    Scripted { key: String, text: String },
}

#[derive(Default)]
struct Walk {
    active: BTreeSet<(u32, String)>,
    built: BTreeMap<(u32, String), Result<String, String>>,
}

struct Frame {
    identity: (u32, String),
    key: String,
    tag: String,
    overlord: u32,
    values: Vec<(&'static str, String)>,
    deferred: Vec<&'static str>,
    asked: Option<&'static str>,
}

enum Step {
    Done(Result<CountryName, String>),
    Subject(Frame),
}

pub struct CountryNames<'a> {
    localisation: &'a Localisation,
    operands: BTreeMap<u32, Result<Operands, String>>,
    reserved_built: Option<u8>,
}

impl<'a> CountryNames<'a> {
    pub fn of_operands(
        localisation: &'a Localisation,
        operands: BTreeMap<u32, Result<Operands, String>>,
    ) -> Self {
        Self {
            localisation,
            operands,
            reserved_built: None,
        }
    }
    pub fn read<F: Tables + ?Sized>(frame: &F, localisation: &'a Localisation) -> Self {
        let (operands, reserved_built) = Self::recorded_operands(frame);
        Self::recorded(localisation, operands, reserved_built)
    }

    pub fn recorded_operands<F: Tables + ?Sized>(
        frame: &F,
    ) -> (BTreeMap<u32, Result<Operands, String>>, Option<u8>) {
        let mut operands = operands(frame);
        operands.insert(0, reserved_zero(frame));
        (operands, reserved_zero_row(frame).ok().map(|row| row.built))
    }

    pub fn recorded(
        localisation: &'a Localisation,
        operands: BTreeMap<u32, Result<Operands, String>>,
        reserved_built: Option<u8>,
    ) -> Self {
        Self {
            localisation,
            operands,
            reserved_built,
        }
    }

    pub fn read_selected<F: Tables + ?Sized>(
        frame: &F,
        localisation: &'a Localisation,
        slots: impl IntoIterator<Item = u32>,
    ) -> Self {
        let mut operands = BTreeMap::new();
        let mut pending: Vec<_> = slots.into_iter().collect();
        let reserved_built = if pending.contains(&0) {
            let input = reserved_zero(frame);
            if let Some(overlord) = input.as_ref().ok().and_then(|input| input.overlord) {
                pending.push(overlord);
            }
            operands.insert(0, input);
            reserved_zero_row(frame).ok().map(|row| row.built)
        } else {
            None
        };
        if let Some(inputs) = Inputs::read(frame) {
            while let Some(slot) = pending.pop() {
                if operands.contains_key(&slot) {
                    continue;
                }
                let country = match inputs.country(slot) {
                    Ok(Some(country)) => country,
                    Ok(None) => continue,
                    Err(reason) => {
                        operands.insert(slot, Err(reason));
                        continue;
                    }
                };
                let input = inputs.operands(&country);
                if let Some(overlord) = input.as_ref().ok().and_then(|input| input.overlord) {
                    pending.push(overlord);
                }
                operands.insert(slot, input);
            }
        }
        Self {
            localisation,
            operands,
            reserved_built,
        }
    }

    pub fn into_operands(self) -> BTreeMap<u32, Result<Operands, String>> {
        self.operands
    }

    pub fn operands(&self, slot: u32) -> Option<&Result<Operands, String>> {
        self.operands.get(&slot)
    }

    pub fn reserved_built(&self) -> Option<u8> {
        self.reserved_built
    }

    pub fn name(&self, slot: u32) -> Result<String, String> {
        self.resolve(slot, "", &mut Walk::default())
    }

    pub fn name_with(&self, slot: u32, suffix: &str) -> Result<String, String> {
        self.resolve(slot, suffix, &mut Walk::default())
    }

    pub fn resolve_name(&self, slot: u32) -> Result<CountryName, String> {
        self.resolve_name_with(slot, "")
    }

    pub fn resolve_name_with(&self, slot: u32, suffix: &str) -> Result<CountryName, String> {
        self.resolve_classified(slot, suffix, &mut Walk::default())
    }

    pub fn ruling_group(&self, slot: u32) -> Result<Option<&str>, String> {
        match self.operands.get(&slot) {
            Some(Ok(input)) => Ok(input
                .group_suffix
                .strip_prefix('_')
                .filter(|group| !group.is_empty())),
            Some(Err(reason)) => Err(reason.clone()),
            None => Err(format!("country slot {slot} has no naming operands")),
        }
    }

    fn resolve_classified(&self, slot: u32, suffix: &str, walk: &mut Walk) -> Result<CountryName, String> {
        let mut stack = Vec::new();
        let mut answer = match self.begin(slot, suffix, walk, true) {
            Step::Done(result) => return result,
            Step::Subject(frame) => {
                walk.active.insert(frame.identity.clone());
                stack.push(frame);
                None
            }
        };
        let mut root_answer: Option<Result<CountryName, String>> = None;
        while let Some(top) = stack.len().checked_sub(1) {
            if let Some(result) = answer.take() {
                match result {
                    Ok(value) => {
                        let name = stack[top].asked.take().expect("an answer follows a question");
                        stack[top].values.push((name, value));
                    }
                    Err(reason) => {
                        let frame = stack.pop().expect("a top frame");
                        let finished = self.finish(frame, Err(reason), walk, top == 0);
                        if top == 0 {
                            root_answer = Some(finished);
                        } else {
                            answer = Some(plain_only(finished));
                        }
                        continue;
                    }
                }
            }
            let reached = {
                let frame = &stack[top];
                let arguments: Vec<_> = frame
                    .values
                    .iter()
                    .map(|(name, value)| (*name, Argument::Text(value.as_str())))
                    .collect();
                self.localisation
                    .localise_with_deferred(frame.key.as_bytes(), &arguments, &frame.deferred)
            };
            let index = match reached {
                Ok(text) => {
                    let text = text.map_err(|why| why.to_string());
                    let frame = stack.pop().expect("a top frame");
                    let finished = self.finish(frame, text, walk, top == 0);
                    if top == 0 {
                        root_answer = Some(finished);
                    } else {
                        answer = Some(plain_only(finished));
                    }
                    continue;
                }
                Err(index) => index,
            };
            let frame = &mut stack[top];
            let name = frame.deferred.remove(index);
            let value = match name {
                "OVERLORDADJ" | "OVERLORDNAMEDEF" => {
                    frame.asked = Some(name);
                    let suffix = if name == "OVERLORDADJ" { "_ADJ" } else { "_DEF" };
                    match self.begin(frame.overlord, suffix, walk, false) {
                        Step::Done(result) => answer = Some(plain_only(result)),
                        Step::Subject(next) => {
                            walk.active.insert(next.identity.clone());
                            stack.push(next);
                        }
                    }
                    continue;
                }
                "NONIDEOLOGY" => self.localised(&frame.tag),
                "NONIDEOLOGYDEF" => self.localised(&format!("{}_DEF", frame.tag)),
                _ => self.localised(&format!("{}_ADJ", frame.tag)),
            };
            match value {
                Ok(value) => frame.values.push((name, value)),
                Err(reason) => {
                    let frame = stack.pop().expect("a top frame");
                    let finished = self.finish(frame, Err(reason), walk, top == 0);
                    if top == 0 {
                        root_answer = Some(finished);
                    } else {
                        answer = Some(plain_only(finished));
                    }
                }
            }
        }
        root_answer.expect("the first frame's name")
    }

    fn resolve(&self, slot: u32, suffix: &str, walk: &mut Walk) -> Result<String, String> {
        plain_only(self.resolve_classified(slot, suffix, walk))
    }

    fn begin(&self, slot: u32, suffix: &str, walk: &mut Walk, allow_scripted: bool) -> Step {
        if slot == 0
            && let Some(built) = self.reserved_built
        {
            let bit = match suffix {
                "" => 1,
                "_DEF" => 2,
                "_ADJ" => 4,
                _ => 0,
            };
            if bit != 0 && built & bit == 0 {
                return Step::Done(Ok(CountryName::Plain(String::new())));
            }
        }
        let identity = (slot, suffix.to_owned());
        if let Some(built) = walk.built.get(&identity) {
            return Step::Done(built.clone().map(CountryName::Plain));
        }
        if walk.active.contains(&identity) {
            return Step::Done(Err("country names refer to each other in a cycle".into()));
        }
        let input = match self.operands.get(&slot) {
            Some(Ok(input)) => input,
            Some(Err(reason)) => return self.built(identity, Err(reason.clone()), walk),
            None => return self.built(identity, Err(format!("country slot {slot} has no naming operands")), walk),
        };
        let exists = |key: &str| self.localisation.get(key).is_some();
        let Some(key) = input.name_key(suffix, exists) else {
            let reason = format!("no localisation key names {}{suffix}", input.tag);
            return self.built(identity, Err(reason), walk);
        };
        match input.overlord {
            None => {
                let text = self.localised(&key).and_then(|text| classify(&key, text, allow_scripted));
                self.built(identity, text, walk)
            }
            Some(overlord) => Step::Subject(Frame {
                identity,
                key,
                tag: input.tag.clone(),
                overlord,
                values: Vec::new(),
                deferred: SUBJECT_ARGUMENTS.to_vec(),
                asked: None,
            }),
        }
    }

    fn built(&self, identity: (u32, String), result: Result<CountryName, String>, walk: &mut Walk) -> Step {
        if let Ok(CountryName::Plain(ref plain)) = result {
            walk.built.insert(identity, Ok(plain.clone()));
        } else if let Err(ref err) = result {
            walk.built.insert(identity, Err(err.clone()));
        }
        Step::Done(result)
    }

    fn finish(&self, frame: Frame, text: Result<String, String>, walk: &mut Walk, allow_scripted: bool) -> Result<CountryName, String> {
        let result = text.and_then(|text| classify(&frame.key, text, allow_scripted));
        walk.active.remove(&frame.identity);
        if let Ok(CountryName::Plain(ref plain)) = result {
            walk.built.insert(frame.identity, Ok(plain.clone()));
        } else if let Err(ref err) = result {
            walk.built.insert(frame.identity, Err(err.clone()));
        }
        result
    }

    fn localised(&self, key: &str) -> Result<String, String> {
        self.localisation
            .localise(key.as_bytes())
            .map_err(|why| why.to_string())
    }
}

fn plain_only(result: Result<CountryName, String>) -> Result<String, String> {
    match result {
        Ok(CountryName::Plain(text)) => Ok(text),
        Ok(CountryName::Scripted { key, .. }) => {
            Err(format!("localisation {key} uses unsupported scripted text"))
        }
        Err(reason) => Err(reason),
    }
}

fn classify(key: &str, text: String, allow_scripted: bool) -> Result<CountryName, String> {
    if text.contains('[') {
        if allow_scripted {
            return Ok(CountryName::Scripted {
                key: key.to_owned(),
                text,
            });
        }
        return Err(format!("localisation {key} uses unsupported scripted text"));
    }
    Ok(CountryName::Plain(text))
}

fn partial<F: Tables + ?Sized, T: TableSchema>(frame: &F, mask: u64) -> (Rowset<'_, T::Row>, bool) {
    match frame.rows::<T>(mask) {
        Some(rows) => (rows, frame.table_status(T::ID).is_table_complete()),
        None => (Rowset::Decoded(&[]), false),
    }
}

fn keyed<'f, R: RowCodec>(rows: &Rowset<'f, R>, key: u128) -> Result<Option<Cow<'f, R>>, String> {
    rows.get(key).map_err(|why| why.to_string())
}

pub fn flag_keys(selected: Option<&str>, cosmetic: &str, group: Option<&str>) -> Vec<String> {
    let mut keys = Vec::new();
    for base in (!cosmetic.is_empty())
        .then_some(cosmetic)
        .into_iter()
        .chain(selected)
    {
        if let Some(group) = group {
            keys.push(format!("{base}_{group}"));
        }
        keys.push(base.to_owned());
    }
    keys
}

fn text(bytes: &[u8], what: &str) -> Result<String, String> {
    String::from_utf8(bytes.to_vec()).map_err(|_| format!("{what} is not UTF-8"))
}

fn reserved_zero_row<F: Tables + ?Sized>(
    frame: &F,
) -> Result<Cow<'_, ReservedCountryNamingRow>, String> {
    let rows = frame
        .complete_rows::<tables::ReservedCountryNaming>(RESERVED)
        .map_err(|why| format!("reserved country naming is unavailable this hour: {why}"))?;
    let mut rows = rows.iter();
    match (rows.next(), rows.next()) {
        (Some(row), None) => row.map_err(|why| why.to_string()),
        _ => Err("reserved country naming has no singleton row for country zero".into()),
    }
}

fn row_text<'f, R: Clone>(
    row: Cow<'f, R>,
    bytes: impl Fn(&R) -> &[u8],
    what: &str,
) -> Result<Cow<'f, str>, String> {
    let utf8 = || format!("{what} is not UTF-8");
    match row {
        Cow::Borrowed(row) => std::str::from_utf8(bytes(row))
            .map(Cow::Borrowed)
            .map_err(|_| utf8()),
        Cow::Owned(row) => std::str::from_utf8(bytes(&row))
            .map(|text| Cow::Owned(text.to_owned()))
            .map_err(|_| utf8()),
    }
}

fn lent(text: Result<Cow<'_, str>, String>) -> Result<&str, String> {
    match text? {
        Cow::Borrowed(text) => Ok(text),
        Cow::Owned(_) => Err("a decoded frame's tag was not lent".into()),
    }
}

pub fn reserved_zero_tag(frame: &dyn replay_frame::frame::DecodedSource) -> Result<&str, String> {
    lent(reserved_zero_tag_in(frame))
}

pub fn reserved_zero_original_tag(
    frame: &dyn replay_frame::frame::DecodedSource,
) -> Result<&str, String> {
    lent(reserved_zero_original_tag_in(frame))
}

pub fn original_or_own_tag(
    frame: &dyn replay_frame::frame::DecodedSource,
    slot: u32,
) -> Result<&str, String> {
    lent(original_or_own_tag_in(frame, slot))
}

pub fn reserved_zero_tag_in<F: Tables + ?Sized>(frame: &F) -> Result<Cow<'_, str>, String> {
    row_text(reserved_zero_row(frame)?, |row| &row.tag, "reserved country tag")
}

pub fn reserved_zero_original_tag_in<F: Tables + ?Sized>(
    frame: &F,
) -> Result<Cow<'_, str>, String> {
    let row = reserved_zero_row(frame)?;
    let reference = row
        .original_tag_reference
        .require("original tag reference of reserved country naming")?
        .ok_or("original tag reference of reserved country naming capture refused")?;
    if reference > 0 {
        row.original_tag
            .require("original tag of reserved country naming")?
            .as_ref()
            .ok_or("original tag of reserved country naming capture refused")?;
    }
    row_text(
        row,
        |row| match (reference > 0, row.original_tag.require("")) {
            (true, Ok(Some(tag))) => tag,
            _ => &row.tag,
        },
        "reserved original-or-own tag",
    )
}

pub fn original_or_own_tag_in<F: Tables + ?Sized>(
    frame: &F,
    slot: u32,
) -> Result<Cow<'_, str>, String> {
    if slot == 0 {
        return reserved_zero_original_tag_in(frame);
    }
    match original_rows(frame, slot)? {
        (Cow::Borrowed(country), Cow::Borrowed(row)) => {
            original_tag(country, row).map(Cow::Borrowed)
        }
        (country, row) => original_tag(&country, &row).map(|tag| Cow::Owned(tag.to_owned())),
    }
}

pub fn original_or_own_reference<F: Tables + ?Sized>(frame: &F, slot: u32) -> Result<i32, String> {
    if slot == 0 {
        return reserved_reference_of(&reserved_zero_row(frame)?.original_tag_reference);
    }
    let (country, row) = original_rows(frame, slot)?;
    original_or_own_reference_of(&row.original_tag_reference, country.id)
}

pub fn original_or_own_reference_of(reference: &Recorded<Option<i32>>, own: u32) -> Result<i32, String> {
    let raw = reference
        .require("original tag reference of country appearance")?
        .ok_or("original tag reference of country appearance capture refused")?;
    Ok(if raw > 0 { raw } else { own as i32 })
}

pub fn reserved_reference_of(reference: &Recorded<Option<i32>>) -> Result<i32, String> {
    let raw = reference
        .require("original tag reference of reserved country naming")?
        .ok_or("original tag reference of reserved country naming capture refused")?;
    Ok(if raw > 0 { raw } else { 0 })
}

fn original_rows<F: Tables + ?Sized>(
    frame: &F,
    slot: u32,
) -> Result<(Cow<'_, CountryRow>, Cow<'_, CountryAppearanceRow>), String> {
    let (countries, countries_complete) = partial::<F, tables::Countries>(frame, COUNTRY);
    let mut last = None;
    for row in countries.with_mask(SLOT).iter() {
        let row = row.map_err(|why| why.to_string())?;
        if row.canonical_slot == slot {
            last = Some(row.key());
        }
    }
    let country = match last {
        Some(key) => keyed(&countries, key)?,
        None => None,
    }
    .ok_or_else(|| {
        if countries_complete {
            format!("country slot {slot} has no Countries row")
        } else {
            "countries is incomplete this hour: a country is unknown".to_owned()
        }
    })?;
    let (appearance, appearance_complete) =
        partial::<F, tables::CountryAppearance>(frame, APPEARANCE);
    let row = keyed(&appearance, u128::from(country.id))?.ok_or_else(|| {
        if appearance_complete {
            format!("country slot {slot} has no country appearance row")
        } else {
            "country appearance is incomplete this hour: a country is unknown".to_owned()
        }
    })?;
    Ok((country, row))
}

pub fn reserved_zero<F: Tables + ?Sized>(frame: &F) -> Result<Operands, String> {
    let row = reserved_zero_row(frame)?;
    let tag = if row.name_tag.is_empty() {
        &row.tag
    } else {
        &row.name_tag
    };
    let overlord = if row.overlord > 0 {
        let links = frame
            .complete_rows::<tables::CountryLinks>(LINK)
            .map_err(|why| format!("country links is unavailable this hour: {why}"))?;
        Some(
            keyed(&links, u128::from(row.overlord as u32))?
                .ok_or_else(|| "the overlord has no recorded country link".to_owned())?
                .canonical_slot,
        )
    } else {
        None
    };
    let (overlord_suffix, autonomy_suffix) = match (overlord, row.autonomy.as_deref()) {
        (Some(slot), Some(autonomy)) => {
            if autonomy.is_empty() {
                return Err("reserved current autonomy has no definition".into());
            }
            let countries = frame
                .complete_rows::<tables::Countries>(COUNTRY)
                .map_err(|why| format!("Countries is unavailable this hour: {why}"))?;
            let mut overlord = None;
            for country in countries.iter() {
                let country = country.map_err(|why| why.to_string())?;
                if country.canonical_slot == slot {
                    overlord = Some(country);
                }
            }
            let overlord =
                overlord.ok_or_else(|| format!("overlord slot {slot} has no country"))?;
            (
                underscore(text(&overlord.tag, "overlord tag")?),
                underscore(text(autonomy, "autonomy")?),
            )
        }
        _ => (String::new(), String::new()),
    };
    Ok(Operands {
        tag: text(tag, "reserved naming tag")?,
        cosmetic: text(&row.cosmetic_tag, "reserved cosmetic tag")?,
        overlord,
        overlord_suffix,
        subtype_suffix: underscore(text(&row.leader_ideology, "reserved leader ideology")?),
        group_suffix: underscore(
            row.group
                .as_deref()
                .map(|group| text(group, "reserved ideology group"))
                .transpose()?
                .unwrap_or_default(),
        ),
        autonomy_suffix,
    })
}

fn underscore(token: String) -> String {
    if token.is_empty() {
        token
    } else {
        format!("_{token}")
    }
}

pub use replay_data::country_flags::original_tag;

const SLOT: u64 = mask::<CountryRow>(&["canonical_slot"]);
const COUNTRY: u64 = mask::<CountryRow>(&["canonical_slot", "tag"]);
const LINK: u64 = mask::<CountryLinkRow>(&["canonical_slot"]);
const APPEARANCE: u64 = mask::<CountryAppearanceRow>(&[
    "cosmetic_tag",
    "name_tag",
    "leader_ideology",
    "overlord",
    "original_tag",
    "original_tag_reference",
]);
const POLITICS: u64 = mask::<PoliticsRow>(&["ruling_party"]);
const PARTY: u64 = mask::<PoliticalPartyRow>(&["token"]);
const AUTONOMY: u64 = mask::<AutonomyRow>(&["state"]);
const RESERVED: u64 = mask::<ReservedCountryNamingRow>(&[
    "tag",
    "name_tag",
    "cosmetic_tag",
    "leader_ideology",
    "group",
    "overlord",
    "autonomy",
    "built",
    "original_tag",
    "original_tag_reference",
]);

struct Inputs<'f> {
    countries: Rowset<'f, CountryRow>,
    slots: BTreeMap<u32, u128>,
    links: Result<Rowset<'f, CountryLinkRow>, String>,
    appearance: (Rowset<'f, CountryAppearanceRow>, bool),
    politics: (Rowset<'f, PoliticsRow>, bool),
    parties: (Rowset<'f, PoliticalPartyRow>, bool),
    autonomy: (Rowset<'f, AutonomyRow>, bool),
}

impl<'f> Inputs<'f> {
    fn read<F: Tables + ?Sized>(frame: &'f F) -> Option<Self> {
        let countries = frame.complete_rows::<tables::Countries>(COUNTRY).ok()?;
        let mut slots = BTreeMap::new();
        for row in countries.with_mask(SLOT).iter() {
            let row = row.ok()?;
            slots.insert(row.canonical_slot, row.key());
        }
        Some(Self {
            countries,
            slots,
            links: frame
                .complete_rows::<tables::CountryLinks>(LINK)
                .map_err(|why| format!("country links is unavailable this hour: {why}")),
            appearance: partial::<F, tables::CountryAppearance>(frame, APPEARANCE),
            politics: partial::<F, tables::Politics>(frame, POLITICS),
            parties: partial::<F, tables::PoliticalParties>(frame, PARTY),
            autonomy: partial::<F, tables::Autonomy>(frame, AUTONOMY),
        })
    }

    fn country(&self, slot: u32) -> Result<Option<Cow<'f, CountryRow>>, String> {
        match self.slots.get(&slot) {
            Some(&key) => keyed(&self.countries, key),
            None => Ok(None),
        }
    }

    fn operands(&self, country: &CountryRow) -> Result<Operands, String> {
        naming_operands(self, country.id, &country.tag)
    }
}

impl NamingRows for Inputs<'_> {
    fn appearance(&self, id: u32) -> Result<Option<NamingAppearance>, String> {
        Ok(keyed(&self.appearance.0, u128::from(id))?.map(|row| NamingAppearance::of(&row)))
    }

    fn ruling_party(&self, id: u32) -> Result<Option<Option<u32>>, String> {
        Ok(keyed(&self.politics.0, u128::from(id))?.map(|row| row.ruling_party))
    }

    fn party_token(&self, id: u32, party: u32) -> Result<Option<Vec<u8>>, String> {
        Ok(keyed(&self.parties.0, u128::from(id) << 32 | u128::from(party))?.map(|row| row.token.clone()))
    }

    fn link(&self, raw: u32) -> Result<Option<u32>, String> {
        let links = self.links.as_ref().map_err(String::clone)?;
        Ok(keyed(links, u128::from(raw))?.map(|row| row.canonical_slot))
    }

    fn autonomy(&self, id: u32) -> Result<Option<Vec<u8>>, String> {
        Ok(keyed(&self.autonomy.0, u128::from(id))?.map(|row| row.state.clone()))
    }

    fn slot_tag(&self, slot: u32) -> Result<Option<Vec<u8>>, String> {
        Ok(self.country(slot)?.map(|country| country.tag.clone()))
    }

    fn complete(&self, table: TableId) -> bool {
        match table {
            TableId::CountryAppearance => self.appearance.1,
            TableId::Politics => self.politics.1,
            TableId::PoliticalParties => self.parties.1,
            TableId::Autonomy => self.autonomy.1,
            _ => true,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NamingAppearance {
    pub name_tag: Vec<u8>,
    pub cosmetic_tag: Vec<u8>,
    pub leader_ideology: Vec<u8>,
    pub overlord: i32,
}

impl NamingAppearance {
    pub fn of(row: &CountryAppearanceRow) -> Self {
        Self {
            name_tag: row.name_tag.clone(),
            cosmetic_tag: row.cosmetic_tag.clone(),
            leader_ideology: row.leader_ideology.clone(),
            overlord: row.overlord,
        }
    }
}

pub trait NamingRows {
    fn appearance(&self, id: u32) -> Result<Option<NamingAppearance>, String>;
    fn ruling_party(&self, id: u32) -> Result<Option<Option<u32>>, String>;
    fn party_token(&self, id: u32, party: u32) -> Result<Option<Vec<u8>>, String>;
    fn link(&self, raw: u32) -> Result<Option<u32>, String>;
    fn autonomy(&self, id: u32) -> Result<Option<Vec<u8>>, String>;
    fn slot_tag(&self, slot: u32) -> Result<Option<Vec<u8>>, String>;
    fn complete(&self, table: TableId) -> bool;
}

pub fn naming_operands(rows: &impl NamingRows, id: u32, own_tag: &[u8]) -> Result<Operands, String> {
    let table = |table: TableId| (!rows.complete(table)).then(|| format!("{} is incomplete this hour", table.words()));
    let row = rows.appearance(id)?.ok_or_else(|| {
        table(TableId::CountryAppearance)
            .unwrap_or_else(|| "no naming operands recorded".into())
    })?;
    let (bytes, what) = if row.name_tag.is_empty() {
        (own_tag, "country tag")
    } else {
        (row.name_tag.as_slice(), "original tag")
    };
    let tag = std::str::from_utf8(bytes)
        .map_err(|_| format!("{what} is not UTF-8"))?
        .to_owned();
    let group = match rows.ruling_party(id)? {
        Some(Some(party)) => match rows.party_token(id, party)? {
            Some(token) => text(&token, "ideology group")?,
            None => {
                return Err(table(TableId::PoliticalParties)
                    .unwrap_or_else(|| "ruling party has no recorded row".into()));
            }
        },
        Some(None) => String::new(),
        None => {
            return Err(table(TableId::Politics)
                .unwrap_or_else(|| "no politics row recorded".into()));
        }
    };
    let overlord = if row.overlord > 0 {
        Some(
            rows.link(row.overlord as u32)?
                .ok_or_else(|| "the overlord has no recorded country link".to_owned())?,
        )
    } else {
        None
    };
    let state = match overlord {
        Some(_) => rows.autonomy(id)?,
        None => None,
    };
    let (overlord_suffix, autonomy_suffix) = match (overlord, state) {
        (Some(slot), Some(state)) => {
            if state.is_empty() {
                return Err("current autonomy has no definition".into());
            }
            let overlord = rows
                .slot_tag(slot)?
                .ok_or_else(|| format!("overlord slot {slot} has no country"))?;
            (
                format!("_{}", text(&overlord, "overlord tag")?),
                format!("_{}", text(&state, "autonomy")?),
            )
        }
        (Some(_), None) if !rows.complete(TableId::Autonomy) => {
            return Err("autonomy is incomplete this hour".into());
        }
        _ => (String::new(), String::new()),
    };
    Ok(Operands {
        tag,
        cosmetic: text(&row.cosmetic_tag, "cosmetic tag")?,
        overlord,
        overlord_suffix,
        subtype_suffix: underscore(text(&row.leader_ideology, "leader ideology")?),
        group_suffix: underscore(group),
        autonomy_suffix,
    })
}

fn operands<F: Tables + ?Sized>(frame: &F) -> BTreeMap<u32, Result<Operands, String>> {
    let Some(inputs) = Inputs::read(frame) else {
        return BTreeMap::new();
    };
    inputs
        .slots
        .iter()
        .map(|(&slot, &key)| {
            let operands = match keyed(&inputs.countries, key) {
                Ok(Some(country)) => inputs.operands(&country),
                Ok(None) => Err(format!("country slot {slot} has no Countries row")),
                Err(reason) => Err(reason),
            };
            (slot, operands)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_data::{
        rows::AutonomyRow, rows::CountryAppearanceRow, rows::CountryLinkRow, rows::CountryRow,
        rows::PoliticalPartyRow, rows::PoliticsRow, rows::TableId, rows::TableRows,
    };
    use replay_frame::{frame::Metrics, frame::TableView};

    fn localisation(entries: &[(&str, &str)]) -> Localisation {
        entries
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn input(tag: &str) -> Operands {
        Operands {
            tag: tag.into(),
            ..Default::default()
        }
    }

    #[test]
    fn flag_keys_take_the_cosmetic_then_the_original_or_own_tag_each_under_its_group() {
        assert_eq!(
            flag_keys(Some("Z01"), "MOD_REPUBLIC", Some("custom_ideology")),
            [
                "MOD_REPUBLIC_custom_ideology",
                "MOD_REPUBLIC",
                "Z01_custom_ideology",
                "Z01"
            ]
        );
        assert_eq!(
            flag_keys(Some("Z01"), "", Some("custom_ideology")),
            ["Z01_custom_ideology", "Z01"]
        );
        assert_eq!(flag_keys(Some("Z01"), "", None), ["Z01"]);
        assert_eq!(
            flag_keys(Some("GER"), "", Some("fascism")),
            ["GER_fascism", "GER"]
        );
        assert_eq!(
            flag_keys(Some("GER"), "GER_NEW", Some("")),
            ["GER_NEW_", "GER_NEW", "GER_", "GER"]
        );
        assert_eq!(flag_keys(None, "COS", Some("group")), ["COS_group", "COS"]);
        assert_eq!(flag_keys(Some(""), "", Some("")), ["_", ""]);
    }

    #[test]
    fn subject_keys_keep_precedence_and_case_retry() {
        let subject = Operands {
            overlord_suffix: "_ENG".into(),
            subtype_suffix: "_conservatism".into(),
            group_suffix: "_democratic".into(),
            autonomy_suffix: "_autonomy_dominion".into(),
            ..input("CAN")
        };
        let keys = subject.keys("CAN", "");
        assert_eq!(keys[0], "CAN_ENG_conservatism_autonomy_dominion");
        assert_eq!(keys[5], "COUNTRY_ENG_autonomy_dominion");
        assert_eq!(keys[12], "COUNTRY_autonomy_dominion");
        assert_eq!(keys.last().unwrap(), "CAN");
        assert_eq!(
            subject.choose("CAN", "", |k| k == "CAN_democratic"
                || k == "CAN_ENG_subject"),
            Some("CAN_ENG_subject".into())
        );
        let plain = Operands {
            subtype_suffix: "_conservatism".into(),
            group_suffix: "_democratic".into(),
            ..input("ENG")
        };
        assert_eq!(
            plain.keys("ENG", "_ADJ"),
            ["ENG_conservatism_ADJ", "ENG_democratic_ADJ", "ENG_ADJ"]
        );
        assert_eq!(
            plain.choose("ENG", "_ADJ", |k| k == "ENG_adj"),
            Some("ENG_adj".into())
        );
    }

    #[test]
    fn cosmetic_candidate_reuses_subject_ladder_and_definite_case_retry() {
        let operands = Operands {
            overlord_suffix: "_AAA".into(),
            subtype_suffix: "_sub".into(),
            group_suffix: "_group".into(),
            autonomy_suffix: "_autonomy".into(),
            ..input("BBB")
        };
        let first = localisation(&[
            ("SYN_AAA_sub_autonomy_def", "Subject"),
            ("SYN_group_DEF", "Group"),
            ("SYN_DEF", "Base"),
        ]);
        assert_eq!(
            operands.candidate("SYN", "_DEF", &first),
            Some("SYN_group_DEF".into())
        );
        let localisation = localisation(&[("SYN_AAA_sub_autonomy_def", "Subject")]);
        assert_eq!(
            operands.candidate("SYN", "_DEF", &localisation),
            Some("SYN_AAA_sub_autonomy_def".into())
        );
        assert_eq!(operands.candidate("UNDEFINED", "_DEF", &localisation), None);
    }

    fn names<'a>(localisation: &'a Localisation, inputs: Vec<(u32, Operands)>) -> CountryNames<'a> {
        CountryNames {
            localisation,
            operands: inputs.into_iter().map(|(k, v)| (k, Ok(v))).collect(),
            reserved_built: None,
        }
    }

    #[test]
    fn subject_names_expand_the_overlord_and_non_ideology_names() {
        let localisation = localisation(&[
            ("ENG_democratic_ADJ", "British"),
            ("ENG_democratic_DEF", "the United Kingdom"),
            ("RAJ_UK_subject", "$OVERLORDADJ$ $NONIDEOLOGY$"),
            ("RAJ", "Raj"),
            ("RAJ_DEF", "India"),
            (
                "RAJ_UK_subject_DEF",
                "$NONIDEOLOGYDEF$ of $OVERLORDNAMEDEF$",
            ),
        ]);
        let names = names(
            &localisation,
            vec![
                (
                    1,
                    Operands {
                        group_suffix: "_democratic".into(),
                        ..input("ENG")
                    },
                ),
                (
                    2,
                    Operands {
                        cosmetic: "RAJ_UK".into(),
                        overlord: Some(1),
                        overlord_suffix: "_ENG".into(),
                        autonomy_suffix: "_autonomy_puppet".into(),
                        group_suffix: "_neutrality".into(),
                        ..input("RAJ")
                    },
                ),
            ],
        );
        assert_eq!(names.name(2).as_deref(), Ok("British Raj"));
        assert_eq!(
            names.name_with(2, "_DEF").as_deref(),
            Ok("India of the United Kingdom")
        );
    }

    #[test]
    fn scripted_text_is_refused_and_cycles_and_long_text_follow_the_localiser() {
        let scripted = localisation(&[("TAG", "[ROOT.GetName]")]);
        assert!(
            names(&scripted, vec![(1, input("TAG"))])
                .name(1)
                .unwrap_err()
                .contains("scripted")
        );
        let cyclic = localisation(&[("TAG", "Name $ALIAS$"), ("ALIAS", "$TAG$")]);
        assert_eq!(
            names(&cyclic, vec![(1, input("TAG"))]).name(1),
            Ok("Name ".repeat(16))
        );
        let doubling = |leaf: &str| -> Localisation {
            (0..12)
                .map(|i| (format!("A{i}"), format!("$A{}$$A{}$", i + 1, i + 1)))
                .chain([("A12".to_owned(), leaf.to_owned())])
                .collect()
        };
        for (leaf, length) in [("x", 4096), ("xx", 0x1800)] {
            assert_eq!(
                names(&doubling(leaf), vec![(1, input("A0"))]).name(1),
                Ok("x".repeat(length))
            );
        }
    }

    #[test]
    fn country_names_use_the_localisers_recovery_and_argument_formats() {
        let localisation = localisation(&[
            ("IND", "Federation $UNDEFINED_NAME$"),
            ("FREE", "$OVERLORDADJ$Republic"),
            ("ENG_ADJ", "british"),
            ("SUB_ENG_subject", "$OVERLORDADJ|U$ $NONIDEOLOGY$"),
            ("SUB", "$OVERLORDADJ$Sub"),
            ("ONLY_ENG_subject", "$NONIDEOLOGYDEF$"),
            ("ONLY_DEF", "the only"),
        ]);
        let subject = |tag: &str, overlord: u32| Operands {
            overlord: Some(overlord),
            overlord_suffix: "_ENG".into(),
            autonomy_suffix: "_autonomy".into(),
            ..input(tag)
        };
        let mut operands: BTreeMap<u32, Result<Operands, String>> = [
            (1, Ok(input("IND"))),
            (2, Ok(input("FREE"))),
            (3, Ok(input("ENG"))),
            (4, Ok(subject("SUB", 3))),
            (5, Ok(subject("ONLY", 9))),
            (6, Ok(subject("SUB", 9))),
        ]
        .into();
        operands.insert(9, Err("overlord operands are unavailable".into()));
        let names = CountryNames::of_operands(&localisation, operands);
        assert_eq!(names.name(1).as_deref(), Ok("Federation "));
        assert_eq!(names.name(2).as_deref(), Ok("Republic"));
        assert_eq!(names.name(4).as_deref(), Ok("British Sub"));
        assert_eq!(names.name(5).as_deref(), Ok("the only"));
        assert_eq!(
            names.name(6),
            Err("overlord operands are unavailable".into())
        );
    }

    #[test]
    fn escaped_and_lone_dollar_signs_are_text() {
        let localisation =
            localisation(&[("TAG", "Cash $$ republic of $ALIAS$ $"), ("ALIAS", "A")]);
        assert_eq!(
            names(&localisation, vec![(1, input("TAG"))])
                .name(1)
                .as_deref(),
            Ok("Cash $ republic of A $")
        );
    }

    fn frame() -> DecodedFrame {
        let mut tables = std::array::from_fn(|_| TableView::Unavailable {
            reason: "fixture".into(),
        });
        tables[TableId::Countries.index()] =
            TableView::Complete(TableRows::Countries(vec![CountryRow {
                id: 400,
                canonical_slot: 30,
                tag: b"D01".to_vec(),
                ..Default::default()
            }]));
        tables[TableId::CountryLinks.index()] =
            TableView::Complete(TableRows::CountryLinks(vec![CountryLinkRow {
                id: 400,
                canonical_slot: 30,
            }]));
        tables[TableId::CountryAppearance.index()] =
            TableView::Complete(TableRows::CountryAppearance(vec![CountryAppearanceRow {
                country: 400,
                cosmetic_tag: b"GER_NEW".to_vec(),
                name_tag: b"GER".to_vec(),
                leader_ideology: b"nazism".to_vec(),
                ..Default::default()
            }]));
        tables[TableId::Politics.index()] =
            TableView::Complete(TableRows::Politics(vec![PoliticsRow {
                country: 400,
                ruling_party: Some(7),
                ..Default::default()
            }]));
        tables[TableId::PoliticalParties.index()] =
            TableView::Complete(TableRows::PoliticalParties(vec![PoliticalPartyRow {
                country: 400,
                id: 7,
                token: b"fascism".to_vec(),
                ..Default::default()
            }]));
        tables[TableId::Autonomy.index()] =
            TableView::Complete(TableRows::Autonomy(Vec::<AutonomyRow>::new()));
        DecodedFrame {
            branch: 1,
            hour: 0,
            tables: tables.map(Into::into),
            metrics: Metrics::default(),
        }
    }

    #[test]
    fn dynamic_countries_take_their_original_tag_and_missing_inputs_are_named() {
        let localisation = localisation(&[
            ("GER_NEW_nazism", "Fixture successor"),
            ("GER_fascism", "Fixture federation"),
            ("D01_fascism", "Wrong dynamic tag"),
        ]);
        let mut frame = frame();
        assert_eq!(
            CountryNames::read(&frame, &localisation)
                .name(30)
                .as_deref(),
            Ok("Fixture successor")
        );
        if let TableView::Complete(TableRows::CountryAppearance(rows)) =
            &mut *frame.tables[TableId::CountryAppearance.index()]
        {
            rows[0].cosmetic_tag.clear();
            rows[0].leader_ideology.clear();
        }
        assert_eq!(
            CountryNames::read(&frame, &localisation)
                .name(30)
                .as_deref(),
            Ok("Fixture federation")
        );
        *frame.tables[TableId::Politics.index()] = TableView::Unavailable {
            reason: "missing".into(),
        };
        assert_eq!(
            CountryNames::read(&frame, &localisation).name(30),
            Err("politics is incomplete this hour".into())
        );
        let selected = CountryNames::read_selected(&frame, &localisation, [30]);
        assert_eq!(
            selected.name(30),
            Err("politics is incomplete this hour".into())
        );
    }

    #[test]
    fn missing_country_links_withhold_only_names_that_follow_an_overlord() {
        let localisation = localisation(&[("GER_NEW_nazism", "Fixture successor")]);
        let mut frame = frame();
        *frame.tables[TableId::CountryLinks.index()] = TableView::Unavailable {
            reason: "unrecorded links".into(),
        };
        let read = CountryNames::read(&frame, &localisation);
        let selected = CountryNames::read_selected(&frame, &localisation, [30]);
        for names in [&read, &selected] {
            assert_eq!(names.name(30).as_deref(), Ok("Fixture successor"));
        }
        if let TableView::Complete(TableRows::CountryAppearance(rows)) =
            &mut *frame.tables[TableId::CountryAppearance.index()]
        {
            rows[0].overlord = 7;
        }
        let read = CountryNames::read(&frame, &localisation);
        let selected = CountryNames::read_selected(&frame, &localisation, [30]);
        for names in [&read, &selected] {
            let why = names.name(30).unwrap_err();
            assert!(
                why.starts_with("country links is unavailable this hour"),
                "{why}"
            );
        }
    }

    #[test]
    fn reserved_zero_requires_its_singleton_and_preserves_each_unbuilt_cache() {
        let localisation = localisation(&[
            ("GER_NEW_nazism", "Fixture successor"),
            ("ZERO_mod_group", "Zero"),
            ("ZERO_mod_group_DEF", "the zero"),
            ("ZERO_mod_group_ADJ", "zeroish"),
        ]);
        let mut frame = frame();
        let names = CountryNames::read(&frame, &localisation);
        assert_eq!(names.name(30).as_deref(), Ok("Fixture successor"));
        assert!(names.name(0).unwrap_err().contains("ReservedCountryNaming"));
        *frame.tables[TableId::ReservedCountryNaming.index()] =
            TableView::Complete(TableRows::ReservedCountryNaming(vec![
                ReservedCountryNamingRow {
                    tag: b"ZERO".to_vec(),
                    group: Some(b"mod_group".to_vec()),
                    built: 3,
                    ..Default::default()
                },
            ]));
        let names = CountryNames::read(&frame, &localisation);
        assert_eq!(names.name(0).as_deref(), Ok("Zero"));
        assert_eq!(names.name_with(0, "_DEF").as_deref(), Ok("the zero"));
        assert_eq!(names.name_with(0, "_ADJ").as_deref(), Ok(""));
        assert_eq!(reserved_zero_tag(&frame), Ok("ZERO"));
        if let TableView::Complete(TableRows::ReservedCountryNaming(rows)) =
            &mut *frame.tables[TableId::ReservedCountryNaming.index()]
        {
            rows[0].built = 0;
            rows[0].cosmetic_tag = vec![0xff];
        }
        let names = CountryNames::read_selected(&frame, &localisation, [0]);
        assert_eq!(names.name(0).as_deref(), Ok(""));
        assert_eq!(names.name_with(0, "_DEF").as_deref(), Ok(""));
        assert_eq!(names.name_with(0, "_ADJ").as_deref(), Ok(""));
        assert_eq!(frame.complete::<tables::Countries>().unwrap().len(), 1);
        assert_eq!(
            frame.complete::<tables::CountryAppearance>().unwrap().len(),
            1
        );
    }

    #[test]
    fn original_getters_use_signed_reference_and_keep_optional_bytes_lazy() {
        use replay_data::rows::Recorded;
        let mut frame = frame();
        *frame.tables[TableId::CountryLinks.index()] = TableView::Unavailable {
            reason: "unrelated links".into(),
        };
        let cases = [
            (
                Recorded::Value(Some(1)),
                Recorded::Value(Some(b"ORIGINAL".to_vec())),
                Ok("ORIGINAL"),
            ),
            (
                Recorded::Value(Some(1)),
                Recorded::Value(Some(Vec::new())),
                Ok(""),
            ),
            (Recorded::Value(Some(0)), Recorded::NotRecorded, Ok("OWN")),
            (Recorded::Value(Some(-7)), Recorded::Value(None), Ok("OWN")),
            (
                Recorded::Value(Some(1)),
                Recorded::Value(None),
                Err("bytes refused"),
            ),
            (
                Recorded::Value(Some(1)),
                Recorded::NotRecorded,
                Err("bytes absent"),
            ),
            (
                Recorded::Value(None),
                Recorded::Value(Some(b"ORIGINAL".to_vec())),
                Err("reference refused"),
            ),
            (
                Recorded::NotRecorded,
                Recorded::Value(Some(Vec::new())),
                Err("reference absent"),
            ),
        ];
        for (reference, bytes, expected) in cases {
            *frame.tables[TableId::CountryAppearance.index()] =
                TableView::Complete(TableRows::CountryAppearance(vec![CountryAppearanceRow {
                    country: 400,
                    name_tag: Vec::new(),
                    original_tag_reference: reference,
                    original_tag: bytes.clone(),
                    ..Default::default()
                }]));
            *frame.tables[TableId::ReservedCountryNaming.index()] =
                TableView::Complete(TableRows::ReservedCountryNaming(vec![
                    ReservedCountryNamingRow {
                        tag: b"ZZ0".to_vec(),
                        original_tag_reference: reference,
                        original_tag: bytes,
                        ..Default::default()
                    },
                ]));
            for (slot, own, prefix) in [
                (30, "D01", "country appearance"),
                (0, "ZZ0", "reserved country naming"),
            ] {
                let result = original_or_own_tag(&frame, slot);
                match expected {
                    Ok("OWN") => assert_eq!(result, Ok(own)),
                    Ok(text) => assert_eq!(result, Ok(text)),
                    Err(why) => {
                        let field = if why.starts_with("reference") {
                            "original tag reference"
                        } else {
                            "original tag"
                        };
                        let suffix = if why.ends_with("absent") {
                            "not recorded"
                        } else {
                            "capture refused"
                        };
                        assert_eq!(result, Err(format!("{field} of {prefix} {suffix}")));
                    }
                }
                let selected = original_or_own_reference(&frame, slot);
                match &reference {
                    Recorded::Value(Some(raw)) => assert_eq!(
                        selected,
                        Ok(if *raw > 0 {
                            *raw
                        } else if slot == 0 {
                            0
                        } else {
                            400
                        })
                    ),
                    _ => assert!(selected.is_err()),
                }
            }
        }
        if let TableView::Complete(TableRows::CountryAppearance(rows)) =
            &mut *frame.tables[TableId::CountryAppearance.index()]
        {
            rows[0].original_tag_reference = Recorded::Value(Some(1));
            rows[0].original_tag = Recorded::Value(Some(vec![0xff]));
        }
        assert_eq!(
            original_or_own_tag(&frame, 30),
            Err("country original-or-own tag is not UTF-8".into())
        );
        if let TableView::Complete(TableRows::Countries(rows)) =
            &mut *frame.tables[TableId::Countries.index()]
        {
            rows[0].id = u32::MAX;
        }
        if let TableView::Complete(TableRows::CountryAppearance(rows)) =
            &mut *frame.tables[TableId::CountryAppearance.index()]
        {
            rows[0].country = u32::MAX;
            rows[0].original_tag_reference = Recorded::Value(Some(0));
            rows[0].original_tag = Recorded::NotRecorded;
        }
        assert_eq!(original_or_own_reference(&frame, 30), Ok(-1));
        assert_eq!(original_or_own_tag(&frame, 30), Ok("D01"));
        assert!(
            original_or_own_tag(&frame, 31)
                .unwrap_err()
                .contains("has no Countries row")
        );
    }

    #[test]
    fn selected_names_read_only_the_subject_and_overlord() {
        let mut frame = frame();
        let countries: Vec<_> = (1..=5)
            .map(|id| CountryRow {
                id,
                canonical_slot: match id {
                    5 => 1,
                    3 | 4 => 2,
                    _ => 10000 + id,
                },
                tag: match id {
                    5 => b"SUB".to_vec(),
                    4 => b"OWN".to_vec(),
                    _ => vec![0xff],
                },
                ..Default::default()
            })
            .collect();
        let links = countries
            .iter()
            .map(|row| CountryLinkRow {
                id: row.id,
                canonical_slot: row.canonical_slot,
            })
            .collect();
        let appearance = countries
            .iter()
            .map(|row| CountryAppearanceRow {
                country: row.id,
                overlord: if row.id == 5 { 4 } else { 0 },
                ..Default::default()
            })
            .collect();
        let politics = countries
            .iter()
            .map(|row| PoliticsRow {
                country: row.id,
                ..Default::default()
            })
            .collect();
        *frame.tables[TableId::Countries.index()] =
            TableView::Complete(TableRows::Countries(countries));
        *frame.tables[TableId::CountryLinks.index()] =
            TableView::Complete(TableRows::CountryLinks(links));
        *frame.tables[TableId::CountryAppearance.index()] =
            TableView::Complete(TableRows::CountryAppearance(appearance));
        *frame.tables[TableId::Politics.index()] =
            TableView::Complete(TableRows::Politics(politics));
        *frame.tables[TableId::Autonomy.index()] =
            TableView::Complete(TableRows::Autonomy(vec![AutonomyRow {
                subject: 5,
                state: b"subject".to_vec(),
                ..Default::default()
            }]));
        let localisation = localisation(&[
            ("SUB_OWN_subject", "$NONIDEOLOGY$ of $OVERLORDNAMEDEF$"),
            ("SUB", "subject"),
            ("OWN_DEF", "the owner"),
            ("ZERO_OWN_subject", "$NONIDEOLOGY$ of $OVERLORDNAMEDEF$"),
            ("ZERO", "reserved"),
        ]);
        *frame.tables[TableId::ReservedCountryNaming.index()] =
            TableView::Complete(TableRows::ReservedCountryNaming(vec![
                ReservedCountryNamingRow {
                    tag: b"ZERO".to_vec(),
                    overlord: 4,
                    autonomy: Some(b"subject".to_vec()),
                    built: 7,
                    ..Default::default()
                },
            ]));
        let selected = CountryNames::read_selected(&frame, &localisation, [1]);
        assert_eq!(selected.operands.len(), 2);
        assert!(selected.operands(10001).is_none());
        assert_eq!(selected.name(1).as_deref(), Ok("subject of the owner"));
        assert_eq!(
            selected.name(1),
            CountryNames::read(&frame, &localisation).name(1)
        );
        let selected = CountryNames::read_selected(&frame, &localisation, [0]);
        assert_eq!(selected.operands.len(), 2);
        assert_eq!(selected.name(0).as_deref(), Ok("reserved of the owner"));
        assert_eq!(
            selected.name(0),
            CountryNames::read(&frame, &localisation).name(0)
        );
    }

    #[test]
    fn a_chain_of_overlords_as_deep_as_the_campaign_names_its_deepest_subject() {
        let localisation = localisation(&[("ROOT_ADJ", "root"), ("SUB", "$OVERLORDADJ$"), ("SUB_ADJ", "$OVERLORDADJ$")]);
        let mut operands = vec![(1, input("ROOT"))];
        for slot in 2..=1500 {
            operands.push((slot, Operands { overlord: Some(slot - 1), ..input("SUB") }));
        }
        assert_eq!(names(&localisation, operands).name(1500).as_deref(), Ok("root"));
    }

    #[test]
    fn overlords_naming_each_other_refuse_only_the_names_that_reach_the_cycle() {
        let localisation = localisation(&[("AAA", "$OVERLORDADJ$"), ("AAA_ADJ", "$OVERLORDADJ$"), ("BBB_ADJ", "$OVERLORDADJ$"), ("BBB", "Plain")]);
        let subject = |tag: &str, overlord| Operands { overlord: Some(overlord), ..input(tag) };
        let names = names(&localisation, vec![(1, subject("AAA", 2)), (2, subject("BBB", 1))]);
        assert!(names.name(1).unwrap_err().contains("cycle"));
        assert_eq!(names.name(2).as_deref(), Ok("Plain"), "a text not reaching its overlord is built");
    }
}

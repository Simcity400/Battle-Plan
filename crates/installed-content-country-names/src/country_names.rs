//! Country display names in the reader's language, rebuilt from recorded
//! operands. Key selection tries cosmetic keys before tag keys, then expands
//! subject substitutions. The game stores the result in its client language,
//! so the recording keeps the operands needed to localise it independently.
//!
//! Operands, per canonical country slot:
//! - name tag: the original tag of a dynamic country, else its own tag;
//! - cosmetic tag, first leader ideology and raw overlord: `CountryAppearance`;
//! - ruling ideology group: `Politics.ruling_party` -> `PoliticalParties.token`;
//! - current autonomy definition: `Autonomy.state`;
//! - overlord tag: the overlord's `Countries.tag`.
//!
//! Scripted localisation (`[...]`) is refused, never drawn as text.

use std::collections::{BTreeMap, BTreeSet};

use replay_data::{
    rows::CountryAppearanceRow, rows::CountryLinkRow, rows::CountryRow,
    rows::ReservedCountryNamingRow, rows::TableId, rows::TableSchema, rows::tables,
};
use replay_frame::{frame::DecodedFrame, frame::TableView};

use crate::localisation::Localisation;

/// Every table the resolver reads, for a reader that reconstructs only those.
pub const TABLES: [TableId; 7] = [
    TableId::Countries,
    TableId::CountryLinks,
    TableId::CountryAppearance,
    TableId::Politics,
    TableId::PoliticalParties,
    TableId::Autonomy,
    TableId::ReservedCountryNaming,
];

/// One country's naming operands. Suffixes carry their leading underscore.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Operands {
    pub tag: String,
    pub cosmetic: String,
    /// The overlord's canonical slot when the raw overlord link is positive.
    pub overlord: Option<u32>,
    overlord_suffix: String,
    subtype_suffix: String,
    group_suffix: String,
    autonomy_suffix: String,
}

impl Operands {
    /// Bytes the operands' strings hold, at their capacity.
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

    /// Static naming operands, without a leader, cosmetic tag or subject.
    pub fn for_tag(tag: &str, group: Option<&str>) -> Self {
        Self {
            tag: tag.to_owned(),
            group_suffix: group.map(|group| format!("_{group}")).unwrap_or_default(),
            ..Self::default()
        }
    }

    /// The same key chooser the recorded country name uses.
    pub fn name_key(&self, suffix: &str, exists: impl Fn(&str) -> bool) -> Option<String> {
        (!self.cosmetic.is_empty())
            .then(|| self.choose(&self.cosmetic, suffix, &exists))
            .flatten()
            .or_else(|| self.choose(&self.tag, suffix, exists))
    }

    /// Candidate key for an authored cosmetic base.
    pub fn candidate(
        &self,
        base: &str,
        suffix: &str,
        localisation: &Localisation,
    ) -> Option<String> {
        self.choose(base, suffix, |key| localisation.get(key).is_some())
    }

    /// Ordered candidate keys for `base`: subject rungs require an overlord
    /// or autonomy, and subtype rungs require a leader.
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

    /// The first existing key; the native retry changes the suffix case once,
    /// lower before upper.
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

/// Resource limits of the viewer, not game rules: all substitutions of one
/// displayed name share one budget.
const WORK_LIMIT: usize = 1024;
const BYTES_LIMIT: usize = 64 * 1024;
const DEPTH_LIMIT: usize = 64;

struct Budget {
    work: usize,
    bytes: usize,
}

impl Budget {
    fn new() -> Self {
        Self {
            work: WORK_LIMIT,
            bytes: BYTES_LIMIT,
        }
    }
    fn visit(&mut self) -> Result<(), String> {
        self.work = self
            .work
            .checked_sub(1)
            .ok_or("localisation expansion exceeds its budget")?;
        Ok(())
    }
    fn append(&mut self, out: &mut String, text: &str) -> Result<(), String> {
        self.bytes = self
            .bytes
            .checked_sub(text.len())
            .ok_or("localisation expansion exceeds its budget")?;
        out.push_str(text);
        Ok(())
    }
}

/// Country operands at one hour, resolved against one localisation.
pub struct CountryNames<'a> {
    localisation: &'a Localisation,
    operands: BTreeMap<u32, Result<Operands, String>>,
    reserved_built: Option<u8>,
}

impl<'a> CountryNames<'a> {
    /// Content-only names from explicit static operands, through the recorded
    /// name resolver; no recording-dependent operands are supplied.
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
    /// Reads the operands of every country in `frame`. A country whose inputs
    /// are missing this hour keeps the reason instead of a name.
    pub fn read(frame: &DecodedFrame, localisation: &'a Localisation) -> Self {
        let mut operands = operands(frame);
        operands.insert(0, reserved_zero(frame));
        Self {
            localisation,
            operands,
            reserved_built: reserved_zero_row(frame).ok().map(|row| row.built),
        }
    }

    /// Reads only the requested canonical slots and their overlords. Raw table
    /// keys are sorted; canonical aliases retain the last Countries row, as
    /// [`Self::read`] does. Name expansion keeps the same cycle and work limits.
    pub fn read_selected(
        frame: &DecodedFrame,
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
        if let (Ok(countries), Ok(links)) = (
            frame.complete::<tables::Countries>(),
            frame.complete::<tables::CountryLinks>(),
        ) {
            let by_slot = |slot: u32| {
                countries
                    .iter()
                    .rev()
                    .find(|row| row.canonical_slot == slot)
            };
            while let Some(slot) = pending.pop() {
                if operands.contains_key(&slot) {
                    continue;
                }
                let Some(country) = by_slot(slot) else {
                    continue;
                };
                let input = country_operands(frame, &by_slot, links, country);
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

    /// The operands of a canonical slot, for callers that cache names.
    pub fn operands(&self, slot: u32) -> Option<&Result<Operands, String>> {
        self.operands.get(&slot)
    }

    /// The display name of a canonical slot, or why it cannot be resolved.
    /// Callers show the tag with that reason. The bare tag is the ladder's
    /// last localisation candidate; when even that key is absent, `name_key`
    /// returns None and this documented display ends in the tag.
    pub fn name(&self, slot: u32) -> Result<String, String> {
        self.resolve(slot, "", &mut BTreeSet::new(), &mut Budget::new())
    }

    /// [`Self::name`] with a key suffix: `_DEF` for the definite name or
    /// `_ADJ` for the adjective, both built by the same key chooser.
    pub fn name_with(&self, slot: u32, suffix: &str) -> Result<String, String> {
        self.resolve(slot, suffix, &mut BTreeSet::new(), &mut Budget::new())
    }

    /// The ruling party's ideology group token (`Politics.ruling_party` ->
    /// `PoliticalParties.token`): `None` without a ruling party.
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

    fn resolve(
        &self,
        slot: u32,
        suffix: &str,
        active: &mut BTreeSet<(u32, String)>,
        budget: &mut Budget,
    ) -> Result<String, String> {
        // Slot zero may still hold constructor strings because name rebuilding
        // skips it. Witness each cache separately instead of rebuilding it early.
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
                return Ok(String::new());
            }
        }
        budget.visit()?;
        let identity = (slot, suffix.to_owned());
        if active.len() >= DEPTH_LIMIT || !active.insert(identity.clone()) {
            return Err("country names refer to each other in a cycle".into());
        }
        let result = (|| {
            let input = match self.operands.get(&slot) {
                Some(Ok(input)) => input,
                Some(Err(reason)) => return Err(reason.clone()),
                None => return Err(format!("country slot {slot} has no naming operands")),
            };
            let exists = |key: &str| self.localisation.get(key).is_some();
            let key = input
                .name_key(suffix, exists)
                .ok_or_else(|| format!("no localisation key names {}{suffix}", input.tag))?;
            self.expand(&key, input, active, &mut BTreeSet::new(), budget)
        })();
        active.remove(&identity);
        result
    }

    /// Localises `key`, expanding `$VARIABLE$` references. Subject variables
    /// apply only with a positive overlord.
    fn expand(
        &self,
        key: &str,
        input: &Operands,
        active: &mut BTreeSet<(u32, String)>,
        keys: &mut BTreeSet<String>,
        budget: &mut Budget,
    ) -> Result<String, String> {
        budget.visit()?;
        if keys.len() >= DEPTH_LIMIT || !keys.insert(key.to_owned()) {
            return Err(format!("localisation {key} refers to itself"));
        }
        let result = (|| {
            let text = self
                .localisation
                .get(key)
                .ok_or_else(|| format!("localisation key {key} is missing"))?;
            if text.contains('[') {
                return Err(format!("localisation {key} uses unsupported scripted text"));
            }
            let mut out = String::new();
            let mut rest = text;
            while let Some(start) = rest.find('$') {
                budget.append(&mut out, &rest[..start])?;
                rest = &rest[start + 1..];
                let end = rest
                    .find('$')
                    .ok_or_else(|| format!("localisation {key} has an unclosed $"))?;
                let variable = &rest[..end];
                let value = match (input.overlord, variable) {
                    (Some(overlord), "OVERLORDADJ") => {
                        self.resolve(overlord, "_ADJ", active, budget)?
                    }
                    (Some(overlord), "OVERLORDNAMEDEF") => {
                        self.resolve(overlord, "_DEF", active, budget)?
                    }
                    (Some(_), "NONIDEOLOGY") => {
                        self.expand(&input.tag, input, active, keys, budget)?
                    }
                    (Some(_), "NONIDEOLOGYADJ") => {
                        self.expand(&format!("{}_ADJ", input.tag), input, active, keys, budget)?
                    }
                    (Some(_), "NONIDEOLOGYDEF") => {
                        self.expand(&format!("{}_DEF", input.tag), input, active, keys, budget)?
                    }
                    _ => self.expand(variable, input, active, keys, budget)?,
                };
                budget.append(&mut out, &value)?;
                rest = &rest[end + 1..];
            }
            budget.append(&mut out, rest)?;
            Ok(out)
        })();
        keys.remove(key);
        result
    }
}

/// The rows a table holds this hour and whether that is all of them.
fn partial<T: TableSchema>(frame: &DecodedFrame) -> (&[T::Row], bool) {
    match &*frame.tables[T::ID.index()] {
        TableView::Complete(rows) => (rows.rows::<T>().unwrap_or(&[]), true),
        TableView::Truncated { rows, .. } => (rows.rows::<T>().unwrap_or(&[]), false),
        TableView::Unavailable { .. } => (&[], false),
    }
}

/// Flag file keys in lookup order: `<cosmetic>_<group>`, `<cosmetic>`, then
/// `<tag>_<group>`, `<tag>`, where the tag is the positive original reference,
/// else its own. A cosmetic tag without a flag file falls back to the tag's
/// group flag, and a country with a positive original never tries its own tag.
/// `selected` is the already resolved original-or-own tag; None withholds
/// only that unknown tail. A known empty selected tag is still a candidate.
/// An authenticated active group with an empty token still adds '_'.
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

fn reserved_zero_row(frame: &DecodedFrame) -> Result<&ReservedCountryNamingRow, String> {
    let rows = frame
        .complete::<tables::ReservedCountryNaming>()
        .map_err(|why| format!("ReservedCountryNaming is unavailable this hour: {why}"))?;
    match rows {
        [row] => Ok(row),
        _ => Err("ReservedCountryNaming has no singleton row for country zero".into()),
    }
}

/// Raw zero's tag comes from the reserved country database row.
pub fn reserved_zero_tag(frame: &DecodedFrame) -> Result<&str, String> {
    std::str::from_utf8(&reserved_zero_row(frame)?.tag)
        .map_err(|_| "reserved country tag is not UTF-8".into())
}

/// Resolve the reserved row's original-or-own tag from its recorded bytes.
/// The name generator's dynamic-only `name_tag` is a separate witness.
pub fn reserved_zero_original_tag(frame: &DecodedFrame) -> Result<&str, String> {
    let row = reserved_zero_row(frame)?;
    let reference = row
        .original_tag_reference
        .require("ReservedCountryNaming.original_tag_reference")?
        .ok_or("ReservedCountryNaming.original_tag_reference capture refused")?;
    let bytes = if reference > 0 {
        row.original_tag
            .require("ReservedCountryNaming.original_tag")?
            .as_ref()
            .ok_or("ReservedCountryNaming.original_tag capture refused")?
    } else {
        &row.tag
    };
    std::str::from_utf8(bytes).map_err(|_| "reserved original-or-own tag is not UTF-8".into())
}

/// Exact original-or-own tag bytes for a canonical country slot.
/// Zero has its own authenticated singleton, outside the positive census.
pub fn original_or_own_tag(frame: &DecodedFrame, slot: u32) -> Result<&str, String> {
    if slot == 0 {
        return reserved_zero_original_tag(frame);
    }
    let (country, row) = original_rows(frame, slot)?;
    original_tag(country, row)
}

/// Selected signed original-or-own tag word. This numeric getter does not
/// require the separately optional tag-string payload.
pub fn original_or_own_reference(frame: &DecodedFrame, slot: u32) -> Result<i32, String> {
    if slot == 0 {
        let raw = reserved_zero_row(frame)?
            .original_tag_reference
            .require("ReservedCountryNaming.original_tag_reference")?
            .ok_or("ReservedCountryNaming.original_tag_reference capture refused")?;
        return Ok(if raw > 0 { raw } else { 0 });
    }
    let (country, row) = original_rows(frame, slot)?;
    let raw = row
        .original_tag_reference
        .require("CountryAppearance.original_tag_reference")?
        .ok_or("CountryAppearance.original_tag_reference capture refused")?;
    if raw > 0 {
        Ok(raw)
    } else {
        Ok(country.id as i32)
    }
}

fn original_rows(
    frame: &DecodedFrame,
    slot: u32,
) -> Result<(&CountryRow, &CountryAppearanceRow), String> {
    let (countries, countries_complete) = partial::<tables::Countries>(frame);
    let country = countries
        .iter()
        .rev()
        .find(|country| country.canonical_slot == slot)
        .ok_or_else(|| {
            if countries_complete {
                format!("country slot {slot} has no Countries row")
            } else {
                format!("Countries is incomplete this hour: country slot {slot} is unknown")
            }
        })?;
    let (appearance, appearance_complete) = partial::<tables::CountryAppearance>(frame);
    let row = last_by_key(appearance, country.id, |row| row.country).ok_or_else(|| {
        if appearance_complete {
            format!("country slot {slot} has no CountryAppearance row")
        } else {
            format!("CountryAppearance is incomplete this hour: country slot {slot} is unknown")
        }
    })?;
    Ok((country, row))
}

/// Slot zero's naming operands from its dedicated singleton. These operands
/// never admit zero into Countries, CountryLinks or CountryAppearance.
pub fn reserved_zero(frame: &DecodedFrame) -> Result<Operands, String> {
    let row = reserved_zero_row(frame)?;
    let tag = if row.name_tag.is_empty() {
        &row.tag
    } else {
        &row.name_tag
    };
    let overlord = if row.overlord > 0 {
        let links = frame
            .complete::<tables::CountryLinks>()
            .map_err(|why| format!("CountryLinks is unavailable this hour: {why}"))?;
        Some(
            last_by_key(links, row.overlord as u32, |row| row.id)
                .ok_or_else(|| format!("overlord link {} has no country", row.overlord))?
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
                .complete::<tables::Countries>()
                .map_err(|why| format!("Countries is unavailable this hour: {why}"))?;
            let overlord = countries
                .iter()
                .rev()
                .find(|country| country.canonical_slot == slot)
                .ok_or_else(|| format!("overlord slot {slot} has no country"))?;
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

/// The dynamic-gated tag used by the name generator, independent of the
/// unconditional positive-original selection used for flags.
fn naming_tag<'a>(
    country: &'a CountryRow,
    appearance: &'a CountryAppearanceRow,
) -> Result<&'a str, String> {
    let (bytes, what) = if appearance.name_tag.is_empty() {
        (&country.tag, "country tag")
    } else {
        (&appearance.name_tag, "original tag")
    };
    std::str::from_utf8(bytes).map_err(|_| format!("{what} is not UTF-8"))
}

/// The original-or-own tag, shared with the collector's status summary.
pub use replay_data::country_flags::original_tag;

/// Per canonical slot: the operands, or why they are unavailable this hour.
fn operands(frame: &DecodedFrame) -> BTreeMap<u32, Result<Operands, String>> {
    let (Ok(countries), Ok(links)) = (
        frame.complete::<tables::Countries>(),
        frame.complete::<tables::CountryLinks>(),
    ) else {
        return BTreeMap::new();
    };
    // The last row of a slot wins, as a reverse search would find it.
    let slots: BTreeMap<u32, &CountryRow> = countries
        .iter()
        .map(|row| (row.canonical_slot, row))
        .collect();
    let by_slot = |slot: u32| slots.get(&slot).copied();
    countries
        .iter()
        .map(|country| {
            (
                country.canonical_slot,
                country_operands(frame, &by_slot, links, country),
            )
        })
        .collect()
}

/// Last matching key, preserving the maps' overwrite rule without indexing a
/// table. Decoded rows ascend by their raw table key.
fn last_by_key<T, K: Ord>(rows: &[T], wanted: K, key: impl Fn(&T) -> K) -> Option<&T> {
    let end = rows.partition_point(|row| key(row) <= wanted);
    rows.get(end.checked_sub(1)?)
        .filter(|row| key(row) == wanted)
}

fn country_operands<'r>(
    frame: &DecodedFrame,
    by_slot: &dyn Fn(u32) -> Option<&'r CountryRow>,
    links: &[CountryLinkRow],
    country: &CountryRow,
) -> Result<Operands, String> {
    let table =
        |name: &str, complete: bool| (!complete).then(|| format!("{name} is incomplete this hour"));
    let (appearance, appearance_complete) = partial::<tables::CountryAppearance>(frame);
    let (politics, politics_complete) = partial::<tables::Politics>(frame);
    let (parties, parties_complete) = partial::<tables::PoliticalParties>(frame);
    let (autonomy, autonomy_complete) = partial::<tables::Autonomy>(frame);
    let row = last_by_key(appearance, country.id, |row| row.country).ok_or_else(|| {
        table("CountryAppearance", appearance_complete)
            .unwrap_or_else(|| "no naming operands recorded".into())
    })?;
    let tag = naming_tag(country, row)?.to_owned();
    let group = match last_by_key(politics, country.id, |row| row.country) {
        Some(row) => match row.ruling_party {
            Some(party) => {
                match last_by_key(parties, (country.id, party), |row| (row.country, row.id)) {
                    Some(row) => text(&row.token, "ideology group")?,
                    None => {
                        return Err(table("PoliticalParties", parties_complete)
                            .unwrap_or_else(|| "ruling party has no recorded row".into()));
                    }
                }
            }
            None => String::new(),
        },
        None => {
            return Err(table("Politics", politics_complete)
                .unwrap_or_else(|| "no politics row recorded".into()));
        }
    };
    let overlord = if row.overlord > 0 {
        Some(
            last_by_key(links, row.overlord as u32, |row| row.id)
                .ok_or_else(|| format!("overlord link {} has no country", row.overlord))?
                .canonical_slot,
        )
    } else {
        None
    };
    // Subject rungs require both a current autonomy status and a positive
    // overlord.
    let state = last_by_key(autonomy, country.id, |row| row.subject);
    let (overlord_suffix, autonomy_suffix) = match (overlord, state) {
        (Some(slot), Some(state)) => {
            if state.state.is_empty() {
                return Err("current autonomy has no definition".into());
            }
            let overlord =
                by_slot(slot).ok_or_else(|| format!("overlord slot {slot} has no country"))?;
            (
                format!("_{}", text(&overlord.tag, "overlord tag")?),
                format!("_{}", text(&state.state, "autonomy")?),
            )
        }
        (Some(_), None) if !autonomy_complete => {
            return Err("Autonomy is incomplete this hour".into());
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
        // A cosmetic tag with no flag of its own falls back to the tag's group
        // flag before its plain flag.
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
        // A selected positive original never tries the own tag.
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
            names
                .resolve(2, "_DEF", &mut BTreeSet::new(), &mut Budget::new())
                .as_deref(),
            Ok("India of the United Kingdom")
        );
    }

    #[test]
    fn scripted_cyclic_and_exponential_text_is_refused() {
        let scripted = localisation(&[("TAG", "[ROOT.GetName]")]);
        assert!(
            names(&scripted, vec![(1, input("TAG"))])
                .name(1)
                .unwrap_err()
                .contains("scripted")
        );
        let cyclic = localisation(&[("TAG", "$ALIAS$"), ("ALIAS", "$TAG$")]);
        assert!(names(&cyclic, vec![(1, input("TAG"))]).name(1).is_err());
        let mut doubling: Vec<(String, String)> = (0..30)
            .map(|i| (format!("A{i}"), format!("$A{}$$A{}$", i + 1, i + 1)))
            .collect();
        doubling.push(("A30".into(), String::new()));
        let doubling: Localisation = doubling.into_iter().collect();
        assert!(names(&doubling, vec![(1, input("A0"))]).name(1).is_err());
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
            Err("Politics is incomplete this hour".into())
        );
        let selected = CountryNames::read_selected(&frame, &localisation, [30]);
        assert_eq!(
            selected.name(30),
            Err("Politics is incomplete this hour".into())
        );
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
                (30, "D01", "CountryAppearance"),
                (0, "ZZ0", "ReservedCountryNaming"),
            ] {
                let result = original_or_own_tag(&frame, slot);
                match expected {
                    Ok("OWN") => assert_eq!(result, Ok(own)),
                    Ok(text) => assert_eq!(result, Ok(text)),
                    Err(why) => {
                        let field = if why.starts_with("reference") {
                            "original_tag_reference"
                        } else {
                            "original_tag"
                        };
                        let suffix = if why.ends_with("absent") {
                            "not recorded"
                        } else {
                            "capture refused"
                        };
                        assert_eq!(result, Err(format!("{prefix}.{field} {suffix}")));
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
        // regression: a39bd906e4 Fix counter hover review gaps and country lookups
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
}

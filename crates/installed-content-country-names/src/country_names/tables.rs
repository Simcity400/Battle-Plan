//! The naming operands read from any frame's tables ([`Tables`]), a walk's views among them:
//! each row built of the fields the names read alone, found by its key.

use super::{CountryNames, NamingAppearance, NamingRows, Operands, naming_operands, text, underscore};
use crate::localisation::Localisation;
use replay_data::rows::{
    AutonomyRow, CountryAppearanceRow, CountryLinkRow, CountryRow, PoliticalPartyRow, PoliticsRow,
    ReservedCountryNamingRow, TableSchema, mask, tables,
};
use replay_frame::frame::{Rowset, Tables};
use std::{borrow::Cow, collections::BTreeMap};

const SLOT: u64 = mask::<CountryRow>(&["canonical_slot"]);
const COUNTRY: u64 = mask::<CountryRow>(&["canonical_slot", "tag"]);
const LINK: u64 = mask::<CountryLinkRow>(&["canonical_slot"]);
const APPEARANCE: u64 = mask::<CountryAppearanceRow>(&[
    "cosmetic_tag",
    "name_tag",
    "leader_ideology",
    "overlord",
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
]);

const ORIGINAL: u64 = mask::<CountryAppearanceRow>(&["original_tag", "original_tag_reference"]);
const RESERVED_ORIGINAL: u64 =
    mask::<ReservedCountryNamingRow>(&["tag", "original_tag", "original_tag_reference"]);

/// [`super::original_or_own_tag`] of any frame's tables, a walk's views among them.
pub fn original_or_own_tag<F: Tables + ?Sized>(frame: &F, slot: u32) -> Result<String, String> {
    if slot == 0 {
        let rows = frame
            .complete_rows::<tables::ReservedCountryNaming>(RESERVED_ORIGINAL)
            .map_err(|why| format!("reserved country naming is unavailable this hour: {why}"))?;
        if rows.len() != 1 {
            return Err("reserved country naming has no singleton row for country zero".into());
        }
        let row = rows
            .iter()
            .next()
            .ok_or("reserved country naming has no singleton row for country zero")?
            .map_err(|why| why.to_string())?;
        let reference = row
            .original_tag_reference
            .require("original tag reference of reserved country naming")?
            .ok_or("original tag reference of reserved country naming capture refused")?;
        let bytes = if reference > 0 {
            row.original_tag
                .require("original tag of reserved country naming")?
                .as_ref()
                .ok_or("original tag of reserved country naming capture refused")?
        } else {
            &row.tag
        };
        return std::str::from_utf8(bytes)
            .map(str::to_owned)
            .map_err(|_| "reserved original-or-own tag is not UTF-8".into());
    }
    let (countries, countries_complete) = partial::<tables::Countries, F>(frame, COUNTRY);
    let country = match countries {
        Some(countries) => by_slot(countries, slot)?,
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
        partial::<tables::CountryAppearance, F>(frame, ORIGINAL);
    let row = get(appearance, u128::from(country.id))?.ok_or_else(|| {
        if appearance_complete {
            format!("country slot {slot} has no country appearance row")
        } else {
            "country appearance is incomplete this hour: a country is unknown".to_owned()
        }
    })?;
    super::original_tag(&country, &row).map(str::to_owned)
}

/// `T`'s rows the hour holds, a truncated table's read ones among them, and whether it is
/// complete, as [`super::partial`] reads them.
fn partial<T: TableSchema, F: Tables + ?Sized>(
    frame: &F,
    fields: u64,
) -> (Option<Rowset<'_, T::Row>>, bool) {
    (
        frame.rows::<T>(fields),
        frame.table_status(T::ID).is_table_complete(),
    )
}

/// The row of `rows` keyed `key`; a row that cannot be built refuses as its error says.
fn get<'f, R: replay_data::rows::RowCodec>(
    rows: Option<Rowset<'f, R>>,
    key: u128,
) -> Result<Option<Cow<'f, R>>, String> {
    match rows {
        Some(rows) => rows.get(key).map_err(|why| why.to_string()),
        None => Ok(None),
    }
}

/// The last Countries row holding canonical `slot`, as a reverse search finds it.
fn by_slot<'f>(
    countries: Rowset<'f, CountryRow>,
    slot: u32,
) -> Result<Option<Cow<'f, CountryRow>>, String> {
    let mut found = None;
    for row in countries.with_mask(SLOT).iter() {
        let row = row.map_err(|why| why.to_string())?;
        if row.canonical_slot == slot {
            found = Some(row.id);
        }
    }
    match found {
        Some(id) => countries
            .get(u128::from(id))
            .map_err(|why| why.to_string()),
        None => Ok(None),
    }
}

fn reserved_row<F: Tables + ?Sized>(frame: &F) -> Result<Cow<'_, ReservedCountryNamingRow>, String> {
    let rows = frame
        .complete_rows::<tables::ReservedCountryNaming>(RESERVED)
        .map_err(|why| format!("reserved country naming is unavailable this hour: {why}"))?;
    if rows.len() != 1 {
        return Err("reserved country naming has no singleton row for country zero".into());
    }
    rows.iter()
        .next()
        .ok_or("reserved country naming has no singleton row for country zero")?
        .map_err(|why| why.to_string())
}

fn links<F: Tables + ?Sized>(frame: &F) -> Result<Rowset<'_, CountryLinkRow>, String> {
    frame
        .complete_rows::<tables::CountryLinks>(LINK)
        .map_err(|why| format!("country links is unavailable this hour: {why}"))
}

/// [`super::reserved_zero`] of any frame's tables.
fn reserved_zero<F: Tables + ?Sized>(frame: &F) -> Result<Operands, String> {
    let row = reserved_row(frame)?;
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
            get(Some(links), u128::from(row.overlord as u32))?
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
            let overlord = by_slot(countries, slot)?
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

/// [`super::country_operands`] of any frame's tables.
fn country_operands<F: Tables + ?Sized>(
    frame: &F,
    countries: Rowset<'_, CountryRow>,
    links: &Result<Rowset<'_, CountryLinkRow>, String>,
    country: &CountryRow,
) -> Result<Operands, String> {
    let rows = FrameRows {
        countries,
        links,
        appearance: partial::<tables::CountryAppearance, F>(frame, APPEARANCE),
        politics: partial::<tables::Politics, F>(frame, POLITICS),
        parties: partial::<tables::PoliticalParties, F>(frame, PARTY),
        autonomy: partial::<tables::Autonomy, F>(frame, AUTONOMY),
    };
    naming_operands(&rows, country.id, &country.tag)
}

/// The naming tables of any frame, each with whether it is wholly recorded.
struct FrameRows<'f, 'l> {
    countries: Rowset<'f, CountryRow>,
    links: &'l Result<Rowset<'f, CountryLinkRow>, String>,
    appearance: (Option<Rowset<'f, CountryAppearanceRow>>, bool),
    politics: (Option<Rowset<'f, PoliticsRow>>, bool),
    parties: (Option<Rowset<'f, PoliticalPartyRow>>, bool),
    autonomy: (Option<Rowset<'f, AutonomyRow>>, bool),
}

impl NamingRows for FrameRows<'_, '_> {
    fn appearance(&self, id: u32) -> Result<Option<NamingAppearance>, String> {
        Ok(get(self.appearance.0, u128::from(id))?.map(|row| NamingAppearance::of(&row)))
    }

    fn ruling_party(&self, id: u32) -> Result<Option<Option<u32>>, String> {
        Ok(get(self.politics.0, u128::from(id))?.map(|row| row.ruling_party))
    }

    fn party_token(&self, id: u32, party: u32) -> Result<Option<Vec<u8>>, String> {
        Ok(get(self.parties.0, u128::from(id) << 32 | u128::from(party))?.map(|row| row.token.clone()))
    }

    fn link(&self, raw: u32) -> Result<Option<u32>, String> {
        let links = *self.links.as_ref().map_err(String::clone)?;
        Ok(get(Some(links), u128::from(raw))?.map(|row| row.canonical_slot))
    }

    fn autonomy(&self, id: u32) -> Result<Option<Vec<u8>>, String> {
        Ok(get(self.autonomy.0, u128::from(id))?.map(|row| row.state.clone()))
    }

    fn slot_tag(&self, slot: u32) -> Result<Option<Vec<u8>>, String> {
        Ok(by_slot(self.countries, slot)?.map(|country| country.tag.clone()))
    }

    fn complete(&self, table: replay_data::rows::TableId) -> bool {
        use replay_data::rows::TableId;
        match table {
            TableId::CountryAppearance => self.appearance.1,
            TableId::Politics => self.politics.1,
            TableId::PoliticalParties => self.parties.1,
            TableId::Autonomy => self.autonomy.1,
            _ => true,
        }
    }
}

impl<'a> CountryNames<'a> {
    /// [`Self::read_selected`] of any frame's tables, a walk's views among them: only the rows
    /// the requested slots and their overlords reach are built.
    pub fn read_selected_tables<F: Tables + ?Sized>(
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
            reserved_row(frame).ok().map(|row| row.built)
        } else {
            None
        };
        if let Ok(countries) = frame.complete_rows::<tables::Countries>(COUNTRY) {
            let links = links(frame);
            while let Some(slot) = pending.pop() {
                if operands.contains_key(&slot) {
                    continue;
                }
                let country = match by_slot(countries, slot) {
                    Ok(Some(country)) => country,
                    Ok(None) => continue,
                    Err(why) => {
                        operands.insert(slot, Err(why));
                        continue;
                    }
                };
                let input = country_operands(frame, countries, &links, &country);
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
}

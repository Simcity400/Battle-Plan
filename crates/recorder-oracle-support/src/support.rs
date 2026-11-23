use super::report::Report;
use content_reader_save::{ActualCountry, CountryEmissionIdentity, Save, SaveDocument};
use replay_data::{
    cadence, cadence::Cadence, rows::CountryRow, rows::Recorded, rows::TableId, rows::TableRows,
    rows::TableSchema, rows::tables,
};
use replay_frame::{frame::DecodedFrame, frame::TableView};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

pub type TokenNames<'a> = BTreeMap<u32, &'a [u8]>;

pub type Spellings<'a> = BTreeMap<content_reader_parser::tokens::Token, Option<&'a [u8]>>;

pub type ContentRead<'a, T> = Result<&'a T, String>;

pub fn serialized_text<'a>(report: &mut Report, field: &str, bytes: &'a [u8]) -> &'a [u8] {
    let text = content_reader_parser::parser::c_string(bytes);
    if text.len() < bytes.len() {
        report.limitation("native text serialization ends at the first NUL; captured trailing bytes are not save-comparable", field);
    }
    text
}

pub fn reverse_spellings<'a>(tokens: &TokenNames<'a>) -> Spellings<'a> {
    let mut spellings = BTreeMap::new();
    for literal in tokens.values() {
        let literal = content_reader_parser::parser::c_string(literal);
        spellings
            .entry(content_reader_parser::tokens::Token::c_str(literal))
            .and_modify(|spelling| {
                if *spelling != Some(literal) {
                    *spelling = None;
                }
            })
            .or_insert(Some(literal));
    }
    spellings
}

pub fn token_spelling<'a>(
    tokens: &TokenNames<'a>,
    spellings: &Spellings<'a>,
    name: &[u8],
    subject: &str,
) -> io::Result<&'a [u8]> {
    let identity = content_reader_parser::tokens::Token::c_str(name);
    let spelling = if identity.is_rejected() {
        tokens
            .get(&0)
            .copied()
            .map(content_reader_parser::parser::c_string)
    } else {
        spellings.get(&identity).copied().ok_or_else(|| {
            invalid(format!(
                "{subject} {:?} is absent from the recorded lexical dictionary",
                String::from_utf8_lossy(name),
            ))
        })?
    };
    spelling.ok_or_else(|| {
        invalid(format!(
            "{subject} {:?} has missing or ambiguous recorded lexical spelling",
            String::from_utf8_lossy(name),
        ))
    })
}

pub fn content_read<T>(result: &io::Result<T>) -> ContentRead<'_, T> {
    result.as_ref().map_err(ToString::to_string)
}

pub fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

pub fn complete<T: TableSchema>(frame: &DecodedFrame) -> io::Result<&[T::Row]> {
    frame.complete::<T>()
}

pub fn sampled_states(frame: &DecodedFrame) -> io::Result<impl Fn(u32) -> bool + '_> {
    let reads = complete::<tables::SliceReads>(frame)?;
    let hour = frame.hour;
    Ok(move |state| {
        replay_data::sliced::read_hour(reads, TableId::StateResourceInputs, state) == Some(hour)
    })
}

pub fn current_slice_owners(
    frame: &DecodedFrame,
    table: TableId,
    report: &Report,
) -> io::Result<Option<BTreeSet<u32>>> {
    if report.full_refresh() {
        return Ok(None);
    }
    let id = table.index() as u32;
    let mut dated = false;
    let mut current = BTreeSet::new();
    for read in complete::<tables::SliceReads>(frame)?
        .iter()
        .filter(|read| read.table == id)
    {
        dated = true;
        if read.sample_hour == frame.hour {
            current.insert(read.owner);
        }
    }
    Ok(dated.then_some(current))
}

pub fn slice_owner_limitation(
    frame: &DecodedFrame,
    table: TableId,
    current: &Option<BTreeSet<u32>>,
    report: &mut Report,
) {
    let Some(current) = current else {
        return;
    };
    let oldest = complete::<tables::SliceReads>(frame)
        .ok()
        .and_then(|reads| {
            reads
                .iter()
                .filter(|row| row.table == table.index() as u32 && row.sample_hour != frame.hour)
                .map(|row| row.sample_hour)
                .min()
        });
    if oldest.is_some() {
        report.carried_owners(table, oldest);
    }
    report.limitation(
        "panel-only tables are read once a game day, one owner slice per hour (owner decision 2026-10-11); only the owners read at the oracle hour are compared with the save, and the others hold earlier reads (their hours are in SliceReads)",
        format!("{table:?}: {} owners read at the oracle hour", current.len()),
    );
}

pub fn complete_table(frame: &DecodedFrame, table: TableId) -> io::Result<&TableRows> {
    match &*frame.tables[table.index()] {
        TableView::Complete(rows) if rows.table() == table => Ok(rows),
        TableView::Complete(_) => Err(invalid(format!("wrong table rows in {table:?}"))),
        TableView::Truncated { missing, .. } => Err(invalid(format!(
            "hour {} {table:?} is truncated ({missing} missing rows)",
            frame.hour
        ))),
        TableView::Unavailable { reason } => Err(invalid(format!(
            "hour {} {table:?} is unavailable: {reason}",
            frame.hour
        ))),
        TableView::NotRecorded { mode } => Err(invalid(format!(
            "hour {} {table:?} is not recorded in a {mode} recording",
            frame.hour
        ))),
    }
}

pub fn lexical_tokens<'a>(
    frame: &'a DecodedFrame,
    report: &mut Report,
) -> io::Result<TokenNames<'a>> {
    if let TableView::Unavailable { reason } = &*frame.tables[TableId::LexicalTokens.index()] {
        report.limitation(
            "recorded lexical dictionary is unavailable; text-to-token comparisons name their missing coverage and do not infer token identities from game content",
            reason.clone(),
        );
        return Ok(BTreeMap::new());
    }
    let mut tokens = BTreeMap::new();
    for row in complete::<tables::LexicalTokens>(frame)? {
        if tokens.insert(row.token, row.literal.as_slice()).is_some() {
            return Err(invalid("duplicate lexical token id"));
        }
    }
    Ok(tokens)
}

pub fn country_context(frame: &DecodedFrame) -> io::Result<CountryContext<'_>> {
    let mut by_tag = BTreeMap::new();
    let mut by_slot = BTreeMap::new();
    for country in frame.complete::<tables::Countries>()? {
        if by_tag.insert(country.tag.as_slice(), country).is_some()
            || by_slot.insert(country.canonical_slot, country).is_some()
        {
            return Err(invalid("ambiguous country tag or canonical slot"));
        }
    }
    let mut links = BTreeMap::new();
    for link in frame.complete::<tables::CountryLinks>()? {
        let country = by_slot
            .get(&link.canonical_slot)
            .ok_or_else(|| invalid(format!("country link {} has no canonical country", link.id)))?;
        if links.insert(link.id, *country).is_some() {
            return Err(invalid("duplicate country link"));
        }
    }
    for (&tag, captured) in &by_tag {
        if links
            .get(&captured.id)
            .map(|country| country.canonical_slot)
            != Some(captured.canonical_slot)
        {
            return Err(invalid(format!(
                "country {} has an inconsistent identity link",
                String::from_utf8_lossy(tag)
            )));
        }
    }
    Ok(CountryContext { by_tag, links })
}

pub fn required_context<'a, T, E: std::fmt::Display>(
    name: &str,
    context: &'a Result<T, E>,
) -> io::Result<&'a T> {
    context
        .as_ref()
        .map_err(|error| invalid(format!("requires {name}: {error}")))
}

pub fn required_country_tag<'a>(
    reference: u32,
    links: &BTreeMap<u32, &'a CountryRow>,
) -> io::Result<&'a [u8]> {
    country_tag(reference, links)?.ok_or_else(|| invalid("null country reference"))
}

pub fn country_tag<'a>(
    reference: u32,
    links: &BTreeMap<u32, &'a CountryRow>,
) -> io::Result<Option<&'a [u8]>> {
    if reference == 0 {
        return Ok(None);
    }
    links
        .get(&reference)
        .map(|country| Some(country.tag.as_slice()))
        .ok_or_else(|| invalid(format!("unresolved country reference {reference}")))
}

pub fn tag_text(tag: Option<&[u8]>) -> String {
    tag.map(|tag| String::from_utf8_lossy(tag).into_owned())
        .unwrap_or_else(|| "<none>".into())
}

pub struct CountryContext<'a> {
    pub by_tag: BTreeMap<&'a [u8], &'a CountryRow>,
    pub links: BTreeMap<u32, &'a CountryRow>,
}

pub const CONVOY_LIMIT: i64 = 0x53e2_d620_fb40;

pub fn convoy_total(
    country: &content_reader_save::Country,
    save: &Save,
    equipment: &content_reader_key_names::equipment::Database,
) -> Result<i64, String> {
    let pool = country
        .convoy_equipment
        .as_ref()
        .ok_or("save lacks the convoy equipment pool")?;
    let mut total = 0;
    for entry in pool {
        let definition = save
            .equipment_definitions
            .get(&(entry.id, entry.id_type))
            .ok_or_else(|| {
                format!(
                    "save lacks equipment definition ({}, {})",
                    entry.id, entry.id_type
                )
            })?;
        if convoy_variant(equipment, definition)? {
            total = convoy_add(total, entry.amount.raw);
        }
    }
    Ok(total)
}

pub fn convoy_variant(
    equipment: &content_reader_key_names::equipment::Database,
    definition: &[u8],
) -> Result<bool, String> {
    let name = || String::from_utf8_lossy(definition).into_owned();
    let kind = equipment
        .named(definition)
        .ok_or_else(|| format!("verified content has no equipment {}", name()))?;
    let allowed = kind
        .archetype
        .map_or(0, |archetype| equipment.equipment[archetype].allowed_types);
    let mut flags = equipment.primary_category(kind);
    if allowed != 0 {
        flags &= allowed;
    }
    if kind.module_slots.is_empty() {
        return Ok(content_reader_key_names::equipment::normalize_category(flags) & 1 != 0);
    }
    if flags & 1 == 0 && !equipment.modules_add(1) {
        return Ok(false);
    }
    Err(format!(
        "equipment {} has module slots, and whether its variants are convoys depends on their installed modules, which the save comparison does not read",
        name()
    ))
}

pub fn convoy_add(total: i64, amount: i64) -> i64 {
    (i128::from(total) + i128::from(amount))
        .clamp(-i128::from(CONVOY_LIMIT), i128::from(CONVOY_LIMIT)) as i64
}

pub fn equal<T: std::fmt::Debug + PartialEq>(
    report: &mut Report,
    label: impl AsRef<str>,
    actual: T,
    expected: T,
) {
    if actual != expected {
        report.difference(label, actual, expected);
    }
}

pub fn compare_text(report: &mut Report, label: &str, actual: &[u8], saved: &[u8]) {
    let end = content_reader_parser::parser::c_string(actual).len();
    equal(report, label, &actual[..end], saved);
    if end < actual.len() {
        report.limitation(
            "native text string serialization stops at the first NUL; only the string prefix is comparable and the captured NUL/suffix remain uncheckable",
            label,
        );
    }
}

pub fn eq<T: PartialEq + std::fmt::Debug>(r: &mut Report, label: &str, actual: T, expected: T) {
    r.scalar_checks += 1;
    if actual != expected {
        r.difference(label, actual, expected);
    }
}

pub fn country_items<'a>(
    document: &'a SaveDocument,
    frame: &DecodedFrame,
) -> io::Result<Vec<&'a content_reader_parser::parser::Item>> {
    match country_emission_layout(frame) {
        Ok(Some(layout)) => document.representative_country_items(&layout),
        Ok(None) if document.has_repeated_country_tags()? => Err(invalid(
            "full repeated-country save requires its physical census",
        )),
        Ok(None) => Ok(document.original_country_items()?.iter().collect()),
        Err(reason) if document.has_repeated_country_tags()? => Err(invalid(format!(
            "full repeated-country save requires its physical census: {reason}"
        ))),
        Err(_) => Ok(document.original_country_items()?.iter().collect()),
    }
}

pub fn country_emission_layout(
    frame: &DecodedFrame,
) -> io::Result<Option<Vec<CountryEmissionIdentity>>> {
    if cadence::cadence(TableId::ReservedCountryNaming) != Cadence::Hourly {
        return Err(invalid(
            "physical country emission census is not a current hourly table",
        ));
    }
    let rows = frame.complete::<tables::ReservedCountryNaming>()?;
    let [reserved] = rows else {
        return Err(invalid(
            "physical country emission census requires one reserved row",
        ));
    };
    let array = match &reserved.country_array {
        Recorded::NotRecorded => return Ok(None),
        Recorded::Value(None) => return Err(invalid("physical country emission census refused")),
        Recorded::Value(Some(array)) => array,
    };
    if array.first() != Some(&0) {
        return Err(invalid(
            "physical country emission census lacks its authenticated reserved entry",
        ));
    }
    let countries = country_context(frame)?;
    for country in countries.by_tag.values() {
        if array.get(country.canonical_slot as usize) != Some(&country.id) {
            return Err(invalid(
                "physical country census differs from its canonical Countries roster",
            ));
        }
    }
    let mut result = Vec::with_capacity(array.len().saturating_sub(1));
    for (physical_slot, &own_id) in array.iter().enumerate().skip(1) {
        let (actual, tag) = if own_id == 0 {
            (ActualCountry::Reserved, reserved.tag.as_slice())
        } else {
            let country = countries.links.get(&own_id).ok_or_else(|| {
                invalid("physical country emission has an unresolved positive own ID")
            })?;
            if country.id != own_id || country.canonical_slot == 0 {
                return Err(invalid(
                    "physical country emission own ID differs from its canonical object",
                ));
            }
            (
                ActualCountry::Canonical(country.canonical_slot),
                country.tag.as_slice(),
            )
        };
        result.push(CountryEmissionIdentity {
            physical_slot: u32::try_from(physical_slot)
                .map_err(|_| invalid("physical country emission census exceeds u32"))?,
            own_id,
            actual,
            tag: tag.to_vec(),
        });
    }
    Ok(Some(result))
}

pub fn parse_country_emissions(document: &SaveDocument, frame: &DecodedFrame) -> io::Result<Save> {
    match country_emission_layout(frame) {
        Ok(Some(layout)) => document.project(&layout)?.parse(),
        Ok(None) => document.parse(),
        Err(reason) if document.has_repeated_country_tags()? => Err(invalid(format!(
            "full repeated-country save requires its physical census: {reason}"
        ))),
        Err(_) => document.parse(),
    }
}

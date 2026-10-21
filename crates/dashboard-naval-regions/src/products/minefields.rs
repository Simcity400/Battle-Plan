//! Names: `OWNER|<regionId>` is a nonzero signed fixed5 mine amount;
//! `OWNER|extent` is the owner's complete vector extent; `OWNER|refused` marks
//! an owner whose mine vector was read but not admitted at that hour, so the
//! inventory is incomplete rather than empty; `?|refused` counts owners the
//! hour holds vectors for but cannot name. OWNER is TAG@slot.
//! No amount below a witnessed extent proves zero; no extent proves nothing.
//! Standing reports retain occurrences and never infer strike dates.
use crate::shared::{self, Choice};
use dashboard_base::{
    catalogue::{self, Refusals},
    prepared::{prepass::Family, query::progressive},
    products::{Product, ReplyWarnings},
};
use dashboard_keyed::{
    Observer, Values,
    countries::Countries,
    query::{KeyedWindow, Query},
};
use replay::container::TableFields;
use replay_data::{
    format::FormatVersion,
    rows::{NavalAccidentReportRow, NavalRegionalMineRow, TableId, tables},
};
use replay_derive::naval_mine_levels::Levels;
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};
use ts_rs::TS;

pub struct Minefields;
#[derive(Clone, Debug, Default, Deserialize, Serialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(rename = "MinefieldsView")]
pub enum View {
    #[default]
    History,
    Reports,
}
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(rename = "MinefieldsParams")]
pub struct Params {
    pub branch_id: String,
    #[ts(optional)]
    pub tags: Option<Vec<String>>,
    #[ts(optional)]
    pub first_hour: Option<i32>,
    #[ts(optional)]
    pub last_hour: Option<i32>,
    #[serde(default)]
    pub view: View,
    #[ts(optional)]
    pub region: Option<u32>,
    #[ts(optional)]
    pub hour: Option<i32>,
}
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "MinefieldsHistoryReply")]
pub struct HistoryReply {
    pub window: KeyedWindow,
    pub regions: Vec<Choice>,
    pub scale: i32,
    pub warnings: Vec<String>,
}
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "MinefieldReport")]
pub struct Report {
    pub kind: u8,
    pub ordinal: u32,
    pub victim: Option<String>,
    pub mine_owner: Option<String>,
    pub ship: Option<String>,
    pub equipment: Option<String>,
    pub damage: String,
    pub sunk: bool,
    pub first_seen: Option<i32>,
}
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "MinefieldsReportsReply")]
pub struct ReportsReply {
    pub hour: i32,
    pub read_hour: Option<i32>,
    pub region: u32,
    pub reports: Vec<Report>,
    pub warnings: Vec<String>,
}
#[derive(Clone, Debug, Serialize, TS)]
#[serde(untagged)]
#[ts(rename = "MinefieldsReply")]
pub enum Reply {
    History(HistoryReply),
    Reports(ReportsReply),
}
impl ReplyWarnings for Reply {
    fn warnings_mut(&mut self) -> &mut Vec<String> {
        match self {
            Self::History(r) => &mut r.warnings,
            Self::Reports(r) => &mut r.warnings,
        }
    }
}
impl Product for Minefields {
    const KEY: &'static str = "minefields";
    const CONTENT_DEPENDENT: bool = true;
    type Params = Params;
    type Reply = Reply;
    fn validate(p: &Params) -> Result<(), &'static str> {
        shared::validate(&p.branch_id, p.first_hour, p.last_hour)?;
        for tag in p.tags.iter().flatten() {
            shared::tag(tag)?;
        }
        if matches!(p.view, View::Reports) && (p.hour.is_none() || p.region.is_none()) {
            return Err("Reports require a region and hour");
        }
        Ok(())
    }
    fn types() -> Vec<String> {
        let mut t = vec![
            Params::decl(),
            Reply::decl(),
            View::decl(),
            HistoryReply::decl(),
            ReportsReply::decl(),
            Report::decl(),
        ];
        t.extend(shared::types());
        t
    }
}
#[derive(Default, Serialize, Deserialize)]
pub struct MinefieldsObserver {
    pub refusals: BTreeMap<String, u64>,
}
pub static FAMILY: Family = dashboard_keyed::block::family::<MinefieldsObserver>(
    "minefields",
    "minefields.extract",
    "minefields.sequential",
    "minefields.fallback",
);
impl Observer for MinefieldsObserver {
    const NAME: &'static str = "minefields";
    const TABLES: &'static [TableId] = &[
        TableId::NavalRegionalMines,
        TableId::CountryLinks,
        TableId::Countries,
    ];
    const READ: TableId = TableId::NavalRegionalMines;
    fn family() -> &'static Family {
        &FAMILY
    }
    fn fields() -> io::Result<Vec<TableFields>> {
        Ok(vec![
            TableFields::all(TableId::NavalRegionalMines),
            TableFields::all(TableId::CountryLinks),
            TableFields::named(TableId::Countries, &["canonical_slot", "tag"])?,
        ])
    }
    fn observe(
        &mut self,
        _: FormatVersion,
        frame: &DecodedFrame,
        values: &mut Values,
    ) -> io::Result<bool> {
        let (Some(countries), Ok(rows), Ok(links)) = (
            Countries::read(frame),
            frame.complete::<tables::NavalRegionalMines>(),
            frame.complete::<tables::CountryLinks>(),
        ) else {
            shared::note(
                &mut self.refusals,
                "Minefield amounts or country identities are not recorded",
            );
            return Ok(false);
        };
        let links: BTreeMap<_, _> = links.iter().map(|r| (r.id, r.canonical_slot)).collect();
        let mut groups = BTreeMap::<u32, Vec<&NavalRegionalMineRow>>::new();
        // Owners whose vectors this hour holds but cannot name.
        let mut unnamed = BTreeSet::new();
        for row in rows {
            if let Some(slot) = countries.slot(row.country) {
                groups.entry(slot).or_default().push(row);
            } else {
                unnamed.insert(row.country);
                shared::note(
                    &mut self.refusals,
                    "Minefield owner country link is unavailable",
                );
            }
        }
        let mut unnamed = unnamed.len() as i128;
        for (slot, group) in groups {
            let Some(owner) = countries.slotted(slot).filter(|s| !s.contains('|')) else {
                unnamed += 1;
                shared::note(&mut self.refusals, "Minefield owner tag is unavailable");
                continue;
            };
            let raw = group[0].country;
            let group: Vec<_> = group.into_iter().copied().collect();
            let admitted = (|| -> Result<u32, String> {
                let levels = Levels::from_rows(&group, &links)?;
                let witnesses: Vec<_> = group.iter().filter(|r| r.kind == 0).collect();
                let [witness] = witnesses.as_slice() else {
                    return Err(
                        "Regional mine owner completeness witness is missing or duplicated".into(),
                    );
                };
                for row in &group {
                    if row.country != raw {
                        return Err("Regional mine vector duplicates a canonical country".into());
                    }
                    if row.kind == 1 {
                        let _ = levels.amount(raw, row.region)?;
                    }
                }
                Ok(witness.region)
            })();
            match admitted {
                Ok(extent) => {
                    // names: OWNER|extent and OWNER|region; zeros stay sparse.
                    values.push(format!("{owner}|extent"), i128::from(extent));
                    for row in group.iter().filter(|r| r.kind == 1 && r.amount != 0) {
                        values.push(format!("{owner}|{}", row.region), i128::from(row.amount));
                    }
                }
                Err(why) => {
                    // Dated: the owner is known to hold an unadmitted vector.
                    values.push(format!("{owner}|refused"), 1);
                    shared::note(&mut self.refusals, why);
                }
            }
        }
        if unnamed > 0 {
            values.push(format!("{UNNAMED}|refused"), unnamed);
        }
        Ok(true)
    }
}
/// The owner part of the count of vectors the hour holds but cannot name.
const UNNAMED: &str = "?";

pub fn query(q: &Query<'_>) -> io::Result<serde_json::Value> {
    let params = shared::params::<Minefields>(q)?;
    if matches!(params.view, View::Reports) {
        return reports(q, &params);
    }
    let (first, last) = q.window()?;
    let labels = shared::labels(q);
    let mut warnings = labels
        .as_ref()
        .map_or_else(|_| Vec::new(), |l| l.warnings.clone());
    let mut ids = BTreeSet::new();
    let window = match q.open::<MinefieldsObserver>()? {
        Some(mut o) => {
            warnings.extend(shared::warnings(&o.scan.observer.refusals));
            for name in o.scan.names() {
                if let Some((_, r)) = name
                    .split_once('|')
                    .and_then(|(c, r)| r.parse::<u32>().ok().map(|r| (c, r)))
                {
                    ids.insert(r);
                }
            }
            o.window(first, last, q.through(), |name| {
                name.split_once('|').is_some_and(|(owner, _)| {
                    // Owners that could not be named may be any selected country.
                    owner == UNNAMED
                        || params
                            .tags
                            .as_ref()
                            .is_none_or(|tags| tags.iter().any(|t| shared::selected(owner, t)))
                })
            })?
        }
        None => shared::empty(first, last),
    };
    if let Ok(l) = &labels
        && let Ok(sea) = &l.sea_regions
    {
        ids.extend(sea);
    }
    let mut notes = Refusals::default();
    let regions = shared::choices(&ids, &labels, &mut notes);
    warnings.extend(notes.counts("minefield region names"));
    shared::finish(
        q,
        &HistoryReply {
            window,
            regions,
            scale: shared::SCALE,
            warnings,
        },
    )
}

fn reports(q: &Query<'_>, p: &Params) -> io::Result<serde_json::Value> {
    let hour = p.hour.expect("validated reports hour");
    let region = p.region.expect("validated reports region");
    let (first, last) = q.bounds()?;
    if hour < first || hour > last {
        return Err(dashboard_core::invalid(
            "Reports hour is outside the branch",
        ));
    }
    let mut reply = ReportsReply {
        hour,
        read_hour: None,
        region,
        reports: Vec::new(),
        warnings: Vec::new(),
    };
    let tables = &[
        TableId::NavalAccidentReports,
        TableId::Countries,
        TableId::CountryLinks,
        TableId::CountryAppearance,
        TableId::VariantDefinitions,
        TableId::LexicalTokens,
        TableId::CountryCounterInputs,
        TableId::ReservedCountryNaming,
    ];
    let Some(frame) = progressive::frame_at(q.source, q.branch, hour, tables, q.cancelled)? else {
        reply
            .warnings
            .push("1 standing mine report list not shown: requested hour is not recorded".into());
        return shared::finish(q, &reply);
    };
    let rows = match frame.complete::<tables::NavalAccidentReports>() {
        Ok(rows) => rows,
        Err(why) => {
            reply
                .warnings
                .push(format!("1 standing mine report list not shown: {why}"));
            return shared::finish(q, &reply);
        }
    };
    reply.read_hour = frame.tables[TableId::NavalAccidentReports.index()]
        .sampled_at()
        .filter(|h| *h <= hour);
    let cat = q
        .content
        .and_then(|c| catalogue::catalogue(q.source, c).ok());
    let countries = Countries::read(&frame);
    let mut notes = Refusals::default();
    for (at, row) in standing(rows, region).enumerate() {
        dashboard_text::metadata::check_cancel(q.cancelled)?;
        let mut country = |raw, field| {
            shared::country_part(&frame, countries.as_ref(), raw).or_else(|| {
                notes.note(
                    format!("Mine report {field} country is unresolved"),
                    at.to_string(),
                );
                None
            })
        };
        let victim = country(row.owner, "victim");
        let mine_owner = country(row.mine_owner, "mine owner");
        let ship = ship_name(q, &frame, row, cat.as_deref())
            .map_err(|why| notes.note(why, at.to_string()))
            .ok()
            .flatten();
        let equipment = equipment_name(&frame, row, cat.as_deref())
            .map_err(|why| notes.note(why, at.to_string()))
            .ok()
            .flatten();
        reply.reports.push(Report {
            kind: row.kind,
            ordinal: row.ordinal,
            victim,
            mine_owner,
            ship,
            equipment,
            damage: row.damage.to_string(),
            sunk: row.sunk != 0,
            first_seen: None,
        });
    }
    reply
        .warnings
        .extend(notes.counts("standing mine report fields"));
    shared::finish(q, &reply)
}
pub(crate) fn standing(
    rows: &[NavalAccidentReportRow],
    region: u32,
) -> impl Iterator<Item = &NavalAccidentReportRow> {
    rows.iter()
        .filter(move |r| r.kind == 1 && r.region == region)
}
fn ship_name(
    q: &Query<'_>,
    frame: &DecodedFrame,
    row: &NavalAccidentReportRow,
    cat: Option<&catalogue::Catalogue>,
) -> Result<Option<String>, String> {
    if row.ship == 0 {
        return Ok(None);
    }
    let bytes = frame
        .complete::<tables::CountryCounterInputs>()
        .ok()
        .and_then(|rows| rows.iter().find(|r| r.pride_of_the_fleet == row.ship))
        .and_then(|r| r.pride_name.require("CountryCounterInputs.pride_name").ok())
        .and_then(Option::as_ref)
        .ok_or("Standing mine report ship name is not recorded for its full handle")?;
    let cat = cat.ok_or("Standing mine report ship naming content is unavailable")?;
    let language = &q.source.replay_ref().identity.language;
    let recorded = language.strip_prefix("l_").unwrap_or(language);
    dashboard_text::scripted_text::stored_text(
        &cat.localisation,
        recorded,
        bytes,
        "Mine report ship name",
    )
    .and_then(|text| catalogue::plain_text(&text))
    .map(Some)
}
fn equipment_name(
    frame: &DecodedFrame,
    row: &NavalAccidentReportRow,
    cat: Option<&catalogue::Catalogue>,
) -> Result<Option<String>, String> {
    use dashboard_text::variant_names::{self, Tags, Texts};
    if row.equipment == 0 {
        return Ok(None);
    }
    let rows = frame
        .complete::<tables::VariantDefinitions>()
        .map_err(|e| e.to_string())?;
    let definition = rows
        .iter()
        .find(|r| r.id == row.equipment)
        .ok_or("Standing mine report equipment variant is not recorded")?;
    let countries = frame
        .complete::<tables::Countries>()
        .map_err(|e| e.to_string())?;
    let links = frame
        .complete::<tables::CountryLinks>()
        .map_err(|e| e.to_string())?;
    let tags = Tags::new(
        countries,
        links,
        frame.complete::<tables::CountryAppearance>().ok(),
    );
    let lexical = frame
        .complete::<tables::LexicalTokens>()
        .map_err(|e| e.to_string())?;
    let raw = variant_names::raw_variant(definition, frame.hour, &tags, lexical);
    variant_names::variant_name(&raw, cat.map(|c| c as &dyn Texts))?
        .ok_or_else(|| "Standing mine report equipment name is unavailable".into())
        .map(Some)
}

use super::{Content, IDENTITY, Tags, branch, missing, placements::{MISSION, Operative, working}};
use dashboard_base::{Source, products::{Country as ParamCountry, LoadContext, at_hour::{At, AtHour}, validate_country}};
use dashboard_world::products::groups::{FactionAt, FactionGroup, factions_at, factions_valid_through};
use replay_data::rows::{IntelligenceAgencyRow, OperativeMembershipRow, Recorded, TableId, tables};
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    io,
    sync::{Arc, atomic::AtomicBool},
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

const TABLES: [TableId; 4] = [TableId::OperativeMemberships, TableId::IntelligenceAgencies, IDENTITY[0], IDENTITY[1]];
const COUNTER_INTELLIGENCE: u32 = 3;
const TARGET_NOT_RECORDED: &str = "An operative's target country was not recorded at this hour";

pub struct OperativeMatrix;

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "lowercase")]
pub enum OperativeMatrixForm {
    #[default]
    Matrix,
    Pairs,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OperativeMatrixParams {
    pub branch_id: String,
    #[doc = " Chosen countries: the pairs touching them. Empty means every country."]
    #[serde(default)]
    pub countries: Vec<ParamCountry>,
    #[doc = " The countries with the most operatives sent and received, and the pairs between them."]
    #[serde(default)]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub top: Option<u32>,
    #[serde(default)]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub form: Option<OperativeMatrixForm>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct OperativeMatrixCountry {
    pub tag: String,
    #[doc = " The faction it belonged to at the hour, as a decimal id; null when in none."]
    pub faction_id: Option<String>,
    pub faction_name: Option<String>,
    pub faction_name_missing: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum AgencyCapacity {
    #[doc = " The agency screen's active operatives and its maximum, as recorded."]
    Known { active: i32, maximum: i32 },
    #[doc = " The country has founded no agency at the hour."]
    NoAgency,
    Missing { reason: String },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct OperativeMatrixRow {
    #[doc = " Index into `countries`."]
    pub country: u32,
    pub capacity: AgencyCapacity,
    #[doc = " Why a pair of this spy without operatives is not known to have none."]
    pub missing: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct OperativeMatrixCell {
    pub row: u32,
    pub column: u32,
    #[doc = " Counter-intelligence at home: the spy and the target are the same country."]
    pub home: bool,
    pub operatives: Vec<Operative>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct OperativePair {
    #[doc = " Indexes into `countries`."]
    pub spy: u32,
    pub target: u32,
    pub operatives: Vec<Operative>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct OperativeMatrixReply {
    #[doc = " Every country the matrix or the pairs name."]
    pub countries: Vec<OperativeMatrixCountry>,
    #[doc = " Spies, grouped by faction (by faction name, factionless last), then by operatives sent."]
    pub rows: Vec<OperativeMatrixRow>,
    #[doc = " Indexes into `countries`, in the rows' faction order, then by operatives received."]
    pub columns: Vec<u32>,
    #[doc = " The pairs with operatives, by row then column; every other pair has none."]
    pub cells: Vec<OperativeMatrixCell>,
    #[doc = " The most operatives in one displayed pair abroad, for the colour scale."]
    pub most: u32,
    #[doc = " The largest agency maximum among the rows, for the capacity bars."]
    pub largest_maximum: i32,
    #[doc = " Pairs abroad, most operatives first; ties by spy tag, then target tag."]
    pub pairs: Vec<OperativePair>,
    #[doc = " The census's read hour the rows hold (it is taken daily)."]
    pub read_hour: Option<i32>,
    pub missing: Option<String>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(OperativeMatrixReply);

pub struct MatrixContext {
    content: Content,
    factions: Arc<Vec<FactionGroup>>,
}

#[derive(Default)]
pub struct Census {
    pub pairs: BTreeMap<(String, String), Vec<Operative>>,
    pub unknown_target: HashSet<String>,
    pub capacity: HashMap<String, AgencyCapacity>,
}

pub fn census(memberships: &[OperativeMembershipRow], agencies: Result<&[IntelligenceAgencyRow], String>, tags: &Tags, content: &Content) -> Census {
    let mut out = Census::default();
    for row in memberships.iter().filter(|row| working(row)) {
        let Some(owner) = tags.tag(row.owner_country) else { continue };
        match row.target_country {
            Recorded::Value(Some(target)) => {
                let Some(target) = tags.tag(target) else { continue };
                if target == owner && !(row.assignment == MISSION && row.action_variant == Some(COUNTER_INTELLIGENCE)) {
                    continue;
                }
                out.pairs.entry((owner, target)).or_default().push(Operative::of(row, content));
            }
            Recorded::Value(None) => {}
            Recorded::NotRecorded => {
                out.unknown_target.insert(owner);
            }
        }
    }
    match agencies {
        Ok(rows) => {
            for row in rows.iter().filter(|row| row.created != 0) {
                if let Some(tag) = tags.tag(row.country) {
                    out.capacity.insert(tag, AgencyCapacity::Known { active: row.active_count, maximum: row.unknown_f0 as i32 });
                }
            }
        }
        Err(reason) => {
            for (owner, _) in out.pairs.keys() {
                out.capacity.insert(owner.clone(), AgencyCapacity::Missing { reason: reason.clone() });
            }
        }
    }
    out
}

pub struct Shape<'a> {
    pub chosen: Option<&'a HashSet<String>>,
    pub top: Option<u32>,
    pub factions: &'a HashMap<String, FactionAt>,
    pub form: OperativeMatrixForm,
}

pub fn build(census: &Census, shape: &Shape<'_>) -> (Vec<OperativeMatrixCountry>, Vec<OperativeMatrixRow>, Vec<u32>, Vec<OperativeMatrixCell>, u32, i32, Vec<OperativePair>) {
    let mut sent: HashMap<&str, usize> = HashMap::new();
    let mut received: HashMap<&str, usize> = HashMap::new();
    for ((owner, target), operatives) in &census.pairs {
        if owner != target {
            *sent.entry(owner).or_default() += operatives.len();
            *received.entry(target).or_default() += operatives.len();
        }
    }
    let both = |tag: &str| sent.get(tag).copied().unwrap_or(0) + received.get(tag).copied().unwrap_or(0);
    let mut spies: HashSet<&str> = census.capacity.iter().filter(|(_, capacity)| !matches!(capacity, AgencyCapacity::NoAgency)).map(|(tag, _)| tag.as_str()).collect();
    spies.extend(census.pairs.keys().map(|(owner, _)| owner.as_str()));
    let ranked_top: Option<HashSet<&str>> = shape.top.filter(|_| shape.chosen.is_none()).map(|limit| {
        let mut everyone: Vec<&str> = spies.iter().copied().chain(census.pairs.keys().map(|(_, target)| target.as_str())).collect::<HashSet<_>>().into_iter().collect();
        everyone.sort_by(|a, b| both(b).cmp(&both(a)).then(a.cmp(b)));
        everyone.truncate(limit as usize);
        everyone.into_iter().collect()
    });
    let keep = |owner: &str, target: &str| match (shape.chosen, &ranked_top) {
        (Some(chosen), _) => chosen.contains(owner) || chosen.contains(target),
        (None, Some(top)) => top.contains(owner) && top.contains(target),
        (None, None) => true,
    };
    let kept: Vec<(&(String, String), &Vec<Operative>)> = census.pairs.iter().filter(|((owner, target), _)| keep(owner, target)).collect();
    let mut row_tags: HashSet<&str> = kept.iter().map(|((owner, _), _)| owner.as_str()).collect();
    let mut column_tags: HashSet<&str> = kept.iter().map(|((_, target), _)| target.as_str()).collect();
    for &spy in &spies {
        let shown = match (shape.chosen, &ranked_top) {
            (Some(chosen), _) => chosen.contains(spy),
            (None, Some(top)) => top.contains(spy),
            (None, None) => true,
        };
        if shown {
            row_tags.insert(spy);
            column_tags.insert(spy);
        }
    }
    if let Some(chosen) = shape.chosen {
        column_tags.extend(chosen.iter().map(String::as_str));
    }
    let faction_of = |tag: &str| shape.factions.get(tag);
    let group_key = |tag: &str| match faction_of(tag) {
        Some(faction) => (0u8, faction.name.clone().unwrap_or_default(), faction.id.clone()),
        None => (1u8, String::new(), String::new()),
    };
    let mut ordered_rows: Vec<&str> = row_tags.into_iter().collect();
    ordered_rows.sort_by(|a, b| group_key(a).cmp(&group_key(b)).then(sent.get(b).cmp(&sent.get(a))).then(a.cmp(b)));
    let mut ordered_columns: Vec<&str> = column_tags.into_iter().collect();
    ordered_columns.sort_by(|a, b| group_key(a).cmp(&group_key(b)).then(received.get(b).cmp(&received.get(a))).then(a.cmp(b)));

    let mut countries: Vec<OperativeMatrixCountry> = Vec::new();
    let mut index: HashMap<String, u32> = HashMap::new();
    let mut intern = |tag: &str, countries: &mut Vec<OperativeMatrixCountry>| -> u32 {
        if let Some(&at) = index.get(tag) {
            return at;
        }
        let at = countries.len() as u32;
        let (faction_id, faction_name, faction_name_missing) = match faction_of(tag) {
            Some(faction) => (Some(faction.id.clone()), faction.name.clone(), faction.name_missing.clone()),
            None => (None, None, None),
        };
        countries.push(OperativeMatrixCountry { tag: tag.to_owned(), faction_id, faction_name, faction_name_missing });
        index.insert(tag.to_owned(), at);
        at
    };

    let mut pairs: Vec<OperativePair> = Vec::new();
    let mut abroad: Vec<&(&(String, String), &Vec<Operative>)> = kept.iter().filter(|((owner, target), _)| owner != target).collect();
    abroad.sort_by(|a, b| b.1.len().cmp(&a.1.len()).then(a.0.0.cmp(&b.0.0)).then(a.0.1.cmp(&b.0.1)));

    if shape.form == OperativeMatrixForm::Pairs {
        for ((owner, target), operatives) in abroad {
            let spy = intern(owner, &mut countries);
            let target = intern(target, &mut countries);
            pairs.push(OperativePair { spy, target, operatives: (*operatives).clone() });
        }
        return (countries, Vec::new(), Vec::new(), Vec::new(), 0, 0, pairs);
    }

    let row_index: HashMap<&str, u32> = ordered_rows.iter().enumerate().map(|(at, tag)| (*tag, at as u32)).collect();
    let column_index: HashMap<&str, u32> = ordered_columns.iter().enumerate().map(|(at, tag)| (*tag, at as u32)).collect();
    let mut rows = Vec::with_capacity(ordered_rows.len());
    let mut largest_maximum = 0;
    for tag in &ordered_rows {
        let country = intern(tag, &mut countries);
        let capacity = census.capacity.get(*tag).cloned().unwrap_or(AgencyCapacity::NoAgency);
        if let AgencyCapacity::Known { maximum, .. } = capacity {
            largest_maximum = largest_maximum.max(maximum);
        }
        let missing = census.unknown_target.contains(*tag).then(|| TARGET_NOT_RECORDED.to_owned());
        rows.push(OperativeMatrixRow { country, capacity, missing });
    }
    let columns: Vec<u32> = ordered_columns.iter().map(|tag| intern(tag, &mut countries)).collect();
    let mut cells = Vec::with_capacity(kept.len());
    let mut most = 0u32;
    for ((owner, target), operatives) in &kept {
        let (Some(&row), Some(&column)) = (row_index.get(owner.as_str()), column_index.get(target.as_str())) else { continue };
        let home = owner == target;
        if !home {
            most = most.max(operatives.len() as u32);
        }
        cells.push(OperativeMatrixCell { row, column, home, operatives: (*operatives).clone() });
    }
    cells.sort_by_key(|cell| (cell.row, cell.column));
    (countries, rows, columns, cells, most, largest_maximum, pairs)
}

fn empty_reply(read_hour: Option<i32>, missing: Option<String>, warnings: Vec<String>) -> OperativeMatrixReply {
    OperativeMatrixReply { countries: Vec::new(), rows: Vec::new(), columns: Vec::new(), cells: Vec::new(), most: 0, largest_maximum: 0, pairs: Vec::new(), read_hour, missing, warnings }
}

impl AtHour for OperativeMatrix {
    const KEY: &'static str = "operative-matrix";
    const TITLE: &'static str = "Operative matrix";
    const CONTENT_DEPENDENT: bool = true;
    const CLOCK: dashboard_base::products::at_hour::Clock = dashboard_base::products::at_hour::Clock::Boundaries;
    type Params = OperativeMatrixParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, countries, "countries[]", top, form];
    type Reply = OperativeMatrixReply;
    type Context = MatrixContext;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![
            OperativeMatrixParams::decl(),
            OperativeMatrixForm::decl(),
            Operative::decl(),
            OperativeMatrixCountry::decl(),
            AgencyCapacity::decl(),
            OperativeMatrixRow::decl(),
            OperativeMatrixCell::decl(),
            OperativePair::decl(),
            OperativeMatrixReply::decl(),
        ]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        branch(&params.branch_id)?;
        params.countries.iter().try_for_each(validate_country)
    }

    fn tables(_params: &Self::Params) -> Vec<TableId> {
        TABLES.to_vec()
    }

    fn context(source: &Source, load: &LoadContext) -> io::Result<Self::Context> {
        Ok(MatrixContext { content: Content::load(source, load), factions: Arc::new(Vec::new()) })
    }

    fn context_for(source: &Source, load: &LoadContext, params: &Self::Params) -> io::Result<Self::Context> {
        let content = Content::load(source, load);
        let factions = params
            .branch_id
            .parse::<u64>()
            .ok()
            .and_then(|id| dashboard_base::queries::query(source, id).ok())
            .and_then(|query| dashboard_world::products::groups::groups(source, &query, &params.branch_id, load, &AtomicBool::new(false)).ok())
            .map_or_else(|| Arc::new(Vec::new()), |reply| Arc::new(reply.factions));
        Ok(MatrixContext { content, factions })
    }

    fn valid_through(_frame: &DecodedFrame, _params: &Self::Params, at: &At<'_, Self::Context>) -> i32 {
        factions_valid_through(&at.context.factions, at.hour)
    }

    fn reply(frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>) -> io::Result<Self::Reply> {
        let read_hour = frame.tables[TableId::OperativeMemberships.index()].sampled_at();
        if let Some(reason) = params.countries.iter().find(|country| at.holders.tag_at(country, at.hour).is_none()).and_then(|country| at.holders.not_known(country, at.hour)) {
            return Ok(empty_reply(read_hour, Some(reason), Vec::new()));
        }
        if let Some(reason) = missing(frame, TableId::OperativeMemberships) {
            return Ok(empty_reply(read_hour, Some(reason), Vec::new()));
        }
        let (tags, memberships) = match (Tags::read(frame), frame.complete::<tables::OperativeMemberships>()) {
            (Ok(tags), Ok(rows)) => (tags, rows),
            (Err(error), _) | (_, Err(error)) => return Ok(empty_reply(read_hour, Some(error.to_string()), Vec::new())),
        };
        let agencies = frame
            .complete::<tables::IntelligenceAgencies>()
            .map_err(|error| missing(frame, TableId::IntelligenceAgencies).unwrap_or_else(|| error.to_string()));
        let census = census(memberships, agencies, &tags, &at.context.content);
        let chosen: Option<HashSet<String>> = (!params.countries.is_empty()).then(|| {
            params.countries.iter().filter_map(|country| at.holders.tag_at(country, at.hour).map(str::to_owned).or_else(|| country.tag().map(str::to_owned))).collect()
        });
        let factions = factions_at(&at.context.factions, at.hour);
        let mut warnings = Vec::new();
        if at.context.factions.is_empty() {
            warnings.push("Factions could not be read for this branch; countries are not grouped.".to_owned());
        }
        let shape = Shape { chosen: chosen.as_ref(), top: params.top, factions: &factions, form: params.form.unwrap_or_default() };
        let (countries, rows, columns, cells, most, largest_maximum, pairs) = build(&census, &shape);
        Ok(OperativeMatrixReply { countries, rows, columns, cells, most, largest_maximum, pairs, read_hour, missing: None, warnings })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn operative(id: u64) -> Operative {
        Operative { id: id.to_string(), name: Some(format!("Agent {id}")), name_missing: None }
    }

    fn census_of(pairs: &[(&str, &str, usize)], capacity: &[(&str, AgencyCapacity)], unknown: &[&str]) -> Census {
        let mut out = Census::default();
        let mut next = 0;
        for &(owner, target, count) in pairs {
            let list = (0..count).map(|_| {
                next += 1;
                operative(next)
            });
            out.pairs.entry((owner.to_owned(), target.to_owned())).or_default().extend(list);
        }
        for (tag, held) in capacity {
            out.capacity.insert((*tag).to_owned(), held.clone());
        }
        out.unknown_target = unknown.iter().map(|tag| (*tag).to_owned()).collect();
        out
    }

    fn faction(tag: &str, id: &str, name: &str) -> (String, FactionAt) {
        (tag.to_owned(), FactionAt { id: id.to_owned(), name: Some(name.to_owned()), name_missing: None })
    }

    #[test]
    fn rows_group_by_faction_then_operatives_sent_and_home_pairs_are_counter_intelligence() {
        let census = census_of(
            &[("GER", "ENG", 3), ("GER", "SOV", 2), ("ENG", "GER", 2), ("ENG", "ENG", 2), ("ITA", "ENG", 1), ("SOV", "GER", 2)],
            &[("GER", AgencyCapacity::Known { active: 5, maximum: 8 }), ("ENG", AgencyCapacity::Known { active: 4, maximum: 7 })],
            &[],
        );
        let factions: HashMap<_, _> = [faction("GER", "2", "Axis"), faction("ITA", "2", "Axis"), faction("ENG", "1", "Allies")].into_iter().collect();
        let (countries, rows, columns, cells, most, largest, pairs) = build(&census, &Shape { chosen: None, top: None, factions: &factions, form: OperativeMatrixForm::Matrix });
        let tag = |at: u32| countries[at as usize].tag.as_str();
        assert_eq!(rows.iter().map(|row| tag(row.country)).collect::<Vec<_>>(), ["ENG", "GER", "ITA", "SOV"]);
        assert_eq!(columns.iter().map(|&at| tag(at)).collect::<Vec<_>>(), ["ENG", "GER", "ITA", "SOV"]);
        assert_eq!(most, 3);
        assert_eq!(largest, 8);
        let home = cells.iter().find(|cell| cell.home).unwrap();
        assert_eq!((tag(rows[home.row as usize].country), home.operatives.len()), ("ENG", 2));
        assert_eq!(rows[2].capacity, AgencyCapacity::NoAgency);
        assert!(pairs.is_empty());
    }

    #[test]
    fn pairs_rank_by_operatives_then_tags_and_leave_out_home() {
        let census = census_of(&[("GER", "ENG", 2), ("ENG", "GER", 2), ("CHI", "JAP", 3), ("ENG", "ENG", 4)], &[], &[]);
        let (countries, rows, _, cells, _, _, pairs) = build(&census, &Shape { chosen: None, top: None, factions: &HashMap::new(), form: OperativeMatrixForm::Pairs });
        assert!(rows.is_empty() && cells.is_empty());
        let named: Vec<(&str, &str, usize)> = pairs.iter().map(|pair| (countries[pair.spy as usize].tag.as_str(), countries[pair.target as usize].tag.as_str(), pair.operatives.len())).collect();
        assert_eq!(named, [("CHI", "JAP", 3), ("ENG", "GER", 2), ("GER", "ENG", 2)]);
    }

    #[test]
    fn a_spy_with_an_unrecorded_target_is_not_known_to_have_none_elsewhere() {
        let census = census_of(&[("GER", "ENG", 1)], &[("ITA", AgencyCapacity::Known { active: 1, maximum: 2 })], &["ITA"]);
        let (countries, rows, ..) = build(&census, &Shape { chosen: None, top: None, factions: &HashMap::new(), form: OperativeMatrixForm::Matrix });
        let italy = rows.iter().find(|row| countries[row.country as usize].tag == "ITA").unwrap();
        assert_eq!(italy.missing.as_deref(), Some(TARGET_NOT_RECORDED));
        let germany = rows.iter().find(|row| countries[row.country as usize].tag == "GER").unwrap();
        assert_eq!(germany.missing, None);
    }

    #[test]
    fn chosen_countries_keep_the_pairs_touching_them() {
        let census = census_of(&[("GER", "ENG", 1), ("SOV", "JAP", 2), ("ENG", "ITA", 1)], &[], &[]);
        let chosen: HashSet<String> = ["ENG".to_owned()].into_iter().collect();
        let (countries, rows, columns, cells, ..) = build(&census, &Shape { chosen: Some(&chosen), top: None, factions: &HashMap::new(), form: OperativeMatrixForm::Matrix });
        let mut row_tags: Vec<&str> = rows.iter().map(|row| countries[row.country as usize].tag.as_str()).collect();
        row_tags.sort_unstable();
        assert_eq!(row_tags, ["ENG", "GER"]);
        assert_eq!(columns.len(), 2);
        assert_eq!(cells.len(), 2);
    }

    #[test]
    fn top_keeps_the_busiest_countries_and_only_pairs_between_them() {
        let census = census_of(&[("GER", "ENG", 3), ("ENG", "GER", 2), ("SOV", "JAP", 1)], &[], &[]);
        let (countries, rows, columns, cells, ..) = build(&census, &Shape { chosen: None, top: Some(2), factions: &HashMap::new(), form: OperativeMatrixForm::Matrix });
        assert_eq!(rows.len(), 2);
        assert_eq!(columns.len(), 2);
        assert_eq!(cells.len(), 2);
        assert!(countries.iter().all(|country| country.tag == "GER" || country.tag == "ENG"));
    }
}

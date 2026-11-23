
use super::{Named, rows};
use dashboard_base::{Source, products::{LoadContext, at_hour::{At, AtHour}}};
use dashboard_follow::live::{Cells, Source as Cell};
use replay_data::rows::{CountryGameScoreRow, GameScoreMultiplierRow, Recorded, TableId, tables};
use dashboard_text::catalogue::{View, localised};
use content_reader_localisation::localisation::Localisation;
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, io};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

pub struct GameScore;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GameScoreParams {
    pub branch_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct GameScoreCell {
    pub missing: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct GameScoreRow {
    pub tag: String,
    pub country_id: u32,
    pub total: GameScoreCell,
    pub army: GameScoreCell,
    pub navy: GameScoreCell,
    pub air: GameScoreCell,
    pub industry: GameScoreCell,
    pub land: GameScoreCell,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct GameScoreMultiplier {
    pub part: String,
    pub raw: Option<String>,
    pub missing: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct GameScoreReply {
    #[doc = " Every country scored at the hour that a tag holds, in tag order."]
    pub rows: Vec<GameScoreRow>,
    pub multipliers: Vec<GameScoreMultiplier>,
    #[doc = " Each column's hint as the game's end screen words its sort control, by column (`country`,"]
    #[doc = " `total`, `army`, `navy`, `air`, `industry`, `land`); a column the content cannot word is"]
    #[doc = " absent, and a warning says why."]
    pub hints: BTreeMap<String, String>,
    #[doc = " Why no country's scores are known at the hour."]
    pub missing: Option<String>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(GameScoreReply);

pub(super) fn cell(value: &Recorded<i64>, reason: u8) -> GameScoreCell {
    match (value, reason) {
        (Recorded::Value(_), CountryGameScoreRow::KNOWN) => GameScoreCell { missing: None },
        (_, CountryGameScoreRow::KNOWN) => GameScoreCell { missing: Some("The score was not recorded".into()) },
        (_, reason) => GameScoreCell {
            missing: Some(CountryGameScoreRow::REASONS.get(usize::from(reason)).map_or_else(|| format!("Unknown game score reason {reason}"), |text| (*text).to_owned())),
        },
    }
}

pub(super) fn row(tag: &str, score: &CountryGameScoreRow) -> GameScoreRow {
    GameScoreRow {
        tag: tag.to_owned(),
        country_id: score.country,
        total: cell(&score.total, score.total_reason),
        army: cell(&score.army, score.army_reason),
        navy: cell(&score.navy, score.navy_reason),
        air: cell(&score.air, score.air_reason),
        industry: cell(&score.industry, score.industry_reason),
        land: cell(&score.land, score.land_reason),
    }
}

const SCORES: [&str; 6] = ["total", "army", "navy", "air", "industry", "land"];

pub(super) fn name_cells(live: &Cells, tag: &str, score: &CountryGameScoreRow, columns: &[usize; 6]) {
    let shown = row(tag, score);
    for ((part, column), cell) in SCORES.iter().zip(columns).zip([&shown.total, &shown.army, &shown.navy, &shown.air, &shown.industry, &shown.land]) {
        if cell.missing.is_none() {
            live.value(tag, part, Cell { table: TableId::CountryGameScores, column: *column, key: u128::from(score.country) }, 1e-5);
        }
    }
}

fn multipliers(row: &GameScoreMultiplierRow) -> Vec<GameScoreMultiplier> {
    let one = |part: &str, raw: Option<i64>, error: &[u8]| GameScoreMultiplier {
        part: part.to_owned(),
        raw: raw.map(|raw| raw.to_string()),
        missing: raw.is_none().then(|| String::from_utf8_lossy(error).into_owned()),
    };
    vec![
        one("total", row.total_raw, &row.total_error),
        one("army", row.army_raw, &row.army_error),
        one("navy", row.navy_raw, &row.navy_error),
        one("air", row.air_raw, &row.air_error),
        one("industry", row.industry_raw, &row.industry_error),
        one("land", row.land_raw, &row.land_error),
    ]
}

const SORT_KEYS: [(&str, &str); 7] = [
    ("country", "ENDGAME_SORT_NAME"),
    ("total", "ENDGAME_SORT_SCORE"),
    ("army", "ENDGAME_SORT_ARMY"),
    ("navy", "ENDGAME_SORT_NAVY"),
    ("air", "ENDGAME_SORT_AIR"),
    ("industry", "ENDGAME_SORT_INDUSTRY"),
    ("land", "ENDGAME_SORT_PROVINCE"),
];

pub(super) fn hints(localisation: &Localisation) -> (BTreeMap<String, String>, Vec<String>) {
    let mut out = BTreeMap::new();
    let mut warnings = Vec::new();
    for (column, key) in SORT_KEYS {
        if localisation.get(key).is_none() {
            warnings.push(format!("Game score hint {column}: localisation {key} is not defined"));
            continue;
        }
        match localised(localisation, key, View::Scripted) {
            Ok(text) => {
                out.insert(column.to_owned(), text);
            }
            Err(reason) => warnings.push(format!("Game score hint {column}: {reason}")),
        }
    }
    (out, warnings)
}

impl AtHour for GameScore {
    const KEY: &'static str = "game-score";
    const TITLE: &'static str = "Game score";
    const CONTENT_DEPENDENT: bool = true;
    type Params = GameScoreParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId];
    type Reply = GameScoreReply;
    type Context = (BTreeMap<String, String>, Vec<String>);

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![GameScoreParams::decl(), GameScoreCell::decl(), GameScoreRow::decl(), GameScoreMultiplier::decl(), GameScoreReply::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        super::branch(&params.branch_id)
    }


    const CLOCK_DEPENDENT: bool = false;

    fn advance(frame: &DecodedFrame, _params: &Self::Params, _at: &At<'_, Self::Context>, edits: &dashboard_base::products::at_hour::Edits, live: &dashboard_follow::live::Live, _data: &mut dashboard_base::products::at_hour::Held<'_>) -> io::Result<bool> {
        let id = TableId::CountryGameScores;
        if !edits.only_columns(id, dashboard_base::products::at_hour::field_mask(id, &SCORES)) { return Ok(false); }
        let rows = frame.complete::<tables::CountryGameScores>()?;
        for (&key, &mask) in edits.tables.get(&id).into_iter().flatten() {
            let Ok(i) = rows.binary_search_by_key(&(key as u32), |row| row.country) else { return Ok(false) };
            let row = &rows[i];
            for (name, known) in SCORES.iter().zip([
                (&row.total, row.total_reason), (&row.army, row.army_reason), (&row.navy, row.navy_reason),
                (&row.air, row.air_reason), (&row.industry, row.industry_reason), (&row.land, row.land_reason),
            ]) {
                let column = dashboard_declare::field(id, name)?;
                if mask & (1 << column) == 0 { continue; }
                let shown = live.groups.iter().any(|g| g.table == id && g.columns.contains(&column) && g.keys.binary_search(&key).is_ok());
                if shown != (matches!(known.0, Recorded::Value(_)) && known.1 == CountryGameScoreRow::KNOWN) { return Ok(false); }
            }
        }
        Ok(true)
    }

    fn tables(_params: &Self::Params) -> Vec<TableId> {
        vec![TableId::CountryGameScores, TableId::GameScoreMultipliers]
    }

    fn context(source: &Source, load: &LoadContext) -> io::Result<Self::Context> {
        Ok(match load.content.as_ref() {
            Some(content) => match dashboard_base::content::localisation(source, content) {
                Ok(localisation) => hints(&localisation),
                Err(error) => (BTreeMap::new(), vec![format!("Game score hints are unavailable: {error}")]),
            },
            None => (BTreeMap::new(), vec!["Game score hints are unavailable until the game folders are chosen".to_owned()]),
        })
    }

    fn reply(frame: &DecodedFrame, _params: &Self::Params, at: &At<'_, Self::Context>) -> io::Result<Self::Reply> {
        let (hints, hint_warnings) = at.context;
        let mut reply = GameScoreReply { rows: Vec::new(), multipliers: Vec::new(), hints: hints.clone(), missing: None, warnings: hint_warnings.clone() };
        match rows::<tables::GameScoreMultipliers>(frame) {
            Ok(held) => reply.multipliers = held.first().map(multipliers).unwrap_or_default(),
            Err(reason) => reply.warnings.push(format!("Score multipliers: {reason}")),
        }
        let scores = match rows::<tables::CountryGameScores>(frame) {
            Ok(scores) => scores,
            Err(reason) => {
                reply.missing = Some(format!("Game scores are not known at this hour: {reason}"));
                return Ok(reply);
            }
        };
        let mut columns = [0usize; 6];
        for (at, name) in SCORES.iter().enumerate() {
            columns[at] = dashboard_declare::field(TableId::CountryGameScores, name)?;
        }
        let named = Named::new(at.holders, frame.hour);
        let mut unheld = 0usize;
        for score in scores {
            match named.id(score.country) {
                Some(tag) => {
                    name_cells(at.live, tag, score, &columns);
                    reply.rows.push(row(tag, score));
                }
                None => unheld += 1,
            }
        }
        reply.rows.sort_unstable_by(|a, b| a.tag.cmp(&b.tag));
        if unheld > 0 {
            reply.warnings.push(format!("{unheld} scored countries are held by no tag at this hour and are not shown"));
        }
        Ok(reply)
    }
}

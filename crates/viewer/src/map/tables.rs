//! Reading one decoded hour's tables when the hour does not hold all of them.
//!
//! The collector grows its buffers over the first hours of a session, so an
//! early frame records several tables truncated and a staggered table not at
//! all. `README.md` ("Truth") says the viewer shows gaps, truncated tables and
//! unavailable tables **as such** — it does not say it refuses the hour — so a
//! pass reads what the hour holds through [`rows`] and states what it could not
//! draw through [`Limitations`].
//!
//! # Core and the rest
//!
//! Three tables are **core**: `Provinces` carries the per-province owner,
//! controller and state the political fill *is*, and `CountryLinks` and
//! `Countries` turn its raw country references into the canonical slots and
//! tags every colour, flag and name is keyed by. Without all three there is no
//! political map to degrade, so [`core`] refuses the frame with a message that
//! names the table and says so. Every other table the map reads degrades: see
//! [`limitations`] for the full list and what each gap costs.

use std::io;

use replay::{
    Hour,
    frame::{DecodedFrame, TableView},
    rows::{TableRows, TableSchema, tables},
};

/// The core tables, named in the error a missing one produces.
const CORE: &str = "Provinces, CountryLinks and Countries";

/// What one table of one decoded hour actually holds.
///
/// `Truncated` keeps the rows the collector's partial walk did reach: they are
/// real recorded rows, not a guess, and a pass may draw them. A partial walk
/// never asserts that the absent keys were deleted, so `missing` counts rows
/// the hour is silent about, not rows that are gone.
pub enum Available<'a, T: TableSchema> {
    /// Every row the hour recorded.
    Complete(&'a [T::Row]),
    /// The rows the walk reached, and how many rows it did not reach.
    Truncated { rows: &'a [T::Row], missing: u32 },
    /// The table failed for this hour, with the collector's own reason.
    Unavailable { reason: &'a str },
}

impl<'a, T: TableSchema> Available<'a, T> {
    /// The rows the hour holds. An unavailable table holds none.
    pub fn rows(&self) -> &'a [T::Row] {
        match self {
            Available::Complete(rows) | Available::Truncated { rows, .. } => rows,
            Available::Unavailable { .. } => &[],
        }
    }

    /// Whether the hour holds the whole table.
    pub fn is_complete(&self) -> bool {
        matches!(self, Available::Complete(_))
    }

    /// How many rows the hour is missing, where that is known. An unavailable
    /// table has no count at all: the walk never ran.
    pub fn missing(&self) -> Option<u32> {
        match self {
            Available::Complete(_) => Some(0),
            Available::Truncated { missing, .. } => Some(*missing),
            Available::Unavailable { .. } => None,
        }
    }
}

/// The rows of `T` this hour holds, whatever state its table is in.
///
/// The only error is a decoded table holding another table's rows, which is a
/// corrupt file rather than a recorded gap.
pub fn rows<T: TableSchema>(frame: &DecodedFrame) -> io::Result<Available<'_, T>> {
    match &frame.tables[T::ID.index()] {
        TableView::Complete(rows) => Ok(Available::Complete(typed::<T>(rows, frame.hour)?)),
        TableView::Truncated { rows, missing } => Ok(Available::Truncated {
            rows: typed::<T>(rows, frame.hour)?,
            missing: *missing,
        }),
        TableView::Unavailable { reason } => Ok(Available::Unavailable { reason }),
    }
}

/// Every row of a core table, or a named error for the whole frame.
///
/// A truncated `Provinces` table would paint part of the world and leave the
/// rest unowned, which is an invented map rather than a stated gap, so the
/// hour is refused instead. See the module docs for why these three.
pub fn core<T: TableSchema>(frame: &DecodedFrame) -> io::Result<&[T::Row]> {
    let table = T::ID;
    let hour = frame.hour;
    match rows::<T>(frame)? {
        Available::Complete(rows) => Ok(rows),
        Available::Truncated { missing, .. } => Err(invalid(format!(
            "hour {hour} {table:?} is truncated ({missing} missing rows); the core tables {CORE} \
             carry the political map itself, so this hour cannot be drawn"
        ))),
        Available::Unavailable { reason } => Err(invalid(format!(
            "hour {hour} {table:?} is unavailable: {reason}; the core tables {CORE} carry the \
             political map itself, so this hour cannot be drawn"
        ))),
    }
}

/// What one hour could not draw, in the words the window and a dump both show.
///
/// One line per degraded table, in the fixed order of [`limitations`], so the
/// same hour always reports the same list.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Limitations {
    lines: Vec<String>,
}

impl Limitations {
    /// Records `affected` when this hour's `T` table is not complete.
    ///
    /// `affected` is what the map draws differently because of the gap, stated
    /// in the present tense: it is read by someone looking at the render.
    pub fn note<T: TableSchema>(&mut self, frame: &DecodedFrame, affected: &str) {
        let table = T::ID;
        match &frame.tables[table.index()] {
            TableView::Complete(_) => {}
            TableView::Truncated { missing, .. } => self.lines.push(format!(
                "Table {table:?} truncated ({missing} missing rows): {affected}"
            )),
            TableView::Unavailable { reason } => self
                .lines
                .push(format!("Table {table:?} unavailable: {reason}: {affected}")),
        }
    }

    /// Records a limitation of one recorded **field** rather than of a table.
    ///
    /// A row can be present, and its table complete, and the field still carry
    /// nothing a pass can draw: `Countries.name` is the engine's own rendered
    /// name and an unused country's is the empty string, and a
    /// `ProvinceNames.name` is raw engine bytes that need not be text at all.
    /// The table is complete, so [`Limitations::note`] has nothing to say about
    /// it, but the pass still fell back and the reader still has to be told
    /// which field it fell back from.
    pub fn note_field(&mut self, field: &str, affected: &str) {
        self.lines.push(format!("Field {field}: {affected}"));
    }

    /// The recorded lines, one per degraded table. An hour that drew
    /// everything it reads has none.
    pub fn lines(&self) -> &[String] {
        &self.lines
    }

    /// Records what the map's chosen perspective is, and what it cannot decide.
    ///
    /// The perspective is not a table gap, so [`Limitations::note`] has nothing
    /// to say about it, but every counter on the frame depends on it and the
    /// reader has to be told which country the map was drawn for — and, when it
    /// was drawn for nobody, why. The wording lives here beside every other
    /// degradation's so the window and a dump cannot state different ones.
    pub fn note_perspective(&mut self, perspective: &Perspective) {
        match perspective {
            Perspective::Country { tag, requested } => {
                if *requested {
                    self.lines.push(format!(
                        "Perspective {tag}: the map is drawn as {tag} sees it rather than as the \
                         recording's own player does, so a stack {tag} cannot see carries the \
                         game's question mark here and one the recorded player could not see may \
                         not",
                    ));
                } else {
                    self.lines.push(format!(
                        "Perspective {tag}: the map is drawn as the recorded human player sees it. \
                         A stack {tag} has no intelligence on carries the game's own question mark \
                         and no bars, as the game draws it",
                    ));
                }
                self.lines.push(
                    "Province visibility is not recorded: the engine also shows a stack in full \
                     when the province it stands in is fully revealed to the viewing country (the \
                     client's own fog array, rebuilt per frame from units, radar and detection), \
                     and no table carries it. A stack standing inside our own lines that the game \
                     would draw in full is drawn here as a question mark"
                        .to_owned(),
                );
            }
            Perspective::Omniscient { reason } => self.lines.push(format!(
                "Perspective omniscient: {reason}, so every stack is drawn in full, including the \
                 ones the game hid behind a question mark",
            )),
        }
    }
}

/// What the map's perspective resolved to, in the two shapes it can take.
///
/// `map::perspective` decides this; the wording of it lives here.
pub enum Perspective {
    /// One country's view. `requested` is set when the caller named the tag
    /// rather than taking the recorded player, so the line can say the map is
    /// not the recording's own view.
    Country { tag: String, requested: bool },
    /// Every stack in full, with the reason in the words the line uses.
    Omniscient { reason: String },
}

/// Every non-core table the map reads, and what a gap in it costs this hour.
///
/// This is the one place that list lives: the window and `--dump` both report
/// it, so neither can state a different limitation from the other. A pass is
/// responsible only for degrading; the wording of the degradation is here, next
/// to the wording of every other one.
///
/// The core tables are absent on purpose. A gap in one of them is not a
/// limitation of the render, it is the absence of the render, and [`core`]
/// names it as a frame error instead.
pub fn limitations(frame: &DecodedFrame) -> Limitations {
    let mut out = Limitations::default();
    // borders: the demilitarised-zone bucket bank
    out.note::<tables::StateFlags>(
        frame,
        "the demilitarised-zone border is not drawn at all: a state this hour is silent about \
         cannot be assumed not to be demilitarised, and half a zone is a wrong outline rather than \
         a short one",
    );
    out.note::<tables::States>(
        frame,
        "the demilitarised-zone border is not drawn at all: the recorded flags are joined to this \
         table's state ids, and a state it is silent about cannot be shown to exist",
    );
    // names
    out.note::<tables::ProvinceNames>(
        frame,
        "a victory point renamed in game keeps the static localised name of its province, because \
         a province this hour is silent about cannot be assumed not to have been renamed",
    );
    out.note::<tables::Politics>(
        frame,
        "country names and counter flags fall back to the plain <TAG> key, and a capital carries \
         no star",
    );
    out.note::<tables::PoliticalParties>(
        frame,
        "country names and counter flags fall back to the plain <TAG> key",
    );
    // counters: allegiance
    out.note::<tables::Players>(
        frame,
        "the allegiance perspective falls back to the first human controller",
    );
    out.note::<tables::HumanControllers>(
        frame,
        "the allegiance perspective is taken from the Players table alone",
    );
    out.note::<tables::WarRelations>(
        frame,
        "no country is coloured as an enemy and the hour is drawn as if no war were running; a war \
         is also what stops a faction partner from being seen in full, so a missing table can only \
         show more than the game did, never less",
    );
    out.note::<tables::FactionMembers>(
        frame,
        "no country is coloured as an ally, and counters are grouped as if no faction existed; a \
         partner among the missing rows is also a country whose stacks this perspective cannot see \
         and draws with the game's question mark",
    );
    out.note::<tables::Subjects>(
        frame,
        "an overlord and its subject among the missing rows do not see each other's stacks, so one \
         of them carries the game's question mark where the game drew it in full",
    );
    out.note::<tables::UnitIntel>(
        frame,
        "a division or task force among the missing rows keeps the detection level 0 the engine \
         gives an absent entry, so a stack this perspective had detected is drawn with the game's \
         question mark",
    );
    // counters
    out.note::<tables::Divisions>(
        frame,
        "land counters count only the recorded divisions, and a land combat's attack direction may \
         be missing",
    );
    out.note::<tables::DivisionTemplates>(
        frame,
        "a land counter whose division's template is among the missing rows draws no unit-type          icon, and its dark overlay panel is left bare",
    );
    out.note::<tables::DivisionStatus>(
        frame,
        "a land counter's organisation and strength bars fall back to full for every division          among the missing rows, and its supply, attrition, entrenchment, exercise and no-order          glyphs are not drawn",
    );
    out.note::<tables::DivisionMovement>(
        frame,
        "a division among the missing rows counts as standing still: its counter sits on its          province's own anchor instead of along its route, and its supply crate is not suppressed          for a convoy crossing",
    );
    out.note::<tables::DivisionEta>(
        frame,
        "nothing changes in this render, which draws each recorded hour and nothing between          them; a moving counter steps from one hour's point to the next where the game glides,          and closing that would mean extrapolating along the route from this table's remaining          time and saying on the frame that the position between hours is extrapolated",
    );
    out.note::<tables::TaskForces>(
        frame,
        "naval counters cover only the recorded task forces, and a naval base holding one of the \
         missing rows is drawn empty",
    );
    out.note::<tables::Ships>(
        frame,
        "a naval counter's ship count and bars cover only the recorded ships",
    );
    out.note::<tables::LandCombats>(
        frame,
        "a land combat among the missing rows carries no combat icon",
    );
    out.note::<tables::LandCombatProgress>(
        frame,
        "a land combat among the missing rows shows no progress percentage, and its direction          wedge falls back to the first attacker whose province differs rather than the attacker          the engine points at",
    );
    out.note::<tables::NavalCombats>(
        frame,
        "a naval combat among the missing rows carries no combat icon",
    );
    out.note::<tables::StateBuildings>(
        frame,
        "an air base among the missing rows carries no base icon",
    );
    out.note::<tables::ProvinceBuildings>(
        frame,
        "a naval base among the missing rows carries no base icon",
    );
    out.note::<tables::AirBases>(
        frame,
        "an air base among the missing rows carries no wing count",
    );
    out.note::<tables::AirWings>(
        frame,
        "an air base's wing count covers only the recorded wings",
    );
    out.note::<tables::LexicalTokens>(
        frame,
        "a base icon whose building token is among the missing rows is not drawn at all",
    );
    // weather
    out.note::<tables::WeatherLimits>(
        frame,
        "the snow texture stays at zero because this hour records no snow ceiling",
    );
    out.note::<tables::WeatherProvinces>(
        frame,
        "the snow and mud texture stays at zero for every province among the missing rows",
    );
    out
}

fn typed<T: TableSchema>(rows: &TableRows, hour: Hour) -> io::Result<&[T::Row]> {
    rows.rows::<T>().ok_or_else(|| {
        invalid(format!(
            "hour {hour} {:?} contains another table's rows",
            T::ID
        ))
    })
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay::{
        frame::Metrics,
        rows::{PoliticalPartyRow, ProvinceRow, TableId},
    };

    /// A frame whose every table is unavailable, so a test only has to fill in
    /// the ones it cares about.
    fn frame() -> DecodedFrame {
        DecodedFrame {
            branch: 1,
            hour: 60_828_478,
            tables: std::array::from_fn(|_| TableView::Unavailable {
                reason: "the collector had not grown its buffer yet".into(),
            }),
            metrics: Metrics::default(),
        }
    }

    fn province(id: u32) -> ProvinceRow {
        ProvinceRow {
            id,
            state: 10,
            owner: 100,
            controller: 100,
            victory_points: 0,
        }
    }

    #[test]
    fn a_truncated_table_still_hands_over_the_rows_it_reached() {
        let mut frame = frame();
        frame.tables[TableId::PoliticalParties.index()] = TableView::Truncated {
            rows: TableRows::PoliticalParties(vec![PoliticalPartyRow {
                country: 100,
                id: 1,
                token: b"fascism".to_vec(),
                ..Default::default()
            }]),
            missing: 38_015,
        };
        let view = rows::<tables::PoliticalParties>(&frame).unwrap();
        assert_eq!(view.rows().len(), 1);
        assert_eq!(view.rows()[0].token, b"fascism".to_vec());
        assert!(!view.is_complete());
        assert_eq!(view.missing(), Some(38_015));
    }

    #[test]
    fn an_unavailable_table_hands_over_no_rows_and_no_count() {
        let empty = frame();
        let view = rows::<tables::Divisions>(&empty).unwrap();
        assert!(view.rows().is_empty());
        assert!(!view.is_complete());
        assert_eq!(view.missing(), None);
        // A complete table is not a gap at all.
        let mut whole = frame();
        whole.tables[TableId::Divisions.index()] =
            TableView::Complete(TableRows::Divisions(Vec::new()));
        let view = rows::<tables::Divisions>(&whole).unwrap();
        assert!(view.is_complete());
        assert_eq!(view.missing(), Some(0));
    }

    #[test]
    fn a_table_holding_another_tables_rows_is_a_named_error() {
        let mut frame = frame();
        frame.tables[TableId::Divisions.index()] =
            TableView::Complete(TableRows::Provinces(vec![province(1)]));
        let error = rows::<tables::Divisions>(&frame).err().unwrap().to_string();
        assert!(
            error.contains("hour 60828478 Divisions contains another table's rows"),
            "{error}"
        );
    }

    #[test]
    fn a_core_table_gap_names_the_table_the_count_and_why_it_is_fatal() {
        let mut frame = frame();
        frame.tables[TableId::Provinces.index()] = TableView::Truncated {
            rows: TableRows::Provinces(vec![province(1)]),
            missing: 1174,
        };
        let error = core::<tables::Provinces>(&frame).err().unwrap().to_string();
        assert!(
            error.contains("hour 60828478 Provinces is truncated (1174 missing rows)"),
            "{error}"
        );
        assert!(error.contains(CORE), "{error}");
        assert!(error.contains("cannot be drawn"), "{error}");
        // An unavailable core table keeps the collector's own reason.
        let error = core::<tables::Countries>(&frame).err().unwrap().to_string();
        assert!(error.contains("Countries is unavailable"), "{error}");
        assert!(
            error.contains("the collector had not grown its buffer yet"),
            "{error}"
        );
        // A complete core table is handed over unchanged.
        frame.tables[TableId::Provinces.index()] =
            TableView::Complete(TableRows::Provinces(vec![province(1), province(2)]));
        assert_eq!(core::<tables::Provinces>(&frame).unwrap().len(), 2);
    }

    #[test]
    fn a_limitation_names_its_table_and_its_missing_count() {
        let mut frame = frame();
        frame.tables[TableId::ProvinceBuildings.index()] = TableView::Truncated {
            rows: TableRows::ProvinceBuildings(Vec::new()),
            missing: 1174,
        };
        let mut limitations = Limitations::default();
        limitations.note::<tables::ProvinceBuildings>(&frame, "a naval base carries no icon");
        limitations.note::<tables::LandCombats>(&frame, "no combat icon is drawn");
        // A complete table contributes no line at all.
        frame.tables[TableId::Ships.index()] = TableView::Complete(TableRows::Ships(Vec::new()));
        limitations.note::<tables::Ships>(&frame, "never reported");
        assert_eq!(
            limitations.lines(),
            [
                "Table ProvinceBuildings truncated (1174 missing rows): a naval base carries no icon".to_owned(),
                "Table LandCombats unavailable: the collector had not grown its buffer yet: no combat icon is drawn".to_owned(),
            ]
        );
        assert!(Limitations::default().lines().is_empty());
    }

    /// A complete table whose field is empty is not a table gap, so it carries
    /// its own wording and names the field rather than the table.
    #[test]
    fn an_empty_recorded_field_names_the_field_and_not_a_table() {
        let mut limitations = Limitations::default();
        limitations.note_field(
            "Countries.name",
            "1 country (SIA) records no name, so its label is built from the localisation key",
        );
        assert_eq!(
            limitations.lines(),
            ["Field Countries.name: 1 country (SIA) records no name, so its label is built from \
              the localisation key"
                .to_owned()]
        );
    }

    /// Every counter on the frame depends on the perspective, so it is stated
    /// whether or not anything degraded — and an omniscient frame says why.
    #[test]
    fn the_perspective_is_stated_on_every_frame_and_names_the_country() {
        let mut recorded = Limitations::default();
        recorded.note_perspective(&Perspective::Country {
            tag: "GER".into(),
            requested: false,
        });
        assert!(
            recorded.lines()[0].starts_with("Perspective GER: the map is drawn as the recorded")
        );
        assert!(recorded.lines()[1].starts_with("Province visibility is not recorded"));

        let mut asked = Limitations::default();
        asked.note_perspective(&Perspective::Country {
            tag: "SOV".into(),
            requested: true,
        });
        assert!(asked.lines()[0].contains("rather than as the recording's own player does"));

        let mut open = Limitations::default();
        open.note_perspective(&Perspective::Omniscient {
            reason: "table FactionMembers is unavailable for this hour".into(),
        });
        assert_eq!(open.lines().len(), 1);
        assert!(open.lines()[0].contains("table FactionMembers is unavailable"));
        assert!(open.lines()[0].contains("every stack is drawn in full"));
    }

    #[test]
    fn the_registry_reports_every_degraded_table_and_no_core_one() {
        let lines = limitations(&frame()).lines().join("\n");
        for table in [
            "StateFlags",
            "States",
            "ProvinceNames",
            "Politics",
            "PoliticalParties",
            "Players",
            "HumanControllers",
            "WarRelations",
            "FactionMembers",
            "Subjects",
            "UnitIntel",
            "Divisions",
            "TaskForces",
            "Ships",
            "LandCombats",
            "NavalCombats",
            "StateBuildings",
            "ProvinceBuildings",
            "AirBases",
            "AirWings",
            "LexicalTokens",
            "WeatherLimits",
            "WeatherProvinces",
        ] {
            assert!(
                lines.contains(&format!("Table {table} unavailable")),
                "{table} is missing from the registry"
            );
        }
        // The core tables are frame errors, not limitations.
        for table in ["Table Provinces", "Table CountryLinks", "Table Countries"] {
            assert!(!lines.contains(table), "{table} must not be a limitation");
        }
        // An hour that holds everything reports nothing.
        let mut whole = frame();
        for table in TableId::ALL {
            whole.tables[table.index()] = TableView::Complete(TableRows::empty(table));
        }
        assert!(limitations(&whole).lines().is_empty());
    }
}

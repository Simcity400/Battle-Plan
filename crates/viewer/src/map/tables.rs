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
    pub fn note_aircraft(&mut self, frame: &DecodedFrame) {
        self.note::<tables::WingEquipment>(
            frame,
            "aircraft with missing equipment identity are not drawn",
        );
        self.note::<tables::VariantDefinitions>(
            frame,
            "aircraft with missing variant definitions are not drawn",
        );
        self.note_field("Flying aircraft", "Generated from seed 0x41495250 and recorded AirWings/AirBases/WingEquipment/VariantDefinitions. UNVERIFIED: mission eligibility and sortie-effective counts, hostile air-intel estimates, scenario roster/outcome/phase, designer/graphical/cosmetic overrides and airborne camera/fade. The port uses recorded effective_planes, independent default mission scenarios and integer province centroids/authored animated formations; no dogfight pairing or particle/death outcomes. Single-CAS attachment slot alias and modded NAirGfx Lua overrides remain UNVERIFIED. Paused animation speed .22; dump clock zero; window clock advances. Carrier/missile basing is skipped. A draw-bearing scenario census and height sweep would settle count, placement and gate; frame33840 contains no airborne PDX draws.");
    }

    pub fn note_trains(&mut self, frame: &DecodedFrame) {
        self.note::<replay::rows::tables::RailEdges>(frame, "train routes unavailable");
        self.note::<replay::rows::tables::RailCooldowns>(frame, "train routes unavailable");
        self.note::<replay::rows::tables::ProvinceSupply>(frame, "train hub locations unavailable");
        self.note_field("Supply trains", "UNVERIFIED reconstruction: no SupplyNodes table exists. Supply companion nodes (+0xb8, stride 0xe8: kind +8, flag +0xc, available +0x38, consumed +0x40), selected parent routes, related-controller access bitmap and CCountrySupplySystem +0x120 allocated train inventory are missing. Routes use recorded RailEdges/RailCooldowns and ProvinceSupply hubs with shortest paths and equal hub weights; equipment_1 substitutes for the missing equipment mix, with no inventory cap. The engine's camera gate, speed, country wagon lists and scales are reproduced. Phase uses seed 0x54524149 XOR the ordered route provinces, a global 9200..12000 ms departure schedule and the frame animation clock (dump=0). Exact count/position modulo phase remains unverified until those simulation inputs are recorded.");
    }

    /// Entity light state is a field omission, even when every table is whole.
    pub fn note_point_lights(&mut self) {
        self.lines.push(
            "Point lights: transient entity emissions are absent. LandCombats and NavalCombats \
         do not record the entity event clock, effect lifetime, flicker phase, random seed, \
         selected model or animated muzzle/attachment transform. Bombing and nuclear events \
         are not reconstructed from hourly totals. Citylight's enable gate and entity-light \
         visibility are UNVERIFIED; an associated entity/light timeline would settle them. \
         The two retained RenderDoc captures select only zero-colour lights; their nonzero \
         data rows are stale and unreferenced. Terrain city emissive masks remain independent."
                .into(),
        );
    }

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

    // orders: the recorded player's battle plan
    /// Records what the drawn battle plan cannot take from the recording.
    ///
    /// These are not table gaps --- `ArmyOrders` and `OrdersGroups` can be
    /// complete and every one of them still holds --- so they are fields rather
    /// than tables, and they are stated only on an hour that draws a plan at
    /// all, because on an hour with none they describe nothing on screen.
    pub fn note_battle_plan(&mut self, drawn: usize) {
        if drawn == 0 {
            return;
        }
        self.note_field("ArmyOrderPlans (geometry)", "unbound snapped province-pair caches and graph traversal priority are not recorded; unbound lines use province-centroid/crossing curves. Child-count front offsets and overlap ordering outside the captured single-army plan remain unverified");
        self.note_field("ArmyOrderPlans (presentation)", "deselected, unexecuted presentation; order text, execution animation, selection and terrain-detail normal distortion are not drawn");
    }

    /// Records that this frame sits between two recorded hours.
    ///
    /// `fraction` is how far past the recorded hour the frame is, in hours. It
    /// is never zero here: a recorded hour draws its recorded positions and has
    /// nothing to state. The line is written once for the whole frame rather
    /// than marked on any counter, because the game marks none and the frame is
    /// meant to be indistinguishable from the game's.
    pub fn note_extrapolated_positions(&mut self, hour: Hour, fraction: f32) {
        self.lines.push(format!(
            "Between recorded hours: this frame is {fraction:.3} h past recorded hour {hour}, so every moving division's counter and close-zoom model is extrapolated along its recorded route by its recorded DivisionEta remaining time. Recorded hours themselves are drawn exactly, and a division with no remaining time recorded does not move at all",
        ));
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
            }
            Perspective::Omniscient { reason } => self.lines.push(format!(
                "Perspective omniscient: {reason}, so every stack is drawn in full, including the \
                 ones the game hid behind a question mark, and no fog of war is drawn over the \
                 land no country has uncovered",
            )),
        }
    }

    /// Records what the drawn fog of war is a snapshot of.
    ///
    /// Called only when the frame draws fog, because the three ways it can fail
    /// to are already stated: the table's own note, [`Limitations::note_viewpoint`]
    /// and the omniscient branch of [`Limitations::note_perspective`].
    ///
    /// `fraction` is how far past the recorded hour the frame sits, and it is
    /// the between-hours term. The client's visibility buffer is rebuilt by the
    /// graphical update rather than at an hour boundary, the recording samples
    /// it once an hour, and nothing interpolates it, so a frame drawn between
    /// two hours carries the earlier hour's fog under the later hour's
    /// divisions.
    pub fn note_fog_of_war(&mut self, fogged: usize, fraction: f32) {
        let between = if fraction > 0.0 {
            format!(
                " This frame sits {fraction:.3} h past its recorded hour and the visibility is \
                 not interpolated, so the fog is the recorded hour's while the divisions standing \
                 in it have moved on."
            )
        } else {
            String::new()
        };
        self.lines.push(format!(
            "Fog of war: {fogged} intel texels are fogged from the hour's recorded \
             ProvinceVisibility, which is the client's own presentation buffer sampled once an \
             hour. The engine's FoWTime epoch is not recorded: the window animates the three \
             noise octaves on its own frame clock and a dump samples time zero. The pattern \
             therefore differs from the instant a screenshot caught it. The engine also \
             gates FOWFadeFactor on map mode: the fill's controller and owner modes are the engine's \
             mode 0, which is inside the set that draws fog, so the gate is open here.{between}",
        ));
    }

    /// Records that the hour's recorded province visibility answers for
    /// another country than the one the map is drawn for.
    ///
    /// This is not a table gap either: the table is complete and every byte in
    /// it is real. It is simply another client's buffer, and one country's
    /// uncovered provinces are not another's, so the wording says whose it is
    /// rather than borrowing it.
    pub fn note_viewpoint(&mut self, recorded: &[String], ours: &str) {
        self.lines.push(format!(
            "Field ProvinceVisibility.viewing_country: the recorded bytes answer for {}, not \
             {ours}, and one country's uncovered provinces are not another's, so they are left \
             out of the engine's max and a land stack is decided by the UnitIntel ladder alone; a \
             stack standing in a province {ours} has uncovered carries no counter here where the \
             game drew its question mark, or a question mark where the game drew it in full. The \
             map draws no fog of war either, for the same reason: the intel texture the lit passes \
             fog with is built from these bytes and nothing else",
            recorded.join(", ")
        ));
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
         of them carries the game's question mark where the game drew it in full; subject colour \
         inheritance among the missing rows is unavailable, so those countries keep their own \
         authored colour",
    );
    out.note::<tables::Autonomy>(
        frame,
        "current subject colour switches among the missing rows are unavailable; those subjects keep their own authored colour",
    );
    out.note::<tables::UnitIntel>(
        frame,
        "a division or task force among the missing rows keeps the detection level 0 the engine \
         gives an absent entry, so a stack this perspective had detected is drawn with the game's \
         question mark",
    );
    out.note::<tables::ProvinceVisibility>(
        frame,
        "a land stack is decided by the UnitIntel detection ladder alone, and the map draws no fog \
         of war at all. The engine takes the greater of that ladder and the viewing country's own \
         visibility byte for the province the stack stands in, and a province among the missing \
         rows contributes no byte rather than a zero, so a stack standing in a province this \
         perspective has uncovered carries no counter here where the game drew its question mark, \
         or a question mark where the game drew it in full. The same bytes are the whole input to \
         the engine's intel texture, which every lit map pass fogs with, so without them the land \
         this perspective has not uncovered is drawn as clear as the land it holds, and none of it \
         is inferred from unit positions, radar or supply",
    );
    // counters
    out.note::<tables::Divisions>(
        frame,
        "land counters count only the recorded divisions, and a land combat's attack direction may \
         be missing",
    );
    out.note::<tables::DivisionTemplates>(
        frame,
            "a land counter whose division's template is among the missing rows draws no unit-type icon, and its dark overlay panel is left bare; question-mark counters with missing template rows retain separate source plates because their grouping keys are unknown; at close zoom its division model falls back to rifle infantry, because with no template there is no sub-unit key for the engine's entity ladder to start from",
    );
    out.note::<tables::DivisionStatus>(
        frame,
        "a land counter's organisation and strength bars fall back to full for every division          among the missing rows, and its supply, attrition, entrenchment, exercise and no-order          glyphs are not drawn",
    );
    out.note::<tables::DivisionMovement>(
        frame,
            "a division among the missing rows counts as standing still: its counter and its close-zoom model sit on its province's own anchor instead of along its route, the model faces a hash of the province instead of along the route, its supply crate is not suppressed for a convoy crossing, and a division the row would have shown in naval transfer draws its land figure at full size rather than a quarter-scale transport",
    );
    out.note::<tables::DivisionEta>(
        frame,
        "a division among the missing rows does not glide: between two recorded hours its counter and its close-zoom model stay on the hour's own recorded point and then step to the next hour's, where a division with a remaining time advances along its recorded route by it",
    );
    out.note::<tables::TaskForces>(
        frame,
        "naval counters cover only the recorded task forces, and a naval base holding one of the \
         missing rows is drawn empty",
    );
    out.note::<tables::Ships>(
        frame,
        "a naval counter's ship count covers only the recorded ships, where the game counts every          ship in the task force's own array",
    );
    if rows::<tables::Ships>(frame).is_ok_and(|ships| !ships.rows().is_empty()) {
        out.note_field(
            "Ships (at-sea models)",
            "at-sea hull models are not drawn: the ship-to-equipment-variant link, hull sprite, model \
         number and formation-selection flags are not recorded. Ships.category is a coarse class \
         enum and class_id names a class-history entry, so neither supplies the engine's entity \
         selector and no substitute hull is drawn. Stationary port formations are hidden by the \
         engine itself; their absence is not this recording limitation. The port's static \
         building mesh and its counter are drawn by their existing passes",
        );
    }
    // orders: the recorded player's battle plan
    out.note::<tables::ArmyOrderPlans>(
        frame,
        "battle-plan binding, style and assignment; absent plans are not drawn",
    );
    out.note::<tables::OrdersGroupColors>(
        frame,
        "battle-plan colour; absent colours are not guessed",
    );
    out.note::<tables::Fronts>(frame, "bound-front geometry; absent fronts are not drawn");
    out.note::<tables::ArmyOrders>(
        frame,
        "the battle plan covers only recorded orders; missing orders are not drawn",
    );
    out.note::<tables::OrdersGroups>(
        frame,
        "an order whose owning group is missing is not drawn; the army-group style is not guessed",
    );
    out.note::<tables::LandCombats>(
        frame,
        "a land combat among the missing rows carries no combat icon",
    );
    out.note::<tables::LandCombatProgress>(
        frame,
        "a land combat among the missing rows shows no progress percentage, and its direction          wedge falls back to the first attacker whose province differs rather than the attacker          the engine points at",
    );
    out.note::<tables::AirBaseRegistrations>(
        frame,
        "an air base among the missing rows shows no plane count, because the recorded          registrations are what says whether the viewing country is shown one at all",
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
    out.note::<tables::WeatherRegions>(
        frame,
        "weather effects are drawn only for the recorded region rows",
    );
    if rows::<tables::WeatherRegions>(frame).is_ok_and(|v| {
        v.rows().iter().any(|r| {
            [r.light_rain, r.heavy_rain, r.snow, r.blizzard, r.sandstorm]
                .iter()
                .any(|v| *v != 0)
        })
    }) {
        out.note_field("weather particle presentation", "WeatherRegions selects the game's authored emitters and textures at map/weatherpositions.txt anchors. Particle states, seed and elapsed time are not recorded: this presentation uses a deterministic seed per region and hour and the window animation clock (dump time 0), initialized with stationary birth phases. UNVERIFIED: variation distribution, curve interpolation and turbulence dynamics require particle-update decompilation or consecutive captured instance buffers. Weather assets are outside hash-verified content coverage.");
    }
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
        // Province visibility is a recorded table now, so its absence is the
        // registry's line and not a sentence every perspective carries.
        assert_eq!(recorded.lines().len(), 1);

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

    /// A complete table whose bytes were recorded for somebody else is not a
    /// table gap and not a field the recording left empty, so it carries its
    /// own wording and names both countries.
    #[test]
    fn visibility_recorded_for_another_country_names_it_and_names_ours() {
        let mut limitations = Limitations::default();
        limitations.note_viewpoint(&["ENG".to_owned()], "GER");
        let line = &limitations.lines()[0];
        assert!(
            line.starts_with(
                "Field ProvinceVisibility.viewing_country: the recorded bytes answer for ENG, not \
                 GER"
            ),
            "{line}"
        );
        assert!(line.contains("UnitIntel ladder alone"), "{line}");
    }

    /// An older recording has no visibility at all, and the line it produces
    /// has to name the format that did not record it, because that is the one
    /// thing the reader can act on: a newer recording of the same campaign.
    #[test]
    fn a_format_that_predates_visibility_names_the_format_in_its_line() {
        let mut frame = frame();
        frame.tables[TableId::ProvinceVisibility.index()] = TableView::Unavailable {
            reason: "not recorded by format 14".into(),
        };
        let lines = limitations(&frame).lines().join("\n");
        assert!(
            lines.contains(
                "Table ProvinceVisibility unavailable: not recorded by format 14: a land stack is \
                 decided by the UnitIntel detection ladder alone, and the map draws no fog of war \
                 at all."
            ),
            "{lines}"
        );
        frame.tables[TableId::ProvinceVisibility.index()] = TableView::Unavailable {
            reason: "not recorded by format 17".into(),
        };
        assert!(
            limitations(&frame)
                .lines()
                .join("\n")
                .contains("not recorded by format 17"),
        );
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
            "Autonomy",
            "UnitIntel",
            "ProvinceVisibility",
            "Divisions",
            "TaskForces",
            "Ships",
            "LandCombats",
            "NavalCombats",
            "StateBuildings",
            "ProvinceBuildings",
            "AirBases",
            "AirBaseRegistrations",
            "AirWings",
            "LexicalTokens",
            "WeatherRegions",
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

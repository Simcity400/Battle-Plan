//! Reading one decoded hour's tables when the hour does not hold all of them.
//!
//! The collector grows its buffers over the first hours of a session, so an
//! early frame records several tables truncated and a staggered table not at
//! all. The viewer shows gaps, truncated tables and unavailable tables **as
//! such** rather than refusing the hour, so a
//! pass reads what the hour holds through `rows` and states what it could not
//! draw through `Limitations`.
//!
//! # Core and the rest
//!
//! Three tables are **core**: `Provinces` carries the per-province owner,
//! controller and state the political fill *is*, and `CountryLinks` and
//! `Countries` turn its raw country references into the canonical slots and
//! tags every colour, flag and name is keyed by. Without all three there is no
//! political map to degrade, so [`core`] refuses the frame with a message that
//! names the table and says so. Every other table the map reads degrades: see
//! `limitations` for the full list and what each gap costs.

use std::io;

use replay_data::{rows::TableSchema, rows::tables};
use replay_frame::{frame::DecodedFrame, frame::TableView};

use super::perspective::Resolution;

/// The core tables, named in the error a missing one produces.
const CORE: &str = "Provinces, CountryLinks and Countries";

pub use crate::table_access::{Available, rows};

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
        Available::NotRecorded { mode } => Err(invalid(format!(
            "hour {hour} {table:?} is not recorded in a {mode} recording; the core tables {CORE} \
             carry the political map itself, so this hour cannot be drawn"
        ))),
    }
}

/// What one hour could not draw, in the words the window and a dump both show.
///
/// One line per degraded table, in the fixed order of [`limitations`], so the
/// same hour always reports the same list.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Limitations {
    lines: Vec<String>,
}

impl Limitations {
    fn push(&mut self, line: String) {
        self.lines.push(line);
    }

    pub fn note_aircraft(&mut self, frame: &DecodedFrame) {
        self.note::<tables::WingEquipment>(
            frame,
            "aircraft with missing equipment identity are not drawn",
        );
        self.note::<tables::VariantDefinitions>(
            frame,
            "aircraft with missing variant definitions are not drawn",
        );
        self.note::<tables::LexicalTokens>(
            frame,
            "aircraft whose equipment type token is among the missing rows are not drawn",
        );
        self.note_field("Flying aircraft", "Wings fly in the recorded mission registration order, for the viewpoint's participant arrays built from the recorded donor, wartime-ally and enemy vectors in native order (AirCountryRelations, sliced by country and emitted whenever a vector changes) and its volunteer hosts; without that complete table and VolunteerHosts only the viewpoint's own wings fly. Wings fill the native per-side arrays of 50 in visit order; the game then draws from each at random with a wall-clock seed, the viewer in order with a fixed seed. Sizing, limits, curves, take-off scale and clock come from NAirGfx. Admission and counts follow mission eligibility and count rules on the recorded mission, base-container, modifier and host-base operands. Scenarios anchor at the mission's last target (bombers, transports), a seeded draw from the recorded reached provinces (scouts, the viewer/A side's fighters) or from the region's provinces (B-side fighters), at integer centroids, with seed 0x41495250 and the authored animated formations; naval-patrol fighters fly the kamikaze scenarios at the province and scenario outcome selects from their country's task forces in naval combat and the viewpoint's regional air history. Opposed regions pair bombers with the other side's fighters and fighters with fighters (dogfights), drawn from the arrays with the same fixed seed; each pair's outcome selects the scenario's authored outcome variant, with its states and particle events, from the viewpoint's first-window regional air history (AirNavalPatrolHistories), and a region without that history omits its opposed and dependent transport scenarios (counted). A region whose bomber and fighter arrays are all empty flies the viewpoint's own training wings based there instead, the first 50 in the recorded pool order (AirWings.pool_ordinal), as planes / NAirGfx.TRANSPORT_DIVISION_FACTOR fighter patrols under MAX_TRAINING_SCENARIOS from one seeded region province each; the game draws them at random, the viewer in wing-ID order; a full region beside a training wing whose pool base does not resolve is counted; a region where a wing that could fill those arrays is undecided does not train (counted). Omitted: naval strike and patrol wings whose only undecided target is an embarked railway gun, whose transfer, combat and owner are not recorded (the diagnostics count them); strategic bombing wings whose only undecided target is the strategic railway-gun branch, which is not recorded either. Base-equipment wings follow the scouts with MISSILES_DIVISION_FACTOR and MAX_MISSILE_BOMBING_SCENARIOS and fly missile bombing scenarios to their mission's last target; a rocket without its attachment node rides the scenario root at ROCKET_SPEED; slots without a landing route (the rocket, successful kamikazes, defenders of a successful interception or dogfight, attackers of a lost dogfight) meet the near-ground band (AIRPLANES_GROUND_COLLISION_OFFSET_Y, the entity's explode state for AIRPLANES_GROUND_EXPLOSION_TIME_DELAY), and a missile scenario ends, with the content's missile explosion entity at the province's standstill anchor, where its rocket reaches the terrain. Bit-17 missions and launch-flagged definitions start launch effects at their bases instead: one record per base province, the primary equipment's entity after a RANDOM_SCENARIO_ANIMATION_MAX_DELAY draw, and for the launch flag a missile explosion in a drawn province of the mission region; every state base stands where the game's base position finder puts it, through the content's building types, and the drawn transforms ease toward their poses by AIRPLANES_SMOOTH_INTERPOLATION_MOVE and _TURN. The game's delay, heading and province draws are wall-clock random; the viewer's are seeded. A carrier wing takes off from its task force's settled route point. Unrecorded or uncertain: hostile air-intel estimates, scenario phase, designer/graphical/cosmetic overrides, airborne camera/fade, the single-CAS attachment slot alias. Dump clock zero; window clock advances.");
    }

    pub fn note_trains(&mut self, frame: &DecodedFrame) {
        self.note::<replay_data::rows::tables::ProvinceSupply>(
            frame,
            "railway supply-source spurs and port anchors unavailable",
        );
        self.note::<replay_data::rows::tables::RailEdges>(
            frame,
            "railway strips, bridges and train routes unavailable",
        );
        self.note::<replay_data::rows::tables::RailCooldowns>(frame, "train routes unavailable");
        self.note::<replay_data::rows::tables::SupplyNodes>(frame, "supply trains are hidden: node weights and selected parent routes are unavailable; no shortest-path substitute is used");
        self.note::<replay_data::rows::tables::TrainInventory>(frame, "supply trains are hidden: allocated variants, fleet cap and controller access are unavailable");
        self.note::<replay_data::rows::tables::VariantDefinitions>(
            frame,
            "supply trains are hidden: allocated variant model keys are unavailable",
        );
        self.note::<replay_data::rows::tables::LexicalTokens>(
            frame,
            "a supply train whose equipment type token is among the missing rows draws no consist",
        );
        self.note_field("Supply train phase", "The scheduler seed/counter, remaining interval and existing train fleet are unrecorded. Presentation uses seed 0x54524149 XOR ordered route provinces, a global 9200..12000 ms schedule and animation time (dump=0). Routes/weights/access and signed ordered inventory are recorded in format 21, each country's once a game day at its owner slice (its read hour is in SliceReads), so a route, weight or fleet can be up to a day old. Rail spline anchors, continent wagon fallback, wheel animation and exhaust remain unverified. A null or missing variant join suppresses that consist; the engine's unrecorded first-owned-train fallback is not invented.");
    }

    /// Entity light state is a field omission, even when every table is whole.
    pub fn note_point_lights(&mut self) {
        self.push(
            "Entity animation: window clocks sample authored states, blends, attachments and muzzle/cigarette light and particle events. \
         DivisionMovement and LandCombats select movement and combat states; missing movement rows cannot identify marching units. \
         Seed is 0x414e494d XOR the unit handle. Unrecorded: original state-entry times, shared RNG sequence, initial combat phase, \
         entity visibility and particle dynamics lack complete event timelines. Dump time zero retains the default first pose and no events. \
         A nuclear impact (NuclearStrikes) plays its type's effect entity at the province's standstill anchor from the moment its hour is presented; \
         the impact's instant inside the hour is unrecorded. Strategic bombing shows through the bombing air scenarios and building damage states, \
         as in the game. The game never attaches the building citylight light, so none is drawn; night city lights are the terrain shader's. \
         A critical shore bombardment (MapEvents) plays the game's small missile explosion entity in its province's effect slot from the moment its hour is presented.          A missile bombing scenario of the flying aircraft plays the content's missile explosion entity in its province's effect slot when it finishes, on the aircraft clock."
                .into(),
        );
    }

    /// Records `affected` when this hour's `T` table is not complete.
    ///
    /// `affected` is what the map draws differently because of the gap, stated
    /// in the present tense: it is read by someone looking at the render.
    pub fn note<T: TableSchema>(&mut self, frame: &DecodedFrame, affected: &str) {
        let table = T::ID;
        match &*frame.tables[table.index()] {
            TableView::Complete(_) => {}
            TableView::Truncated { missing, .. } => self.push(format!(
                "Table {table:?} truncated ({missing} missing rows): {affected}"
            )),
            TableView::Unavailable { reason } => {
                self.push(format!("Table {table:?} unavailable: {reason}: {affected}"))
            }
            TableView::NotRecorded { mode } => self.push(format!(
                "Table {table:?} not recorded in a {mode} recording: {affected}"
            )),
        }
    }

    /// Records a limitation of one recorded **field** rather than of a table.
    ///
    /// A row can be present, and its table complete, and the field still carry
    /// nothing a pass can draw: a `ProvinceNames.name` is raw engine bytes that
    /// need not be text at all.
    /// The table is complete, so [`Limitations::note`] has nothing to say about
    /// it, but the pass still fell back and the reader still has to be told
    /// which field it fell back from.
    pub fn note_field(&mut self, field: &str, affected: &str) {
        self.push(format!("Field {field}: {affected}"));
    }

    /// The recorded lines, one per degraded table. An hour that drew
    /// everything it reads has none.
    pub fn lines(&self) -> Vec<String> {
        self.lines.clone()
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
        self.note_field("ArmyOrderPlans (geometry)", "Fronts, snapped offensive borders, one-province lines and move endpoints use the recorded order graph and content neighbour order. Sea-crossing fronts use the native one-hop Through neighbour's border. Offset fronts resubmit through the intersection-tested builder; army-group child sections use recorded spans and the same-pass child count. Paradrops are straight, densified and sine-lifted at native control indices; invasions use the recorded movement world and relation costs. Missing operands and unsupported content retain named refusals");
        self.note_field("ArmyOrderPlans (presentation)", "Plans follow the viewer's Allied battle plans toggle and native admission rules. Long invasion and paradrop labels use recorded simulation operands and content. Active plans use current simulation prerequisites; blocked plans stop texture scrolling while their active effect remains. Missing execution operands retain named refusals. The read-only viewer does not select or edit order groups");
    }

    // raids: the viewing country's raid arrows
    /// Records what the raid arrows cannot take from the recording, when the
    /// viewing country has `raids` non-ended raids, `launched` of them in flight.
    pub fn note_raid_arrows(&mut self, raids: usize, launched: usize) {
        if raids == 0 {
            return;
        }
        self.note_field("Raids (arrows)", "The viewing country's raid arrows derive from the recorded raids and the content: straight, ballistic and air arrows between the source and target anchors with the NGraphics lift callbacks, naval and land arrows along the raid path search's route. A prepared raid (phase 3) is READY or PREPARING from launch eligibility: the unit at its source, the raid's own equipment pool's recorded archetype totals and nukes (RaidEquipment) against the type's essential and additional equipment, then the launch triggers. A raid whose check stays undecided (a withheld table, a missing row, an undecidable trigger) is refused and counted");
        if launched > 0 {
            self.note_field("Raids (unit)", "A launched raid's unit uses progress from RaidRow.travelled and RaidSpeedInputs. The tick's progress is not recorded, so a dump uses its --hour-fraction, a paused window the hour's start, and a playing window its playback fraction, placing the unit again every rendered frame so it glides and reaches each stage at its sub-hour moment. It plays the raid type's unit_animations stage reached at that progress. The entity clock is the viewer's wall clock, advancing entities by each frame's delta whether or not the game runs: a stage reached under a shown unit plays its intro, then the intro's next_state; a first placement, seek or dump shows the stage's state, as a freshly shown entity does. No camera height hides it");
        }
    }

    /// Records the game's two controls: the viewing country and the fog switch.
    ///
    /// Neither is a table gap, so [`Limitations::note`] has nothing to say about
    /// them, but every counter, colour and plan on the frame depends on them and
    /// the reader has to be told which country the map was viewed as — and, when
    /// that is not the country asked for, why. The wording lives here beside
    /// every other degradation's so the window and a dump cannot state different
    /// ones.
    pub fn note_perspective(&mut self, perspective: &Perspective) {
        let tag = &perspective.tag;
        let country = match &perspective.resolution {
            Resolution::Recorded => {
                format!("Viewing country {tag}: the recording client's stored viewpoint")
            }
            Resolution::Requested => format!(
                "Viewing country {tag}: as asked, whoever the recording's own player was, so \
                 allegiance colours, the battle plan, aircraft and what the map shows are {tag}'s"
            ),
            Resolution::NoPlayer => format!(
                "Viewing country {tag}: this hour has no resolved client viewpoint, so the viewer takes \
                 the country in the lowest canonical slot; that choice is this viewer's, not the \
                 game's"
            ),
            Resolution::Absent(asked) => format!(
                "Viewing country {tag}: this hour records no country {asked}, so the map is viewed \
                 as {tag} instead"
            ),
        };
        let fog = if perspective.fog {
            format!(
                "fog of war on, so a stack {tag} has no intelligence on carries the game's own \
                 question mark and no bars, as the game draws it"
            )
        } else {
            format!(
                "fog of war off, the game's fow console switch: every stack is drawn in full and \
                 no fog is drawn, while allegiance, the battle plan, shared vision and the combat \
                 icons' question marks stay {tag}'s"
            )
        };
        self.push(format!("{country}; {fog}"));
    }

    /// Records the sight inputs the fog-on unit byte takes from the hour.
    ///
    /// Only fog of war on reads them: with it off unit intel returns
    /// `0xff` before any of them is asked, so a gap in them costs nothing and
    /// is not stated.
    pub fn note_fog_on_sight(&mut self, frame: &DecodedFrame) {
        self.note::<tables::UnitIntel>(
            frame,
            "with fog of war on, a division or task force among the missing rows keeps the \
             detection level 0 the engine gives an absent entry, so a stack the viewing country had \
             detected is drawn with the game's question mark",
        );
        self.note::<tables::CountryIntel>(
            frame,
            "with fog of war on, an air base icon whose controller the missing rows name keeps \
             the icon: hiding on air intel alone needs the whole table, and a truncated one \
             proves nothing about the lanes it did not reach",
        );
        self.note::<tables::RelationLinks>(
            frame,
            "with fog of war on, an air base icon keeps the icon when access to its controller \
             cannot be decided: hiding on access alone needs the whole table",
        );
        self.note_field(
            "Map objects (fog of war on)",
            "a second building slot kind hides below 150, but no \
             table records the kind, so those slots stay drawn. A slot with no province under its \
             authored position is kept without a byte to judge it by. Hidden counts differ by \
             pass: buildings count every fogged slot, air bases only the on-screen icons",
        );
    }

    /// Records that fog of war is on but a sight table is unavailable, so the
    /// unit byte cannot be decided and every stack is drawn as fog off draws it.
    pub fn note_undecided_sight(&mut self, table: &str) {
        self.push(format!(
            "Fog of war on, but table {table} is unavailable for this hour and an unavailable \
             table is not evidence that it is empty, so every stack is drawn in full, as with fog \
             of war off, including the ones the game hid behind a question mark",
        ));
    }

    /// Records what the drawn fog of war is a snapshot of.
    ///
    /// Called only when the frame draws fog, because the ways it can fail to
    /// are already stated: fog of war off in [`Limitations::note_perspective`],
    /// and with it on [`Limitations::note_visibility_gap`]. The fog is rebuilt
    /// from the recorded hour and not interpolated, so a frame drawn between
    /// two hours carries the earlier hour's fog (`fraction`).
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
        self.push(format!(
            "Fog of war: {fogged} intel texels are fogged, rebuilt for the viewing country from recorded simulation. FoWTime follows the global presentation clock, whose epoch is unrecorded; windows use their frame clock and dumps use zero. FOWFadeFactor is enabled for controller/owner political views. The unrecorded map-panel fog-display toggle is treated as on.{between}",
        ));
    }

    /// Records that fog of war is on but the province fog rebuild lacks an input.
    pub fn note_visibility_gap(&mut self, input: &str) {
        self.push(format!(
            "Fog of war on, but the province fog cannot be rebuilt this hour ({input}): no fog is \
             drawn, map objects are kept, and every land stack is drawn in full, as for an hour \
             missing a unit-sight table, including the ones the game hid behind a question mark; \
             naval stacks are still decided by their detection level"
        ));
    }
}

/// What the game's two controls resolved to for this hour.
///
/// `map::perspective` decides this; the wording of it lives here.
pub struct Perspective {
    /// The viewing country's tag.
    pub tag: String,
    /// How the viewing country was settled.
    pub resolution: Resolution,
    /// Whether fog of war is on.
    pub fog: bool,
}

/// Every non-core table the map reads, and what a gap in it costs this hour.
///
/// This is the one place that list lives: the window and `--dump` both report
/// it, so neither can state a different limitation from the other. A pass is
/// responsible only for degrading; the wording of the degradation is here, next
/// to the wording of every other one.
///
/// The core tables are absent on purpose. A gap in one of them is not a
/// limitation of the render, it is the absence of the render, and `core`
/// names it as a frame error instead.
pub fn limitations(frame: &DecodedFrame) -> Limitations {
    let mut out = Limitations::default();
    if rows::<tables::Provinces>(frame).is_ok_and(|v| !v.rows().is_empty()) {
        out.note_field("Water animation", "The water presentation clock is not recorded. Windows use replay presentation time; dumps start at zero. Water time is separate from global/FOW time and cannot be recovered from the simulation hour.");
    }
    // borders: the demilitarised-zone bucket bank
    out.note::<tables::StateFlags>(
        frame,
        "the demilitarised-zone border is not drawn at all: a state this hour is silent about \
         cannot be assumed not to be demilitarised, and half a zone is a wrong outline rather than \
         a short one; with the table truncated a state among the missing rows shows no air-base \
         plane-count box, while a recorded state reads its own row; an unavailable table \
         hides every plane-count box — report-only, the count box is \
         never drawn outside the air map mode",
    );
    out.note::<tables::States>(
        frame,
        "the demilitarised-zone border is not drawn at all: the recorded flags are joined to this \
         table's state ids, and a state it is silent about cannot be shown to exist; a capital \
         also carries no star without its state's recorded controller",
    );
    out.note::<tables::CountryAppearance>(
        frame,
        "a controller with no recorded colour is filled grey, its name shows its tag, \
         and a counter flag falls back from its cosmetic tag",
    );
    out.note::<tables::ArmyIconInputs>(frame, "missing division rows use the standard template and recover only HQ-provider separation from OrdersGroups; alternate template grouping is unavailable");
    // names
    out.note::<tables::ProvinceNames>(
        frame,
        "a victory point renamed in game keeps the static localised name of its province, because \
         a province this hour is silent about cannot be assumed not to have been renamed",
    );
    out.note::<tables::Politics>(
        frame,
        "country names show their tag, counter flags fall back to the plain <TAG> key, and a \
         capital carries no star",
    );
    out.note::<tables::PoliticalParties>(
        frame,
        "country names show their tag and counter flags fall back to the plain <TAG> key",
    );
    // counters: allegiance
    out.note::<tables::LocalPlayer>(
        frame,
        "the recording client's viewpoint is unavailable; the default view uses the lowest country slot",
    );
    out.note::<tables::WarRelations>(
        frame,
        "no country is coloured as an enemy and the hour is drawn as if no war were running; a war \
         is also what stops a faction partner from sharing vision, so a combat icon can show a \
         percentage the game hid; with fog of war on, an unavailable table leaves every stack drawn \
         in full, and a truncated one can only show more than the game did, never less",
    );
    out.note::<tables::FactionMembers>(
        frame,
        "no country is coloured as an ally, and counters are grouped as if no faction existed; a \
         partner among the missing rows also shares no vision, so its combats keep the game's \
         question mark and its visible stacks lose their template picture; with fog of war on, a \
         truncated table draws its stacks with the question mark and an unavailable one draws \
         every stack in full",
    );
    out.note::<tables::Subjects>(
        frame,
        "an overlord and its subject among the missing rows share no vision, so a combat icon \
         between them keeps the game's question mark and, with fog of war on, one of them carries \
         the game's question mark where the game drew it in full; subject colour inheritance \
         among the missing rows is unavailable, so those countries keep their own authored colour",
    );
    out.note::<tables::Autonomy>(
        frame,
        "current subject colour switches among the missing rows are unavailable; those subjects keep their own colour and their names fall back to their tags",
    );
    // counters: co-belligerency. `Relations::read` needs all four tables complete; without
    // them the counter pass knows no side but the viewing country's own.
    const COUNTER_RELATIONS: &str = "unit counters take no side but the viewing country's own,         and different owners never share a counter column, because the co-belligerent byte the         grouping reads is derived from the complete war, faction, subject and relation tables";
    out.note::<tables::WarRelations>(frame, COUNTER_RELATIONS);
    out.note::<tables::FactionMembers>(frame, COUNTER_RELATIONS);
    out.note::<tables::Subjects>(frame, COUNTER_RELATIONS);
    out.note::<tables::RelationLinks>(frame, COUNTER_RELATIONS);
    out.note::<tables::ProvinceUnitOrder>(frame, "units are offered to the counter first-fit in cold handle order within each province bucket, and port occupant order is unknown; merges and plate order that depend on which unit came first can differ from the game");
    out.note::<tables::NavalBases>(
        frame,
        "a port with no row draws no base icon because its country and access list are unknown",
    );
    out.note::<tables::NavalProvinceCombats>(frame, "naval combat icons in sea provinces lose their stack order, spill-over icons are not drawn and the progress size is undecided where two combats of a province might be engaged: the provinces' combat vectors are unknown");
    out.note::<tables::NavalCombatTransfers>(frame, "naval combat icons that spill onto a transferred unit's sea province are not drawn: the combats' transfer vectors are unknown");
    out.note::<tables::NavalTransferUnits>(frame, "naval combat icons that spill onto a transferred unit's sea province are not drawn: the transfers' unit vectors are unknown");
    const RESULTS: &str = "retained naval result icons are not drawn: every visible result of a province decides its sprite and frame";
    out.note::<tables::NavalResults>(frame, RESULTS);
    out.note::<tables::NavalResultMembers>(frame, RESULTS);
    out.note::<tables::NavalResultCountries>(frame, RESULTS);
    if let Ok(bases) = rows::<tables::NavalBases>(frame) {
        let unreadable = bases
            .rows()
            .iter()
            .filter(|base| base.countries_unreadable)
            .count();
        if unreadable > 0 {
            out.note_field(
                "Naval base access lists",
                &format!(
                    "{unreadable} naval bases recorded an unreadable country list; where the viewing country knows the port (its own, or at peace with military access, docking rights, a faction or an overlord-subject tie to its country) the icon's permission glyph needs the list, so it is not drawn and the diagnostics count it by reason"
                ),
            );
        }
    }
    out.note::<tables::TaskForceMissions>(frame, "naval mission map icons are not drawn: an incomplete mission census cannot give a region's registrations");
    out.note::<tables::FloatingHarborDeployments>(frame, "a floating harbour whose province carries the temporary-supply bit is not drawn: its controller's deployment records are unknown");
    out.note::<tables::FacilityMapFlags>(frame, "a program province with no row draws no special-facility icon: the recording client's map flag is unknown there");
    out.note::<tables::Raids>(
        frame,
        "raid icons are withheld while the raid census is incomplete",
    );
    out.note::<tables::RaidTargets>(
        frame,
        "raid icons are withheld while the raid targets are incomplete",
    );
    out.note::<tables::RaidCountries>(
        frame,
        "raid icons are withheld while the countries' raid statuses are incomplete",
    );
    out.note::<tables::RaidTargetStatuses>(frame, "raid offers are withheld while the target statuses are incomplete, and foreign need_detection buildings stay hidden");
    out.note::<tables::RaidCooldowns>(
        frame,
        "raid offers cannot be proved off cooldown, so their tint is unverified",
    );
    out.note::<tables::RaidPathDistances>(
        frame,
        "naval and land raid ranges use their own content searches while the dispatcher memos are unavailable",
    );
    let resources = "raid offers whose target type compares a state resource are withheld while the state resource inputs are incomplete";
    out.note::<tables::StateResourceInputs>(frame, resources);
    out.note::<tables::StateResources>(frame, resources);
    out.note::<tables::CountryBuildingResources>(frame, resources);
    out.note::<tables::StateDynamicModifiers>(
        frame,
        "raid offers that test a state's dynamic modifiers are withheld while those modifiers are incomplete",
    );
    out.note::<tables::CountryWars>(
        frame,
        "raid army reach cannot decide entry into an occupied neutral's province: the enemy lists and occupation rules are unknown",
    );
    out.note::<tables::NavalRegionAccess>(
        frame,
        "raid army reach through a strategic sea region stays unknown while the region access modes are incomplete",
    );
    out.note::<tables::BorderConflicts>(
        frame,
        "raid army reach cannot decide which orders groups a border conflict admits",
    );
    out.note::<tables::FactionTheatres>(frame, "a theatre with no row draws no faction-theatre pin; its regions, country count, emblem and hidden list are the pin's inputs");
    out.note::<tables::LeaderPortraits>(
        frame,
        "HQ portraits remain undrawn while the runtime leader census is incomplete or unavailable",
    );
    out.note::<tables::CharacterPortraits>(frame, "character-backed HQ portraits remain undrawn while their runtime portrait census is incomplete or unavailable; recorded non-character leader pictures remain usable");
    out.note::<tables::OrdersGroupCounterInputs>(frame, "saved withdrawing status is unknown for the missing groups, so their HQ leader-en-route flag is not known");
    out.note::<tables::LeaderPortraitPaths>(
        frame,
        "missing leader country/tag operands leave the picture directory unavailable",
    );
    out.note::<tables::TaskForceCounterInputs>(frame, "mission-region presence, running-away status, engagement rule, bombardment region, accessible-region count, repair parent, detached activity, repair last mission and received-supply ratio are unknown for the missing task forces");
    out.note::<tables::TaskForceMapMotion>(frame, "no task force is placed on the map or in a port: an incomplete motion census cannot say which task forces are docked, in combat or under way");
    out.note::<tables::FleetCounterInputs>(frame, "fleet marker RGBA, mission-region count and ordered task-force roster are unknown for the missing fleets");
    out.note::<tables::CountryCounterInputs>(frame, "pride-of-the-fleet references and owned-state counts are unknown for the missing countries");
    out.note::<tables::RailwayGunCounterInputs>(frame, "railway guns are not drawn: an incomplete gun census cannot establish which guns share a plate");
    out.note::<tables::ActiveAbilities>(frame, "the land counter ability grid is not drawn: a commander missing from the census cannot be shown to have no active ability");
    let recorded_order =
        rows::<tables::ProvinceUnitOrder>(frame).is_ok_and(|rows| rows.is_complete());
    if rows::<tables::Divisions>(frame).is_ok_and(|rows| !rows.rows().is_empty()) {
        let counter_inputs = [
            complete::<tables::OrdersGroupCounterInputs>(frame),
            complete::<tables::OrdersGroupPlans>(frame),
            complete::<tables::LeaderPortraitPaths>(frame),
        ]
        .into_iter()
        .all(|complete| complete);
        if recorded_order && counter_inputs {
            out.note_field("Counter presentation", "Units follow recorded province occupant order. Columns are the native rebuild from cursor 0 over the camera-admitted sources in province and slot order: the game's own source array and cursor are presentation history of one client, so its column order, and the flag of a mixed-country plate (its first admitted division's), can differ from what the recording player saw. Order resolution, group colours and portrait directory operands are used when available. The NATO counter and counter colour options are the viewer's (from the user folder's settings.txt, switchable), not the recording player's, which no recording holds. Generated culture portraits remain unavailable; absent ArmyIconInputs rows use the true template.");
        } else if recorded_order {
            out.note_field("Counter presentation", "Province occupant order is recorded, but the orders-group plans, withdrawal bytes or portrait directory inputs are incomplete; the missing fields use the individual table limitations listed above.");
        } else {
            out.note_field("Counter presentation", "The province occupant census is incomplete or unavailable. Units use cold province/bucket and handle order, so first-fit grouping, plate order and mixed-country lead flags can differ from the game. Other missing counter operands follow the individual table limitations listed above.");
        }
    }
    out.note::<tables::OrdersGroups>(frame, "HQ provider plates and portraits require hq_provider and leader; missing groups have no HQ decoration");
    out.note::<tables::UnitLeaders>(
        frame,
        "character-backed HQ portraits require the orders-group leader's recorded category",
    );
    if rows::<tables::TaskForces>(frame).is_ok_and(|rows| !rows.rows().is_empty()) {
        let naval_inputs = [
            complete::<tables::TaskForceCounterInputs>(frame),
            complete::<tables::FleetCounterInputs>(frame),
            complete::<tables::CountryCounterInputs>(frame),
        ]
        .into_iter()
        .all(|complete| complete);
        if recorded_order && naval_inputs {
            out.note_field("Naval counter presentation", "Naval counters use recorded occupant and roster order, ship flags and maxima, mission and repair operands, fleet marker colours and naval pride references. Bars use the native strength and organisation formulas over visible members at sea and all members in port. Missing rows keep their stated per-table gaps; unknown reserve status does not merge fleets or draw a sea marker. Task forces at sea share their province's sources with land units and first-fit across the column's sources; a moving one stands at the settled route point, and one in combat has no counter. Uncertain: hull ties between equal-strength variants and port approximate-intel text. Unknown engagement-rule operands suppress order glyphs.");
        } else if recorded_order {
            out.note_field("Naval counter presentation", "Recorded occupant and ship roster order is available, but some mission, repair, fleet or country inputs are unavailable. Capital counts, hull icons and carrier glyphs use complete ship operands; missing status and marker operands stay unknown.");
        } else {
            out.note_field("Naval counter presentation", "The province occupant census is incomplete or unavailable. Naval counters use cold order, so native grouping may differ. Incomplete ship operands leave capital counts, hull icons and status cells unknown; bar fallback is reported by the missing tables above.");
        }
    }
    // counters
    out.note::<tables::Divisions>(
        frame,
        "land counters count only the recorded divisions, and a land combat's attack direction may \
         be missing",
    );
    out.note::<tables::DivisionTemplateIcons>(
        frame,
            "a land counter whose division's template is among the missing rows draws no unit-type icon, and its dark overlay panel is left bare; different missing templates remain separate because their grouping keys are unknown; at close zoom its division draws no model, because with no template there is no sub-unit for the engine's entity ladder to start from",
    );
    if let Ok(templates) = rows::<tables::DivisionTemplateIcons>(frame) {
        let unreadable = templates
            .rows()
            .iter()
            .filter(|template| template.model_unreadable != 0)
            .count();
        if unreadable > 0 {
            out.note_field(
                "Division template model inputs",
                &format!(
                    "model inputs of {unreadable} division {} were unreadable this hour; at close zoom their divisions draw no model, and their counters are unaffected",
                    if unreadable == 1 { "template" } else { "templates" }
                ),
            );
        }
    }
    // A recording without DivisionStatus draws its land figures and exercise glyphs from
    // DivisionModelInputs.
    if !frame.recorded(tables::DivisionStatus::ID) {
        out.note::<tables::DivisionStatus>(
            frame,
            "a land counter with a division among the missing rows draws no organisation or strength bars, and that division's supply, attrition, entrenchment and no-order glyphs are not drawn",
        );
        out.note::<tables::DivisionModelInputs>(
            frame,
            "a division among the missing rows has no known model number, camouflage or exercise: at close zoom it draws no model, and its land counter draws no exercise glyph unless another division of its stack exercises",
        );
    } else {
        out.note::<tables::DivisionStatus>(
            frame,
            "a land counter with a division among the missing rows draws no organisation or strength bars, and that division's supply, attrition, entrenchment, exercise and no-order glyphs are not drawn; at close zoom its model number and camouflage are unknown, so it draws no model",
        );
    }
    // close zoom: the 3D model selectors' daily inputs
    out.note::<tables::DivisionEquipment>(
        frame,
        "a close-zoom division among the missing rows has no known equipment, so its model is unknown and not drawn",
    );
    out.note::<tables::NewDivisionEquipment>(
        frame,
        "a close-zoom division deployed since the daily equipment refresh has no known equipment until the next one, so its model is not drawn",
    );
    out.note::<tables::VariantDefinitions>(
        frame,
        "a close-zoom division whose equipment names a missing variant draws no model",
    );
    out.note::<tables::LexicalTokens>(
        frame,
        "a close-zoom division whose equipment type token is among the missing rows draws no model",
    );
    out.note::<tables::CountryLocalSubunits>(
        frame,
        "a close-zoom division whose template holds a country-local sub-unit clone draws no model, because the clone's requirements are unknown",
    );
    out.note::<tables::AvailableEquipmentVariants>(
        frame,
        "a close-zoom division with no qualifying equipment of its own draws no model, because the viewing country's rosters are unknown",
    );
    out.note::<tables::DivisionMovement>(
        frame,
            "a division among the missing rows counts as standing still: its counter and its close-zoom model sit on its province's own anchor instead of along its route, the model faces a hash of the province instead of along the route, its supply crate is not suppressed for a convoy crossing, its counter's tooltip shows no arrival estimate, and a division the row would have shown in naval transfer draws its land figure at full size rather than a quarter-scale transport",
    );
    out.note::<tables::TaskForces>(
        frame,
        "naval counters cover only the recorded task forces, and a naval base whose recorded \
         occupant array names one of the missing rows draws none of its port items (the task \
         force is listed by why)",
    );
    out.note::<tables::Ships>(
        frame,
        "a naval counter's ship count covers only the recorded ships, where the game counts every          ship in the task force's own array",
    );
    out.note::<tables::ShipModels>(frame, "at-sea hulls are hidden unless ShipModels, Ships and TaskForces are complete; stationary port formations stay hidden by the engine's own rule");
    if rows::<tables::ShipModels>(frame).is_ok_and(|r| !r.rows().is_empty()) {
        if rows::<tables::ShipModels>(frame)
            .is_ok_and(|r| r.rows().iter().all(|row| row.counter.is_some()))
        {
            out.note_field("ShipModels formation", "The capital slot takes the first recorded roster ship with flag bit 6 or 22; support slots take bit 8 then bits 7/39 in roster order. The formation stands where its task force's counter does. Named locators, retained slots, node RNG, the formation's facing and earlier explicit graphical variant selection are unrecorded. Models use deterministic handle phase and authored default animation pose.");
        } else {
            out.note_field("ShipModels formation", "Missing ship roster operands suppress the formation; neither capital choice nor support slot order can be established.");
        }
    }
    // orders: the recorded player's battle plan
    out.note::<tables::ArmyOrderPlans>(
        frame,
        "battle-plan binding, style and assignment; absent plans are not drawn",
    );
    out.note::<tables::OrdersGroupPlans>(
        frame,
        "battle-plan colour and root order lists; absent colours are not guessed and a missing group's orders stay unresolved",
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
    out.note::<tables::LandCombatSides>(
        frame,
        "a land combat among the missing rows carries no combat icon: its sides decide whether \
         the viewing country sees it",
    );
    out.note::<tables::AirBaseRegistrations>(
        frame,
        "an air base among the missing rows shows no plane count, because the recorded \
         registrations are what says whether the viewing country is shown one at all; with \
         the table truncated every base keeps the fallback badge for any recorded plane \
         instead of the recorded stationed-or-detail frame",
    );
    out.note::<tables::NavalCombats>(
        frame,
        "a naval combat among the missing rows carries no combat icon",
    );
    out.note::<tables::StateBuildings>(
        frame,
        "a building among the missing rows has no map mesh, and an air base no icon: the icon \
         lives on the state's building slot, so without the row there is no anchor, no icon and \
         no count",
    );
    out.note::<tables::ProvinceBuildings>(
        frame,
        "a building among the missing rows has no map mesh; a naval base carries no base icon",
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
        out.note_field("weather particle presentation", "WeatherRegions selects the common/weather.txt effect entities at map/weatherpositions.txt anchors; their particles follow the particle-update rules. A region's emitters start empty when its weather starts and grow on the window clock, so a dump (time 0) draws none; native seeds and frame deltas are not recorded. Weather particle descriptors are outside hash-verified content coverage.");
    }
    out.note::<tables::WeatherProvinces>(
        frame,
        "the snow and mud texture draws the missing provinces bare, and a winter post-effect volume missing a representative's snow is not graded",
    );
    if rows::<tables::WeatherProvinces>(frame).is_ok_and(|v| !v.rows().is_empty()) {
        let weather = rows::<tables::WeatherProvinces>(frame).unwrap();
        let display = weather
            .rows()
            .iter()
            .find(|row| row.id == 0)
            .map(|row| &row.display);
        if !matches!(display, Some(replay_data::rows::Recorded::Value(Some(_)))) {
            let reason = if matches!(display, Some(replay_data::rows::Recorded::NotRecorded)) {
                "not recorded"
            } else {
                "unavailable"
            };
            out.note_field("WeatherProvinces.display", &format!("1 retained snow/mud boundary {reason}; edge cells and refresh timing are unknown (weather.rs), so no current-simulation edge is substituted"));
        }
        out.note_field("Winter grading", "Each post-effect volume uses one representative province per strategic region: the last overlapping land province in list order. Its weight is the whole area of overlapping land provinces, and grading uses raw hourly WeatherProvinces snow. The runtime region-list order remains unverified.");
    }
    out
}

/// Whether this hour holds a complete recorded `T` table.
fn complete<T: TableSchema>(frame: &DecodedFrame) -> bool {
    rows::<T>(frame).is_ok_and(|rows| rows.is_complete())
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_data::rows::TableRows;
    use replay_data::{rows::PoliticalPartyRow, rows::ProvinceRow, rows::TableId};
    use replay_frame::frame::Metrics;

    /// A frame whose every table is unavailable, so a test only has to fill in
    /// the ones it cares about.
    fn frame() -> DecodedFrame {
        DecodedFrame {
            branch: 1,
            hour: 60_828_478,
            tables: std::array::from_fn(|_| TableView::Unavailable {
                reason: "the collector had not grown its buffer yet".into(),
            })
            .map(Into::into),
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
            ..Default::default()
        }
    }

    #[test]
    fn a_truncated_table_still_hands_over_the_rows_it_reached() {
        let mut frame = frame();
        *frame.tables[TableId::PoliticalParties.index()] = TableView::Truncated {
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
        assert!(matches!(
            view,
            Available::Truncated {
                missing: 38_015,
                ..
            }
        ));
    }

    #[test]
    fn an_unavailable_table_hands_over_no_rows_and_no_count() {
        let empty = frame();
        let view = rows::<tables::Divisions>(&empty).unwrap();
        assert!(view.rows().is_empty());
        assert!(!view.is_complete());
        assert!(matches!(view, Available::Unavailable { .. }));
        // A complete table is not a gap at all.
        let mut whole = frame();
        *whole.tables[TableId::Divisions.index()] =
            TableView::Complete(TableRows::Divisions(Vec::new()));
        let view = rows::<tables::Divisions>(&whole).unwrap();
        assert!(view.is_complete());
        assert!(matches!(view, Available::Complete(_)));
    }

    #[test]
    fn a_table_holding_another_tables_rows_is_a_named_error() {
        let mut frame = frame();
        *frame.tables[TableId::Divisions.index()] =
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
        *frame.tables[TableId::Provinces.index()] = TableView::Truncated {
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
        *frame.tables[TableId::Provinces.index()] =
            TableView::Complete(TableRows::Provinces(vec![province(1), province(2)]));
        assert_eq!(core::<tables::Provinces>(&frame).unwrap().len(), 2);
    }

    #[test]
    fn a_limitation_names_its_table_and_its_missing_count() {
        let mut frame = frame();
        *frame.tables[TableId::ProvinceBuildings.index()] = TableView::Truncated {
            rows: TableRows::ProvinceBuildings(Vec::new()),
            missing: 1174,
        };
        let mut limitations = Limitations::default();
        limitations.note::<tables::ProvinceBuildings>(&frame, "a naval base carries no icon");
        limitations.note::<tables::LandCombats>(&frame, "no combat icon is drawn");
        // A complete table contributes no line at all.
        *frame.tables[TableId::Ships.index()] = TableView::Complete(TableRows::Ships(Vec::new()));
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
        limitations.note_field("ProvinceNames.name", "1 province name is not text");
        assert_eq!(
            limitations.lines(),
            ["Field ProvinceNames.name: 1 province name is not text".to_owned()]
        );
    }

    /// Every counter on the frame depends on the two controls, so both are
    /// stated whether or not anything degraded — and a country other than the
    /// one asked for says why.
    #[test]
    fn the_viewing_country_and_the_fog_switch_are_stated_on_every_frame() {
        let line = |tag: &str, resolution: Resolution, fog: bool| {
            let mut out = Limitations::default();
            out.note_perspective(&Perspective {
                tag: tag.into(),
                resolution,
                fog,
            });
            // Province visibility is a recorded table, so its absence is the
            // registry's line and not a sentence every view carries.
            assert_eq!(out.lines().len(), 1);
            out.lines()[0].clone()
        };
        let recorded = line("GER", Resolution::Recorded, true);
        assert!(
            recorded.starts_with(
                "Viewing country GER: the recording client's stored viewpoint; fog of war on"
            ),
            "{recorded}"
        );
        let asked = line("SOV", Resolution::Requested, false);
        assert!(
            asked.contains("whoever the recording's own player was"),
            "{asked}"
        );
        assert!(asked.contains("fog of war off"), "{asked}");
        assert!(asked.contains("stay SOV's"), "{asked}");
        let absent = line("GER", Resolution::Absent("SOV".into()), true);
        assert!(absent.contains("records no country SOV"), "{absent}");
        let nobody = line("AFG", Resolution::NoPlayer, true);
        assert!(nobody.contains("this viewer's, not the game's"), "{nobody}");

        let mut undecided = Limitations::default();
        undecided.note_undecided_sight("FactionMembers");
        assert!(undecided.lines()[0].contains("table FactionMembers is unavailable"));
        assert!(undecided.lines()[0].contains("every stack is drawn in full"));
    }

    #[test]
    fn a_fog_rebuild_gap_names_its_input() {
        let mut limitations = Limitations::default();
        limitations.note_visibility_gap("RadarCoverage");
        assert!(limitations.lines()[0].contains("(RadarCoverage): no fog is drawn"));
    }

    #[test]
    fn the_registry_reports_every_degraded_table_and_no_core_one() {
        let mut registry = limitations(&frame());
        registry.note_fog_on_sight(&frame());
        let lines = registry.lines().join("\n");
        for table in [
            "StateFlags",
            "States",
            "ProvinceNames",
            "Politics",
            "PoliticalParties",
            "LocalPlayer",
            "WarRelations",
            "FactionMembers",
            "Subjects",
            "Autonomy",
            "UnitIntel",
            "RelationLinks",
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
            "WeatherProvinces",
            "ProvinceUnitOrder",
            "LeaderPortraits",
            "CharacterPortraits",
            "OrdersGroupCounterInputs",
            "LeaderPortraitPaths",
            "TaskForceCounterInputs",
            "TaskForceMapMotion",
            "FleetCounterInputs",
            "CountryCounterInputs",
            "RailwayGunCounterInputs",
            "ActiveAbilities",
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
            *whole.tables[table.index()] = TableView::Complete(TableRows::empty(table));
        }
        assert!(limitations(&whole).lines().is_empty());
    }

    #[test]
    fn unreadable_template_model_inputs_are_counted_in_their_own_line() {
        use replay_data::rows::DivisionTemplateIconRow;
        let mut whole = frame();
        for table in TableId::ALL {
            *whole.tables[table.index()] = TableView::Complete(TableRows::empty(table));
        }
        let template = |template, model_unreadable| DivisionTemplateIconRow {
            template,
            model_unreadable,
            ..Default::default()
        };
        *whole.tables[TableId::DivisionTemplateIcons.index()] =
            TableView::Complete(TableRows::DivisionTemplateIcons(vec![
                template(1, 1),
                template(2, 0),
                template(3, 1),
            ]));
        let lines = limitations(&whole).lines();
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(
            lines[0].contains("model inputs of 2 division templates were unreadable this hour"),
            "{lines:?}"
        );
        assert!(
            lines[0].contains("their counters are unaffected"),
            "{lines:?}"
        );
        *whole.tables[TableId::DivisionTemplateIcons.index()] =
            TableView::Complete(TableRows::DivisionTemplateIcons(vec![template(1, 1)]));
        assert!(
            limitations(&whole).lines()[0]
                .contains("model inputs of 1 division template were unreadable this hour")
        );
        // Readable templates add no line.
        *whole.tables[TableId::DivisionTemplateIcons.index()] =
            TableView::Complete(TableRows::DivisionTemplateIcons(vec![template(2, 0)]));
        assert!(limitations(&whole).lines().is_empty());
    }
}

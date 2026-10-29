//! Land combat icons.

use super::*;
use viewer_foundation::engagement_history::Provenance;
use viewer_foundation::icon_frames::{self, Kept};
use viewer_foundation::map::gui_resources::viewport;
use {
    viewer_foundation::engaged::Engaged, viewer_foundation::engagement_progress as progress,
    viewer_foundation::engagement_progress::FIXED,
    viewer_foundation::land_combat_math::ScoreDefines,
    viewer_foundation::land_combat_math::ScoreGap, viewer_foundation::land_combat_math::conditions,
    viewer_foundation::land_combat_math::side_scores,
};

/// The `NDefines` the land combat marker reads besides its priority.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LandCombatDefines {
    /// `NGraphics.LAND_COMBAT_{WINNING,LOSING,BALANCED}_COLOR`.
    pub colors: [[f32; 4]; 3],
    pub scores: ScoreDefines,
}

impl LandCombatDefines {
    pub fn read(defines: &Defines) -> io::Result<Self> {
        let color = |name: &str| -> io::Result<[f32; 4]> {
            match defines.numbers("NGraphics", name)?[..] {
                [red, green, blue, alpha] => {
                    Ok([red as f32, green as f32, blue as f32, alpha as f32])
                }
                _ => Err(invalid(format!(
                    "NDefines.NGraphics.{name} is not an RGBA colour"
                ))),
            }
        };
        Ok(Self {
            colors: [
                color("LAND_COMBAT_WINNING_COLOR")?,
                color("LAND_COMBAT_LOSING_COLOR")?,
                color("LAND_COMBAT_BALANCED_COLOR")?,
            ],
            scores: ScoreDefines::read(defines)?,
        })
    }

    /// The shipped values, for tests only.
    #[cfg(any(test, feature = "test-support"))]
    pub const VANILLA: Self = Self {
        colors: [
            [0.0, 1.0, 0.0, 1.0],
            [1.0, 0.0, 0.0, 1.0],
            [1.0, 1.0, 0.0, 1.0],
        ],
        scores: ScoreDefines::VANILLA,
    };
}

/// Tint a seen combat by the viewer's side estimate against the
/// other side's, with a ten-percent band either way.
pub(super) fn combat_tint(
    mut scores: [i64; 2],
    defending: bool,
    colors: &[[f32; 4]; 3],
) -> [f32; 4] {
    if defending {
        scores.swap(0, 1);
    }
    if scores[0] > scores[1].wrapping_mul(110_000) / FIXED {
        colors[0]
    } else if scores[1] > scores[0].wrapping_mul(110_000) / FIXED {
        colors[1]
    } else {
        colors[2]
    }
}

/// The progress box localises `COMBAT_ICON_PROGRESS` with `VAL` as a fixed-point
/// argument. A key the content lacks keeps its own bytes; text layout retains
/// colour runs and inline icons.
fn progress_text(
    localisation: &installed_content_localisation::localisation::Localisation,
    value: i64,
) -> Result<String, String> {
    const KEY: &str = "COMBAT_ICON_PROGRESS";
    localisation
        .localise_with(
            KEY.as_bytes(),
            &[(
                "VAL",
                installed_content_localisation::localisation::Argument::Fixed5(value),
            )],
        )
        .map_err(|refusal| refusal.to_string())
}

/// Keep a land combat's icon only while both sides' country lists are non-empty.
/// Without the side row, the icon's existence is unknown.
const NO_SIDE_ROW: &str =
    "icon omitted (it exists only while both sides' country lists are non-empty)";

/// Combatant flag bits the game shows the bombardment child for, on either side.
const BOMBARDMENT: u32 = (1 << 11) | (1 << 12);

/// One land combat's marker for the hour's viewing country. Nothing here moves
/// with the camera; [`land_combats`] only places and draws it.
#[derive(Clone, Debug, PartialEq)]
pub struct LandMarker {
    pub id: u64,
    pub(super) province: u32,
    /// The first attacker with a map entity: its province and stack kind.
    /// `None` keeps both anchors on the defender.
    pub(super) attacker: Option<(u32, u32)>,
    /// Whether the viewer has intelligence on the combat.
    pub seen: bool,
    /// Progress percentage on the fixed-point scale: fraction times one hundred.
    /// An unavailable fraction retains the question mark.
    pub(super) progress: Option<i64>,
    /// The plate and wedge colour; `None` for a seen combat whose survival
    /// estimate is unknown, which draws neither.
    pub(super) tint: Option<[f32; 4]>,
    /// The zero-based frame of the frame child: set by a seen combat's
    /// indicator byte, kept by an unseen one ([`icon_frames`]); `None`
    /// draws the frame child's authored initial frame, with its history reason counted.
    pub frame: Option<u32>,
    /// The bombardment child, for a seen combat.
    pub(super) bombardment: bool,
}

/// Undecided combat icons or parts of them: cause, then the combats it affects.
pub type Gaps = BTreeMap<String, BTreeSet<u64>>;

fn gap(gaps: &mut Gaps, cause: String, combat: u64) {
    gaps.entry(cause).or_default().insert(combat);
}

/// How `table` lacks the row for `key` this hour.
fn missing<T: replay_data::rows::TableSchema>(
    table: &Available<'_, T>,
    name: &str,
    key: impl std::fmt::Display,
) -> String {
    match table {
        Available::Complete(_) => format!("the {name} table has no row for {key}"),
        Available::Truncated { .. } => {
            format!("the truncated {name} table did not reach {key}")
        }
        Available::Unavailable { reason } => {
            format!("the {name} table is unavailable ({reason}), so {key} is unknown")
        }
        Available::NotRecorded { mode } => {
            format!("the {name} table is not recorded in a {mode} recording, so {key} is unknown")
        }
    }
}

/// This hour's land markers, why any combat has none or draws one without
/// an undecided part, and the limitations of the values they were drawn
/// with, each naming its combats.
#[derive(Debug, Default)]
pub struct LandMarkers {
    pub markers: Vec<LandMarker>,
    pub gaps: Gaps,
    /// Per limitation, the land combats whose value it applies to.
    pub limits: Gaps,
}

impl LandMarkers {
    /// `engaged` is this hour's flag per combat and where it came from
    /// ([`viewer_foundation::engagement_history::combats`]); `kept` what each live icon keeps
    /// for its frame ([`viewer_foundation::engagement_history::kept_frames`]); `movement`
    /// holds each division's derived stack kind.
    pub(super) fn read(
        frame: &DecodedFrame,
        vision: &Vision,
        links: &BTreeMap<u32, u32>,
        (engaged, provenance): &(BTreeMap<u64, Engaged>, Provenance),
        kept: &BTreeMap<u64, Kept>,
        movement: &super::land::Movements,
        defines: &LandCombatDefines,
    ) -> io::Result<Self> {
        let divisions = rows::<tables::Divisions>(frame)?;
        // Current organisation is the Divisions row's.
        let conditions = conditions(frame)?;
        let provinces: BTreeMap<_, _> = divisions
            .rows()
            .iter()
            .map(|row| (row.id, row.province))
            .collect();
        // Each recorded division's map entity bit and its
        // stack kind; only a kind rebuilt from content is the unit's native
        // kind ([`super::land::Movements`]).
        let kinds: BTreeMap<_, _> = rows::<tables::DivisionMovement>(frame)?
            .rows()
            .iter()
            .map(|row| {
                let kind = movement
                    .get(row.division)
                    .filter(|route| route.kind_resolved)
                    .map(|route| route.stack_kind);
                (row.division, (kind, row.map_entity != 0))
            })
            .collect();
        let status = rows::<tables::DivisionStatus>(frame)?;
        let progresses = progress::Progresses::read(frame)?;
        let sides_table = rows::<tables::LandCombatSides>(frame)?;
        let sides = progress::sides(frame)?;
        let details_table = rows::<tables::LandCombatDetails>(frame)?;
        let details: BTreeMap<_, _> = details_table.rows().iter().map(|r| (r.combat, r)).collect();
        let members = rows::<tables::LandCombatDivisions>(frame)?;
        let mut members_by_combat: BTreeMap<_, Vec<_>> = BTreeMap::new();
        for row in members.rows() {
            members_by_combat.entry(row.combat).or_default().push(row);
        }
        let resolve = |raw: u32| links.get(&raw).copied();
        let mut out = Self::default();
        let combats = rows::<tables::LandCombats>(frame)?.rows();
        // A load-rebuilt flag is kept, and named as such for every combat.
        if let Some(limit) = provenance.limitation()
            && !combats.is_empty()
        {
            out.limits
                .insert(limit.to_owned(), combats.iter().map(|c| c.id).collect());
        }
        for combat in combats {
            let marker = (|| {
                let sides = sides.get(&combat.id).ok_or_else(|| {
                    format!(
                        "{NO_SIDE_ROW}: {}",
                        missing(&sides_table, "LandCombatSides", "the combat")
                    )
                })?;
                // The game erases (or never takes) the icon of a combat
                // with an empty side: the game shows nothing, as here.
                if sides.attacker_primary.is_empty() || sides.defender_primary.is_empty() {
                    return Ok(None);
                }
                let attacker = first_attacker(&combat.attackers, &provinces, &kinds)
                    .map_err(|reason| format!("icon omitted: {reason}"))?;
                // The game branches on the cached engaged flag; the unseen
                // branch leaves the frame child as the last seen hour set it.
                let seen = match engaged.get(&combat.id).copied().unwrap_or(Engaged::Unknown) {
                    Engaged::Yes => true,
                    Engaged::No => false,
                    Engaged::Ambiguous(cause) => {
                        return Err(format!("icon omitted: {}", cause.reason()));
                    }
                    // Either cause: the samples keep no finer one.
                    Engaged::Unknown => {
                        return Err(
                            "icon omitted: engaged flag unknown (no refresh seen since the branch joined a running game or skipped hours, or an input the test reads is unrecorded)"
                                .to_owned(),
                        );
                    }
                };
                if !seen {
                    let kept = kept.get(&combat.id).copied().unwrap_or(Kept::Undecided);
                    return Ok(Some((
                        LandMarker {
                            id: combat.id,
                            province: combat.province,
                            attacker,
                            seen,
                            progress: None,
                            tint: Some([1.0; 4]),
                            frame: match kept {
                                Kept::Frame(frame) => Some(frame),
                                _ => None,
                            },
                            bombardment: false,
                        },
                        None,
                    )));
                }
                let defending = progress::defending(sides, vision, &resolve)
                    .map_err(|cause| format!("icon omitted: {}", progress::undefended(cause)))?;
                // The estimate, or the operand it lacks by the table
                // that lacks it.
                let (tint, tint_gap) = match details.get(&combat.id) {
                    None => (
                        None,
                        Some(missing(&details_table, "LandCombatDetails", "the combat")),
                    ),
                    Some(detail) => match side_scores(
                        detail,
                        members_by_combat
                            .get(&combat.id)
                            .into_iter()
                            .flatten()
                            .copied(),
                        &conditions,
                        [&combat.attackers, &combat.defenders],
                        defines.scores,
                    ) {
                        Ok(scores) => (Some(combat_tint(scores, defending, &defines.colors)), None),
                        Err(ScoreGap::Division(id)) => (
                            None,
                            Some(missing(&divisions, "Divisions", format!("division {id}"))),
                        ),
                        Err(ScoreGap::MaximumStrength(id)) => (
                            None,
                            Some(missing(
                                &status,
                                "DivisionStatus",
                                format!(
                                    "committed division {id}, whose maximum hit points the early damage estimate reads"
                                ),
                            )),
                        ),
                        Err(ScoreGap::Member { side, division }) => (
                            None,
                            Some(missing(
                                &members,
                                "LandCombatDivisions",
                                format!("committed division {division} on side {side}"),
                            )),
                        ),
                        Err(gap @ ScoreGap::Numbers { .. }) => (None, Some(gap.reason())),
                    },
                };
                Ok(Some((
                    LandMarker {
                        id: combat.id,
                        province: combat.province,
                        attacker,
                        seen,
                        // Pass the side's fraction times 100 as
                        // fixed point, `VAL` of COMBAT_ICON_PROGRESS.
                        progress: progresses.attacker(combat.id).map(|attacker| {
                            let side = if defending {
                                FIXED - attacker
                            } else {
                                attacker
                            };
                            side.wrapping_mul(10_000_000) / FIXED
                        }),
                        tint,
                        frame: Some(u32::from(sides.indicator != 0)),
                        bombardment: (sides.flags[0] | sides.flags[1]) & BOMBARDMENT != 0,
                    },
                    tint_gap,
                )))
            })();
            match marker {
                Ok(None) => {}
                Ok(Some((marker, tint_gap))) => {
                    if marker.seen {
                        // Sum every ordered member's organisation:
                        // the percentage is unknown exactly when a member is
                        // unrecorded.
                        if let Some(division) = progresses.unrecorded(combat.id) {
                            let cause =
                                missing(&divisions, "Divisions", format!("division {division}"));
                            gap(
                                &mut out.gaps,
                                format!("percentage shows '?' (progress): {cause}"),
                                combat.id,
                            );
                        }
                        if let Some(cause) = tint_gap {
                            gap(
                                &mut out.gaps,
                                format!("plate and wedge omitted (survival estimate): {cause}"),
                                combat.id,
                            );
                        }
                    } else {
                        let limit = kept
                            .get(&combat.id)
                            .copied()
                            .unwrap_or(Kept::Undecided)
                            .limitation(icon_frames::LAND_NO_HISTORY);
                        gap(&mut out.limits, limit, combat.id);
                    }
                    out.markers.push(marker);
                }
                Err(cause) => gap(&mut out.gaps, cause, combat.id),
            }
        }
        Ok(out)
    }
}

/// Point the icon using the first participant attacker's display province
/// and recorded-route stack kind. Missing movement or geometry leaves its
/// direction unavailable.
pub(super) fn first_attacker(
    attackers: &[u64],
    provinces: &BTreeMap<u64, u32>,
    kinds: &BTreeMap<u64, (Option<u32>, bool)>,
) -> Result<Option<(u32, u32)>, &'static str> {
    for id in attackers {
        match kinds.get(id) {
            Some(&(_, false)) => {}
            Some(&(kind, true)) => {
                let province = *provinces.get(id).ok_or(
                    "the first mapped attacker's province is unrecorded (it has no Divisions row)",
                )?;
                let kind = kind.ok_or(
                    "the first mapped attacker's stack kind is unknown (content cannot place its route)",
                )?;
                return Ok(Some((province, kind)));
            }
            None => {
                return Err(
                    "an attacker's map entity is unrecorded (it has no DivisionMovement row), so the icon's first mapped attacker is unknown",
                );
            }
        }
    }
    Ok(None)
}

/// Combat direction uses the first participant attacker and the defender anchor.
/// Start with both anchors at the combat province's defending slot, then replace
/// the attacker anchor when the referenced unit and its current stack kind resolve.
pub(super) fn combat_direction(
    inputs: &CounterInputs<'_>,
    province: u32,
    attacker_anchor: Option<(u32, u32)>,
) -> Result<([f32; 3], f32), String> {
    let (defender, attacker) = combat_anchors(
        |province, kind| inputs.content.anchor(province, kind),
        province,
        attacker_anchor,
    )?;
    Ok(combat_midpoint(inputs.camera.map.x(), defender, attacker))
}

/// The defender and attacker anchors [`combat_direction`] reads from the
/// content's unit stacks, or which one the content lacks. Without a mapped
/// attacker both are the defender's, as the game seeds them.
pub(super) fn combat_anchors(
    anchor: impl Fn(u32, u32) -> Option<[f32; 3]>,
    province: u32,
    attacker: Option<(u32, u32)>,
) -> Result<([f32; 3], [f32; 3]), String> {
    let defender = anchor(province, DEFENDING)
        .or_else(|| anchor(province, STANDSTILL))
        .ok_or_else(|| {
            format!(
                "the content places neither a Defending nor a Standstill unit-stack anchor for combat province {province}"
            )
        })?;
    let Some((at, kind)) = attacker else {
        return Ok((defender, defender));
    };
    let attacker = anchor(at, kind)
        .or_else(|| anchor(at, STANDSTILL))
        .ok_or_else(|| {
            format!(
                "the content places neither a unit-stack anchor of kind {kind} nor a Standstill one for the first mapped attacker's province {at}"
            )
        })?;
    Ok((defender, attacker))
}

/// The midpoint of the two anchors and the wedge angle, with the attacker's x
/// read on the wrapped map **near the defender**. `unwrap_x(reference, x)`
/// moves `x`; passing the defender as the value moved the defender onto the
/// attacker's copy and left the midpoint on the defender's own x. Reference 03
/// put the Danzig bubble 42 device pixels east of the midpoint that way.
pub(super) fn combat_midpoint(
    width: f32,
    defender: [f32; 3],
    attacker: [f32; 3],
) -> ([f32; 3], f32) {
    let attacker = [
        counter_groups::unwrap_x(width, defender[0], attacker[0]),
        attacker[1],
        attacker[2],
    ];
    let midpoint = [
        (attacker[0] + defender[0]) * 0.5,
        (attacker[1] + defender[1]) * 0.5,
        (attacker[2] + defender[2]) * 0.5,
    ];
    // Apply the direction angle to the icon's rotation scalar.
    let angle = (attacker[0] - defender[0]).atan2(-(attacker[2] - defender[2]));
    (midpoint, angle)
}

/// Offers the hour's land combats to the persistent native windows ([`land_windows`]).
///
/// The game binds `bg_btn`, `battle_indicator_direction`, `air_superiority_icon`,
/// `mapicon_land_combat_frame`, `mapicon_land_combat_bombardment` and
/// `combat_progress` by name, then drives them:
///
/// * **Unseen**: the wedge, air icon and bombardment child hide, the plate is white,
///   the text keeps its `?` and the frame child retains the last seen hour's frame
///   through the engaged index ([`icon_frames`]), or the authored initial frame
///   where history is unknown, with the reason counted per combat.
/// * **Seen**: plate and wedge take the winning, losing or balanced colour; the
///   text shows the viewer's side; the frame child shows frame 1 or 2 by the
///   combat's indicator byte; the bombardment child shows when either side's
///   flags carry bits 11 or 12. The air icon replaces the text and frame only
///   in air map mode.
///
/// The game appends `'_'` and the province's terrain key to the background button's
/// authored sprite name and uses that sprite when declared; otherwise it keeps
/// the authored name.
pub(super) fn land_combats(
    inputs: &CounterInputs<'_>,
    markers: &LandMarkers,
    out: &mut CounterFrame,
) -> io::Result<()> {
    for (cause, combats) in &markers.gaps {
        out.report
            .combat_gaps
            .entry(("Land", cause.clone()))
            .or_default()
            .extend(combats);
    }
    for (limit, combats) in &markers.limits {
        out.report
            .combat_limits
            .entry(("Land", limit.clone()))
            .or_default()
            .extend(combats);
    }
    // The raids map mode clears land combat icons with the other province icons.
    if !inputs.visibility.province_icons || !inputs.shows(inputs.content.defines.land_combat_cutoff)
    {
        return Ok(());
    }
    for marker in &markers.markers {
        let (anchor, angle) = match combat_direction(inputs, marker.province, marker.attacker) {
            Ok(direction) => direction,
            Err(cause) => {
                out.report
                    .combat_gaps
                    .entry(("Land", format!("icon omitted: {cause}")))
                    .or_default()
                    .insert(marker.id);
                continue;
            }
        };
        let Some(projection) = inputs.camera.project_map_icon(
            anchor,
            inputs.viewport,
            viewport(inputs.viewport, inputs.ui_scale),
        ) else {
            continue;
        };
        // Write "?" when unseen.
        let text = match marker.progress {
            Some(value) => progress_text(inputs.localisation, value),
            None => Ok("?".to_owned()),
        }
        .unwrap_or_else(|cause| {
            out.report
                .combat_gaps
                .entry(("Land", format!("progress text refused: {cause}")))
                .or_default()
                .insert(marker.id);
            String::new()
        });
        out.native_land_combats.placed.push(land_windows::Placed {
            marker: marker.clone(),
            screen: projection.position,
            depth: projection.depth,
            angle,
            terrain: inputs
                .content
                .terrain
                .combat_terrain
                .get(&marker.province)
                .cloned(),
            text,
            anchor: inputs.icon_anchor(
                IconFamily::Combats,
                anchor,
                inputs.content.defines.land_combat_cutoff,
            ),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_data::{
        rows::DivisionMovementRow, rows::DivisionRow, rows::DivisionStatusRow,
        rows::LandCombatDetailsRow, rows::LandCombatDivisionRow, rows::LandCombatRow,
        rows::LandCombatSideRow, rows::TableId, rows::TableRows,
    };
    use replay_frame::{frame::Metrics, frame::TableView};

    fn frame() -> DecodedFrame {
        DecodedFrame {
            branch: 0,
            hour: 3,
            tables: std::array::from_fn(|_| {
                TableView::Unavailable {
                    reason: "not recorded".into(),
                }
                .into()
            }),
            metrics: Metrics::default(),
        }
    }

    /// One seen combat 62: attackers 1 and 2, whose map entities are
    /// recorded null, and defender 3. `divisions` are the recorded Divisions
    /// rows; every recorded one has a status row.
    fn land_frame(divisions: &[u64]) -> DecodedFrame {
        let mut frame = frame();
        *frame.tables[TableId::LandCombats.index()] =
            TableView::Complete(TableRows::LandCombats(vec![LandCombatRow {
                id: 62,
                province: 9,
                attacker_classes: vec![0; 2],
                attackers: vec![1, 2],
                defenders: vec![3],
                ..Default::default()
            }]));
        *frame.tables[TableId::LandCombatSides.index()] =
            TableView::Complete(TableRows::LandCombatSides(vec![LandCombatSideRow {
                combat: 62,
                attacker_primary: vec![5],
                defender_primary: vec![6],
                ..Default::default()
            }]));
        *frame.tables[TableId::Divisions.index()] = TableView::Complete(TableRows::Divisions(
            divisions
                .iter()
                .map(|&id| DivisionRow {
                    id,
                    organisation: 10,
                    ..Default::default()
                })
                .collect(),
        ));
        *frame.tables[TableId::DivisionStatus.index()] =
            TableView::Complete(TableRows::DivisionStatus(
                divisions
                    .iter()
                    .map(|&division| DivisionStatusRow {
                        division,
                        ..Default::default()
                    })
                    .collect(),
            ));
        *frame.tables[TableId::DivisionMovement.index()] =
            TableView::Complete(TableRows::DivisionMovement(
                [1, 2]
                    .map(|division| DivisionMovementRow {
                        division,
                        ..Default::default()
                    })
                    .to_vec(),
            ));
        frame
    }

    fn land_gaps(frame: &DecodedFrame) -> Gaps {
        let links = BTreeMap::from([(5, 5), (6, 6)]);
        let engaged = (BTreeMap::from([(62, Engaged::Yes)]), Provenance::Indexed);
        let markers = LandMarkers::read(
            frame,
            // The viewer attacks: its side is proven without relations.
            &Vision::sharing(5, []),
            &links,
            &engaged,
            &BTreeMap::new(),
            &super::super::land::Movements::default(),
            &LandCombatDefines::VANILLA,
        )
        .unwrap();
        assert!(markers.limits.is_empty(), "{:?}", markers.limits);
        markers.gaps
    }

    fn causes(gaps: &Gaps) -> Vec<(&str, Vec<u64>)> {
        gaps.iter()
            .map(|(cause, combats)| (cause.as_str(), combats.iter().copied().collect()))
            .collect()
    }

    /// The progress box localises COMBAT_ICON_PROGRESS with `VAL` as a
    /// fixed-point argument, so the content's own
    /// format and text around it show; a key the content lacks is drawn as
    /// its bytes, not as a number.
    #[test]
    fn the_progress_box_localises_its_fixed_point_value() {
        let localisation: installed_content_localisation::localisation::Localisation =
            [("COMBAT_ICON_PROGRESS", "§Y$VAL|1$§! pct")]
                .into_iter()
                .map(|(key, value)| (key.to_owned(), value.to_owned()))
                .collect();
        // 0.4567 of the fight: VAL 45.67 in fixed point, one decimal shown.
        let value = 4_567_000;
        assert_eq!(
            progress_text(&localisation, value).as_deref(),
            Ok("\u{11}Y45.6\u{11}! pct")
        );
        let empty = installed_content_localisation::localisation::Localisation::default();
        assert_eq!(
            progress_text(&empty, value).as_deref(),
            Ok("COMBAT_ICON_PROGRESS")
        );
    }

    /// A seen marker names each undecided part's own cause and its combat:
    /// the unrecorded member behind the percentage, and the missing details
    /// row behind the plate and wedge, each with how its table lacks it.
    #[test]
    fn land_gaps_name_the_combat_and_each_part_s_own_cause() {
        let frame = land_frame(&[1, 3]);
        assert_eq!(
            causes(&land_gaps(&frame)),
            [
                (
                    "percentage shows '?' (progress): the Divisions table has no row for division 2",
                    vec![62]
                ),
                (
                    "plate and wedge omitted (survival estimate): the LandCombatDetails table is unavailable (not recorded), so the combat is unknown",
                    vec![62]
                ),
            ]
        );
    }

    /// With a details row, the estimate's gap names the participant without
    /// its Divisions row, the committed unit early damage needs, and a
    /// committed unit without the maximum hit points the game reads, each
    /// by the table that lacks it. A participant's missing DivisionStatus
    /// row alone leaves the estimate whole: the game sums current
    /// organisation only.
    #[test]
    fn a_missing_survival_estimate_names_its_operand() {
        let mut frame = land_frame(&[1, 2, 3]);
        let mut detail = LandCombatDetailsRow {
            combat: 62,
            ..Default::default()
        };
        detail.sides[0].committed = vec![1];
        *frame.tables[TableId::LandCombatDetails.index()] =
            TableView::Complete(TableRows::LandCombatDetails(vec![detail]));
        let TableView::Complete(TableRows::Divisions(divisions)) =
            &mut *frame.tables[TableId::Divisions.index()]
        else {
            unreachable!()
        };
        let second = divisions.remove(1);
        assert_eq!(
            causes(&land_gaps(&frame)),
            [
                (
                    "percentage shows '?' (progress): the Divisions table has no row for division 2",
                    vec![62]
                ),
                (
                    "plate and wedge omitted (survival estimate): the Divisions table has no row for division 2",
                    vec![62]
                ),
            ]
        );
        let TableView::Complete(TableRows::Divisions(divisions)) =
            &mut *frame.tables[TableId::Divisions.index()]
        else {
            unreachable!()
        };
        divisions.insert(1, second);
        *frame.tables[TableId::LandCombatDivisions.index()] = TableView::Truncated {
            rows: TableRows::LandCombatDivisions(Vec::new()),
            missing: 4,
        };
        assert_eq!(
            causes(&land_gaps(&frame)),
            [(
                "plate and wedge omitted (survival estimate): the truncated LandCombatDivisions table did not reach committed division 1 on side 0",
                vec![62]
            )]
        );
        *frame.tables[TableId::LandCombatDivisions.index()] =
            TableView::Complete(TableRows::LandCombatDivisions(vec![
                LandCombatDivisionRow {
                    combat: 62,
                    side: 0,
                    division: 1,
                    ..Default::default()
                },
            ]));
        let TableView::Complete(TableRows::DivisionStatus(status)) =
            &mut *frame.tables[TableId::DivisionStatus.index()]
        else {
            unreachable!()
        };
        // Participant 2 is not committed: its maxima are never read.
        status.retain(|row| row.division != 2);
        assert_eq!(causes(&land_gaps(&frame)), []);
        let TableView::Complete(TableRows::DivisionStatus(status)) =
            &mut *frame.tables[TableId::DivisionStatus.index()]
        else {
            unreachable!()
        };
        status.retain(|row| row.division != 1);
        assert_eq!(
            causes(&land_gaps(&frame)),
            [(
                "plate and wedge omitted (survival estimate): the DivisionStatus table has no row for committed division 1, whose maximum hit points the early damage estimate reads",
                vec![62]
            )]
        );
    }

    /// A combat with no marker names its combat and why.
    #[test]
    fn an_omitted_land_icon_names_its_combat() {
        let mut frame = land_frame(&[1, 2, 3]);
        *frame.tables[TableId::LandCombatSides.index()] = TableView::Truncated {
            rows: TableRows::LandCombatSides(Vec::new()),
            missing: 1,
        };
        assert_eq!(
            causes(&land_gaps(&frame)),
            [(
                "icon omitted (it exists only while both sides' country lists are non-empty): the truncated LandCombatSides table did not reach the combat",
                vec![62]
            )]
        );
    }

    /// Read both anchors from the content's unit stacks; content
    /// that places neither slot refuses the icon by province, never a guess.
    #[test]
    fn combat_anchors_refuse_by_province_and_slot() {
        let anchors = BTreeMap::from([
            ((9, DEFENDING), [1.0, 0.0, 1.0]),
            ((4, STANDSTILL), [2.0, 0.0, 2.0]),
        ]);
        let anchor = |province, kind| anchors.get(&(province, kind)).copied();
        assert_eq!(
            combat_anchors(anchor, 9, None),
            Ok(([1.0, 0.0, 1.0], [1.0, 0.0, 1.0]))
        );
        assert_eq!(
            combat_anchors(anchor, 9, Some((4, 7))),
            Ok(([1.0, 0.0, 1.0], [2.0, 0.0, 2.0]))
        );
        assert_eq!(
            combat_anchors(anchor, 9, Some((5, 7))),
            Err("the content places neither a unit-stack anchor of kind 7 nor a Standstill one for the first mapped attacker's province 5".to_owned())
        );
        assert_eq!(
            combat_anchors(anchor, 8, None),
            Err("the content places neither a Defending nor a Standstill unit-stack anchor for combat province 8".to_owned())
        );
    }
}

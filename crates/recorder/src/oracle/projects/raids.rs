use super::*;

pub(super) fn compare_raid_countries(
    frame: &DecodedFrame,
    pack: &saved::Pack,
    countries: &BTreeMap<u32, &CountryRow>,
    tokens: &TokenNames<'_>,
    report: &mut Report,
) -> io::Result<()> {
    let expected = pack
        .raids
        .as_ref()
        .ok_or_else(|| invalid("save lacks raids root; absence does not certify empty graph"))?;
    let mut parents = BTreeSet::new();
    let mut count = 0;
    for row in complete::<tables::RaidCountries>(frame)? {
        if !parents.insert(row.canonical_slot) {
            return Err(invalid("duplicate raid parent slot"));
        }
        let Some(saved) = expected.get(row.canonical_slot as usize) else {
            missing(
                report,
                "RaidCountries",
                row.canonical_slot,
                "absent from save",
            );
            continue;
        };
        let label = format!("RaidCountry {}", row.canonical_slot);
        equal(
            report,
            format!("{label}.country"),
            signed_country(row.country, countries)?,
            saved.country.as_deref(),
        );
        equal(
            report,
            format!("{label}.used_types.len"),
            row.used_types.len(),
            saved.used_types.len(),
        );
        for (index, (&actual, expected)) in row.used_types.iter().zip(&saved.used_types).enumerate()
        {
            compare_token(
                report,
                &format!("{label}.used_types[{index}]"),
                tokens,
                actual,
                expected,
            );
        }
        count += 1;
    }
    for parent in expected {
        if !parents.contains(&parent.ordinal) {
            missing(
                report,
                "RaidCountries",
                parent.ordinal,
                "absent from replay",
            );
        }
    }
    report.compared("RaidCountries", count);
    report.limitation(
        "the eligible-type cache is rebuilt at load and not saved",
        "RaidCountries.eligible_types",
    );
    Ok(())
}

/// A per-raid reader's census rule over a raid's phase, outcome and end hour
/// at the frame's hour, as the reader applies it to the live raid.
type RaidRule = fn(&engine::raid_speed_inputs::Raid, i32) -> bool;

/// RaidSpeedInputs' census: [`engine::raid_speed_inputs::live`].
fn raid_flying(raid: &engine::raid_speed_inputs::Raid, hour: i32) -> bool {
    engine::raid_speed_inputs::live(raid.phase, raid.outcome, raid.end_hour, hour)
}

/// A saved raid's end hour: the engine hour of its `end_date`, written from
/// the same stored end-hour word the readers record as `end_hour`
/// ([`saved::Raid::end_date`]).
fn saved_end_hour(raid: &saved::Raid) -> io::Result<Option<i32>> {
    raid.end_date.map(|date| date.engine_hour()).transpose()
}

/// Saved raids by full reference.
type SavedRaids<'a> = BTreeMap<u64, &'a saved::Raid>;
/// The saved raids a per-raid reader records, with their end hours.
type RaidCensus<'a> = BTreeMap<u64, (&'a saved::Raid, Option<i32>)>;

/// Every saved raid by full reference, and the ones a per-raid reader must
/// have recorded at the frame's hour by its `rule`, with their end hours.
/// The frame's hour is the save's (`matching_hour`), the current engine hour
/// the reader tests the live raid against.
fn raid_census(
    pack: &saved::Pack,
    hour: i32,
    rule: RaidRule,
) -> io::Result<(SavedRaids<'_>, RaidCensus<'_>)> {
    let saved = pack
        .raids
        .as_ref()
        .ok_or_else(|| invalid("save lacks raids root; absence does not certify empty graph"))?;
    let raids: BTreeMap<u64, &saved::Raid> = saved
        .iter()
        .flat_map(|parent| parent.instances.iter())
        .map(|raid| (raid.id, raid))
        .collect();
    let mut expected = BTreeMap::new();
    for (&id, &raid) in &raids {
        let end_hour = saved_end_hour(raid)?;
        let live = engine::raid_speed_inputs::Raid {
            pointer: 0,
            reference_type: id as u32,
            id: (id >> 32) as u32,
            phase: raid.phase,
            outcome: raid.outcome,
            end_hour,
        };
        if rule(&live, hour) {
            expected.insert(id, (raid, end_hour));
        }
    }
    Ok((raids, expected))
}

/// A recorded per-raid row against the census ([`raid_census`]): `None`, with
/// the difference reported, when the census holds no such raid. The saved
/// raid's phase, outcome (when the table records it) and end hour are
/// compared with the row's.
#[allow(clippy::too_many_arguments)]
fn census_member<'a>(
    report: &mut Report,
    table: &str,
    id: u64,
    raids: &SavedRaids<'a>,
    expected: &RaidCensus<'a>,
    phase: i32,
    outcome: Option<i32>,
    end_hour: Option<i32>,
) -> Option<&'a saved::Raid> {
    let label = format!("{table} {id}");
    let Some(&(saved, saved_end)) = expected.get(&id) else {
        let side = if raids.contains_key(&id) {
            "saved raid outside the reader's census"
        } else {
            "absent from save"
        };
        missing(report, table, id, side);
        return None;
    };
    equal(report, format!("{label}.phase"), phase, saved.phase);
    if let Some(outcome) = outcome {
        equal(report, format!("{label}.outcome"), outcome, saved.outcome);
    }
    equal(report, format!("{label}.end_hour"), end_hour, saved_end);
    Some(saved)
}

/// RaidEquipment against each saved raid's `equipment` pool and `nukes`.
/// The pool writer writes the entries in pool order and omits zero amounts unless
/// `allow_zero_entries`; the raid writer omits the whole pool when the gate finds it
/// empty and `nukes` when zero.
///
/// Membership is checked both ways: every saved raid the reader's rule
/// ([`engine::raid_equipment::recorded`]: phases 1 to 3, or flying) admits at
/// the frame's hour needs a row, and every row needs such a saved raid.
///
/// The pool writer does not write the pool's per-archetype totals. Every pool
/// operation the raid functions call moves a total by its entry's change:
/// variant addition (from raid equipment allocation, and through pool
/// subtraction and variant subtraction), pool addition and pool clearing.
/// Loading assigns each entry and moves the total by the difference.
/// Quantity scaling, which rounds totals and entries separately,
/// has no raid caller. So each total is compared exactly with the sum of its
/// archetype's saved amounts, the archetype of a variant being its
/// `VariantDefinitions.archetype_token`, as for StockpileArchetypes.
///
/// Each of those operations clamps the total and the entry separately to
/// `+-0x53E2D620FB40` (`L`): a row whose recorded total, a contributing saved
/// entry or the saved sum reaches the bound is a limitation. The one case
/// the comparison cannot see is a clamp that later unwound: adding `3/4 L`
/// to two variants of one archetype clamps the total at `L`, and removing
/// one leaves a native total of `1/4 L` beside saved entries summing to
/// `3/4 L`, with no current value at the bound. That needs the pool to have
/// held at least `L` (about 9.2e8 units) of one archetype at some point, so
/// in a real save it would only ever surface as a reported difference.
pub(super) fn compare_raid_equipment(
    frame: &DecodedFrame,
    pack: &saved::Pack,
    report: &mut Report,
) -> io::Result<()> {
    let (raids, expected) = raid_census(pack, frame.hour, engine::raid_equipment::recorded)?;
    // Without the definitions no total is derived; the entries still compare.
    let archetype_of: Option<BTreeMap<u64, u32>> = complete::<tables::VariantDefinitions>(frame)
        .ok()
        .map(|rows| {
            rows.iter()
                .map(|row| (row.id, row.archetype_token))
                .collect()
        });
    let mut recorded = BTreeSet::new();
    let mut compared = 0;
    let mut totals_compared = 0;
    for row in complete::<tables::RaidEquipment>(frame)? {
        let id = (u64::from(row.id) << 32) | u64::from(row.reference_type);
        if !recorded.insert(id) {
            return Err(invalid("duplicate raid equipment full reference"));
        }
        let label = format!("RaidEquipment {id}");
        let Some(saved) = census_member(
            report,
            "RaidEquipment",
            id,
            &raids,
            &expected,
            row.phase,
            None,
            row.end_hour,
        ) else {
            continue;
        };
        if row.variants.len() != row.amounts.len() {
            return Err(invalid("raid equipment variant and amount lists disagree"));
        }
        if row.archetype_tokens.len() != row.archetype_amounts.len() {
            return Err(invalid(
                "raid equipment archetype token and total lists disagree",
            ));
        }
        compared += 1;
        equal(report, format!("{label}.nukes"), row.nukes, saved.nukes);
        let allow_zero = saved
            .equipment
            .as_ref()
            .and_then(|pool| pool.allow_zero_entries)
            .unwrap_or(false);
        let visible: Vec<(u64, i64)> = row
            .variants
            .iter()
            .copied()
            .zip(row.amounts.iter().copied())
            .filter(|&(_, amount)| allow_zero || amount != 0)
            .collect();
        let pool = saved
            .equipment
            .as_ref()
            .map_or(&[][..], |pool| pool.equipment.as_slice());
        equal(
            report,
            format!("{label}.equipment.count"),
            visible.len(),
            pool.len(),
        );
        for (ordinal, (&(variant, amount), saved)) in visible.iter().zip(pool).enumerate() {
            equal(
                report,
                format!("{label}.equipment[{ordinal}].variant"),
                variant,
                saved.variant,
            );
            report.scalar(
                &format!("{label}.equipment[{ordinal}]"),
                "amount",
                Some(amount),
                Some(&saved.amount),
            );
        }
        match &archetype_of {
            Some(archetype_of) => {
                totals_compared +=
                    compare_raid_archetype_totals(row, pool, archetype_of, &label, report)
            }
            None => report.limitation(
                "VariantDefinitions is not complete in this frame, so no saved variant's archetype and no raid pool archetype total is derived",
                format!("{label}.archetype_amounts"),
            ),
        }
    }
    for id in expected.keys().filter(|id| !recorded.contains(id)) {
        missing(report, "RaidEquipment", id, "absent from replay");
    }
    report.compared("RaidEquipment", compared);
    report.compared("RaidEquipment.archetype_amounts", totals_compared);
    report.limitation(
        "the pool writer omits zero entries unless allow_zero_entries, so omitted zero membership and absolute pool ordinals are not save-certified",
        "RaidEquipment",
    );
    report.limitation(
        "The pool writer does not write the per-archetype totals; they are compared as sums of the saved amounts by VariantDefinitions archetype, so their native order and a zero total's membership are not save-certified. variant addition clamps a total and an entry separately to +-0x53E2D620FB40 (L); a clamp that later unwound (3/4 L added to two variants of one archetype, then one removed: native total 1/4 L, saved entries 3/4 L, nothing at the bound) is not visible and needs a pool that once held at least L (about 9.2e8 units) of one archetype, so it can only surface as a reported difference",
        "RaidEquipment.archetype_tokens",
    );
    Ok(())
}

/// One raid's recorded archetype totals against the sums of its saved
/// amounts by archetype ([`compare_raid_equipment`]). Returns the totals
/// compared.
fn compare_raid_archetype_totals(
    row: &replay::rows::RaidEquipmentRow,
    saved: &[installed_content_save::production::Equipment],
    archetype_of: &BTreeMap<u64, u32>,
    label: &str,
    report: &mut Report,
) -> usize {
    /// Each pool operation clamps a total and an entry to this bound.
    const LIMIT: i128 = 0x53e2_d620_fb40;
    // Per archetype: the saved sum, and whether an entry sits at the bound.
    let mut expected: BTreeMap<u32, (i128, bool)> = BTreeMap::new();
    for entry in saved {
        let Some(&archetype) = archetype_of.get(&entry.variant) else {
            report.limitation(
                "a saved raid pool variant has no VariantDefinitions row (a variant created after the table's last read), so this raid's archetype totals are not derived",
                format!("{label}: variant {}", entry.variant),
            );
            return 0;
        };
        let amount = i128::from(entry.amount.raw);
        let sum = expected.entry(archetype).or_default();
        sum.0 += amount;
        sum.1 |= amount.abs() >= LIMIT;
    }
    let mut compared = 0;
    let mut seen = BTreeSet::new();
    for (&archetype, &total) in row.archetype_tokens.iter().zip(&row.archetype_amounts) {
        let field = format!("{label}.archetype[{archetype}]");
        if !seen.insert(archetype) {
            report.difference(field, "repeated archetype", "one total per archetype");
            continue;
        }
        let total_at_bound = i128::from(total).abs() >= LIMIT;
        match expected.remove(&archetype) {
            // A saved sum at or past the bound: the native total is clamped
            // to L, so a sum of exactly L can hide a clamp that later unwound
            // (3/4 L added to two variants, 1/4 L removed from each: entries
            // sum to L, native total 1/2 L).
            Some((sum, entry_at_bound)) if total_at_bound || entry_at_bound || sum.abs() >= LIMIT => {
                report.limitation(
                    "this raid pool archetype's recorded total, a contributing saved entry or their saved sum reaches the bound variant addition clamps a total and an entry to separately; the clamped total depends on the unsaved order of operations",
                    field,
                )
            }
            Some((sum, _)) => {
                compared += 1;
                if i128::from(total) != sum {
                    report.difference(field, total, sum);
                }
            }
            None if total_at_bound => report.limitation(
                "this raid pool archetype's recorded total reaches the bound variant addition clamps a total to; the clamped total depends on the unsaved order of operations",
                field,
            ),
            None if total != 0 => {
                compared += 1;
                report.difference(field, total, "no saved variant of this archetype");
            }
            // An archetype whose entries are zero keeps its zero total.
            None => {}
        }
    }
    for (archetype, (sum, _)) in expected {
        report.difference(format!("{label}.archetype[{archetype}]"), "absent", sum);
    }
    compared
}

/// RaidTooltipOperands against the saved non-ended raids: per-raid ordinals,
/// both reference witnesses, and saved army and wing scalars and name bytes.
/// The other families are runtime operands the save omits, reported as limitations.
pub(in crate::oracle) fn compare_raid_tooltip_operands(
    frame: &DecodedFrame,
    pack: &saved::Pack,
    divisions: &BTreeMap<u64, installed_content_save::Division>,
    air: &installed_content_save::air::Pack,
    report: &mut Report,
) -> io::Result<()> {
    use engine::raid_tooltip_operands::*;
    let (_, expected) = raid_census(pack, frame.hour, |raid, _| {
        raid.phase != engine::craid_instance::PHASE_ENDED
    })?;
    let rows = complete::<tables::RaidTooltipOperands>(frame)?;
    let mut ordinal = BTreeMap::<u64, u32>::new();
    let mut references = BTreeSet::new();
    for row in rows {
        let label = format!("RaidTooltipOperands {}:{}", row.raid, row.ordinal);
        let next = ordinal.entry(row.raid).or_default();
        equal(report, format!("{label}.ordinal"), row.ordinal, *next);
        *next = next
            .checked_add(1)
            .ok_or_else(|| invalid("raid tooltip ordinal overflow"))?;
        if row.raid as u32 == 0 {
            if !matches!(
                row.kind,
                ESSENTIAL_ORDER | ADDITIONAL_ORDER | SUBUNIT_ACTIVE
            ) {
                return Err(invalid(
                    "definition-scoped raid operand has non-definition family",
                ));
            }
            report.limitation("raid definition equipment-pointer order and global subunit active bytes are runtime-only; save serializes neither the definitions nor their pointers",label);
            continue;
        }
        let Some((raid, _)) = expected.get(&row.raid) else {
            missing(
                report,
                "RaidTooltipOperands",
                row.raid,
                "not a non-ended saved raid",
            );
            continue;
        };
        if row.kind == UNREADABLE {
            references.extend([(row.raid, 0), (row.raid, 1)]);
            report.limitation(
                "raid operands were unreadable when recorded; the viewer refuses that raid's operand lines",
                label,
            );
            continue;
        }
        if row.kind == WING_NAME {
            continue;
        }
        let value = match (row.kind, row.key) {
            (REFERENCES, 0) => {
                references.insert((row.raid, 0));
                Some(raid.army.unwrap_or(0) as i64)
            }
            (REFERENCES, 1) => {
                references.insert((row.raid, 1));
                Some(raid.air_wing.unwrap_or(0) as i64)
            }
            (REFERENCES, 2) => raid.source.as_ref().map(|s| s.ship.unwrap_or(0) as i64),
            (ARMY, 0x420 | 0x428) => {
                let unit = raid
                    .army
                    .and_then(|id| divisions.get(&id))
                    .ok_or_else(|| invalid("raid army operand has no saved army"))?;
                report.scalar(
                    &label,
                    "raw",
                    Some(row.raw),
                    Some(if row.key == 0x420 {
                        &unit.hit_points
                    } else {
                        &unit.organisation
                    }),
                );
                continue;
            }
            (WING, 0x218) => {
                let wing = air
                    .wings
                    .iter()
                    .find(|w| Some(w.id) == raid.air_wing)
                    .ok_or_else(|| invalid("raid wing operand has no saved wing"))?;
                report.scalar(&label, "raw", Some(row.raw), Some(&wing.experience));
                continue;
            }
            _ => None,
        };
        if let Some(expected) = value {
            equal(report, label, row.raw, expected);
        } else {
            report.limitation(&format!("RaidTooltipOperands family {} key {:#x}: runtime operand or naming provenance has no independent comparison in the text-save oracle",row.kind,row.key),label);
        }
    }
    for id in expected.keys() {
        for key in [0, 1] {
            if !references.contains(&(*id, key)) {
                missing(
                    report,
                    "RaidTooltipOperands reference witness",
                    (*id, key),
                    "absent from replay",
                );
            }
        }
    }
    compare_raid_wing_names(rows, &expected, air, report)?;
    report.compared("RaidTooltipOperands", rows.len());
    Ok(())
}

/// The game selects the wing only when its army fails resolution. The save
/// writes handles, not resolution, so require the recorded NAME witnesses.
/// The game writes the unit's stored name outside checksum
/// mode. The game escapes quotes and backslashes: compare
/// decoded bytes, including the final chunk's zero padding.
fn compare_raid_wing_names(
    rows: &[replay::rows::RaidTooltipOperandRow],
    expected: &RaidCensus<'_>,
    air: &installed_content_save::air::Pack,
    report: &mut Report,
) -> io::Result<()> {
    use engine::raid_tooltip_operands::*;
    let mut names = BTreeMap::<u64, BTreeMap<u32, i64>>::new();
    let mut resolution = BTreeMap::<u64, BTreeMap<u32, i64>>::new();
    let mut ambiguous_resolution = BTreeSet::new();
    let mut wings = BTreeMap::new();
    for wing in &air.wings {
        wings
            .entry(wing.id)
            .and_modify(|value| *value = None)
            .or_insert(Some(wing));
    }
    for row in rows {
        if !expected.contains_key(&row.raid) {
            continue;
        }
        if row.kind == NAME && matches!(row.key, ARMY_RESOLVED | WING_RESOLVED) {
            if resolution
                .entry(row.raid)
                .or_default()
                .insert(row.key, row.raw)
                .is_some()
            {
                ambiguous_resolution.insert(row.raid);
            }
        } else if row.kind == WING_NAME
            && names
                .entry(row.raid)
                .or_default()
                .insert(row.key, row.raw)
                .is_some()
        {
            return Err(invalid("duplicate raid wing-name operand key"));
        }
    }
    let ids: BTreeSet<_> = resolution.keys().chain(names.keys()).copied().collect();
    for id in ids {
        let label = format!("RaidTooltipOperands {id}.wing_name");
        let witnesses = resolution.get(&id);
        let selected = !ambiguous_resolution.contains(&id)
            && witnesses.and_then(|rows| rows.get(&ARMY_RESOLVED)) == Some(&0)
            && witnesses.and_then(|rows| rows.get(&WING_RESOLVED)) == Some(&1);
        let mut chunks = names.remove(&id).unwrap_or_default();
        let length = chunks.remove(&u32::MAX);
        let emitted = length.is_some() || !chunks.is_empty();
        if !selected {
            if emitted {
                report.difference(
                    &label,
                    "wing-name operands without selected-wing resolution witnesses",
                    "army unresolved and wing resolved",
                );
            }
            continue;
        }
        let Some(wing_id) = expected[&id].0.air_wing else {
            missing(
                report,
                "RaidTooltipOperands selected wing",
                id,
                "no saved wing reference",
            );
            continue;
        };
        let Some(wing) = wings.get(&wing_id) else {
            missing(
                report,
                "RaidTooltipOperands selected wing",
                wing_id,
                "absent from save",
            );
            continue;
        };
        let Some(wing) = wing else {
            report.difference(
                &label,
                "duplicate saved wing ID",
                "one saved wing per full reference",
            );
            continue;
        };
        if let Some(code) = length.filter(|code| *code < 0) {
            if !chunks.is_empty() {
                report.difference(
                    &label,
                    "capture failure together with chunks",
                    "capture failure replaces length and chunks",
                );
            }
            report.limitation(
                &format!("raid wing name unavailable: {}", wing_name_error(code)),
                label,
            );
            continue;
        }
        if length.is_none() {
            missing(
                report,
                "RaidTooltipOperands wing name length",
                id,
                "absent from replay",
            );
        }
        let Some(saved) = &wing.name else {
            missing(
                report,
                "RaidTooltipOperands saved wing name",
                id,
                "name is absent from a non-checksum text save",
            );
            continue;
        };
        if let Some(length) = length {
            equal(
                report,
                format!("{label}.length"),
                length,
                i64::try_from(saved.len())
                    .map_err(|_| invalid("saved wing name length exceeds i64"))?,
            );
        }
        let mut remaining = chunks;
        for (index, bytes) in saved.chunks(8).enumerate() {
            let key = u32::try_from(index)
                .map_err(|_| invalid("saved wing name chunk index exceeds u32"))?;
            let mut padded = [0; 8];
            padded[..bytes.len()].copy_from_slice(bytes);
            equal(
                report,
                format!("{label}.chunk[{index}]"),
                remaining.remove(&key),
                Some(i64::from_le_bytes(padded)),
            );
        }
        for (key, raw) in remaining {
            report.difference(
                format!("{label}.chunk[{key}]"),
                raw,
                "absent from saved name",
            );
        }
    }
    Ok(())
}

/// RaidSpeedInputs' census against the saved raids, both ways: every saved
/// raid the reader's rule ([`engine::raid_speed_inputs::live`]: phase 4, or
/// ended, not cancelled, at most an hour before the frame's hour) admits needs
/// a row, and every row needs such a saved raid; each row's phase, outcome and
/// end hour are the saved raid's. The speed operands themselves are live
/// getter inputs the raid writer does not save.
pub(super) fn compare_raid_speed_inputs(
    frame: &DecodedFrame,
    pack: &saved::Pack,
    report: &mut Report,
) -> io::Result<()> {
    let (raids, expected) = raid_census(pack, frame.hour, raid_flying)?;
    let mut recorded = BTreeSet::new();
    for row in complete::<tables::RaidSpeedInputs>(frame)? {
        let id = (u64::from(row.id) << 32) | u64::from(row.reference_type);
        if !recorded.insert(id) {
            return Err(invalid("duplicate raid speed input full reference"));
        }
        census_member(
            report,
            "RaidSpeedInputs",
            id,
            &raids,
            &expected,
            row.phase,
            Some(row.outcome),
            row.end_hour,
        );
    }
    for id in expected.keys().filter(|id| !recorded.contains(id)) {
        missing(report, "RaidSpeedInputs", id, "absent from replay");
    }
    report.compared("RaidSpeedInputs", recorded.len());
    report.limitation(
        "Raid unit speed inputs (wing and army maximum speeds, transport stats, the ace's own and type speed terms, its wing's plane count and the ace owner and wing country slots that join CountryCommandPower's ace country modifiers) are live getter operands the raid writer does not save; the census, phase, outcome and end hour are compared, and the travelled distance they advance is compared in Raids",
        "RaidSpeedInputs",
    );
    Ok(())
}

/// `RaidPlayerAllowedTypes` has no save counterpart: the player raid cache
/// is interface state that rebuilds on load and when the player changes. Only its shape is checked:
/// one cache at most.
pub(super) fn compare_raid_player(frame: &DecodedFrame, report: &mut Report) -> io::Result<()> {
    let rows = complete::<tables::RaidPlayerAllowedTypes>(frame)?;
    if rows.len() > 1 {
        report.difference("RaidPlayerAllowedTypes rows", rows.len(), 1);
    }
    report.limitation(
        "the player raid cache is rebuilt at load and on a player switch and is not saved",
        "RaidPlayerAllowedTypes",
    );
    Ok(())
}

pub(super) fn compare_raids(
    frame: &DecodedFrame,
    pack: &saved::Pack,
    countries: &BTreeMap<u32, &CountryRow>,
    tokens: &TokenNames<'_>,
    report: &mut Report,
) -> io::Result<()> {
    let expected = pack
        .raids
        .as_ref()
        .ok_or_else(|| invalid("save lacks raids root; absence does not certify empty graph"))?;
    let mut children = BTreeSet::new();
    let mut count = 0;
    let expected_children: BTreeMap<_, _> = expected
        .iter()
        .flat_map(|parent| {
            parent
                .instances
                .iter()
                .enumerate()
                .map(move |(ordinal, raid)| (raid.id, (parent.ordinal, ordinal, raid)))
        })
        .collect();
    for row in complete::<tables::Raids>(frame)? {
        let id = (u64::from(row.id) << 32) | u64::from(row.reference_type);
        if !children.insert(id) {
            return Err(invalid("duplicate raid full reference"));
        }
        let Some((parent, ordinal, saved)) = expected_children.get(&id) else {
            missing(report, "Raids", id, "absent from save");
            continue;
        };
        let label = format!("Raid {id}");
        equal(report, format!("{label}.parent"), row.parent_slot, *parent);
        equal(
            report,
            format!("{label}.ordinal"),
            u64::from(row.ordinal),
            *ordinal as u64,
        );
        equal(report, format!("{label}.phase"), row.phase, saved.phase);
        equal(
            report,
            format!("{label}.outcome"),
            row.outcome,
            saved.outcome,
        );
        let show_for = row
            .show_for
            .iter()
            .map(|&raw| signed_country(raw as i32, countries))
            .collect::<io::Result<Vec<_>>>()?;
        equal(
            report,
            format!("{label}.show_for"),
            show_for,
            saved
                .show_for
                .iter()
                .map(|tag| Some(tag.as_slice()))
                .collect(),
        );
        if row.source.kind_present > 1
            || (row.source.kind_present != 0) != row.source.kind.is_some()
        {
            return Err(invalid("raid source kind payload disagrees with presence"));
        }
        equal(
            report,
            format!("{label}.source_kind"),
            row.source.kind,
            saved.source.as_ref().map(|source| source.kind),
        );
        if let Some(source) = &saved.source {
            if source.province == Some(0) {
                return Err(invalid(
                    "saved raid source has zero id for nonnull province pointer",
                ));
            }
            equal(
                report,
                format!("{label}.source_province"),
                row.source.province,
                source.province.unwrap_or(0),
            );
            equal(
                report,
                format!("{label}.source_ship"),
                (u64::from(row.source.ship_reference_id) << 32)
                    | u64::from(row.source.ship_reference_type),
                source.ship.unwrap_or(0),
            );
            equal(
                report,
                format!("{label}.source_country"),
                signed_country(row.source.country, countries)?,
                source.country.as_deref(),
            );
            compare_source_building(report, &label, tokens, &row.source, source);
        } else {
            report.limitation("absent raid source kind suppresses the whole saved source; province, ship and country raw fields cannot be checked", label.clone());
        }
        equal(
            report,
            format!("{label}.unit"),
            row.unit,
            saved.army.unwrap_or(0),
        );
        equal(
            report,
            format!("{label}.air_wing"),
            row.air_wing,
            saved.air_wing.unwrap_or(0),
        );
        // The raid writer writes `prep_time` only when nonzero.
        equal(
            report,
            format!("{label}.preparation_hours"),
            row.preparation_hours,
            saved.preparation_hours,
        );
        // The raid writer writes `distance` as the exact raw / 100000 when nonzero.
        match row.travelled {
            Some(travelled) => equal(
                report,
                format!("{label}.travelled"),
                travelled,
                saved.travelled.as_ref().map_or(0, |distance| distance.raw),
            ),
            None => report.limitation(
                "the recording did not read this raid's travelled distance",
                format!("{label}.travelled"),
            ),
        }
        count += 1;
    }
    for id in expected_children.keys() {
        if !children.contains(id) {
            missing(report, "Raids", id, "absent from replay");
        }
    }
    report.compared("Raids", count);
    report.limitation(
        "dummy payloads, equipment payloads, dates and victims are outside this field set",
        "Raids and RaidCountries",
    );
    Ok(())
}

/// The source's building reference against the saved `raid_source.building`:
/// The writer emits it only when the building-reference test passes (a type
/// other than 0x4c17, and a location other than -1 or a positive country),
/// with the type token's literal as `template` and the location.
fn compare_source_building(
    report: &mut Report,
    label: &str,
    tokens: &TokenNames<'_>,
    source: &RaidSource,
    saved: &saved::RaidSource,
) {
    const NO_TYPE: u32 = 0x4c17;
    let set = source.building_type != NO_TYPE
        && (source.building_location != -1 || source.building_country > 0);
    match (set, &saved.building_template) {
        (false, None) => {}
        (true, Some(template)) => {
            match tokens.get(&source.building_type) {
                Some(literal) => equal(
                    report,
                    format!("{label}.source_building_type"),
                    *literal,
                    template.as_slice(),
                ),
                None => report.limitation(
                    "the source building type token is absent from the recorded lexical dictionary",
                    format!("{label}.source_building_type"),
                ),
            }
            equal(
                report,
                format!("{label}.source_building_location"),
                i64::from(source.building_location),
                saved.building_location.unwrap_or(-1),
            );
        }
        (set, saved) => report.difference(
            format!("{label}.source_building"),
            if set { "set" } else { "unset" },
            saved.as_ref().map_or("absent".to_owned(), |t| {
                String::from_utf8_lossy(t).into_owned()
            }),
        ),
    }
}

/// One recorded scope against the saved `target` block:
/// It contains `province`, `state`, `leader`,
/// `leader_province` and `building`, whose `template` is the
/// building's lexer token and whose `location` is its stored building location.
pub(super) fn compare_scope(
    report: &mut Report,
    label: &str,
    tokens: &TokenNames<'_>,
    scope: &RaidScope,
    saved: &saved::RaidTarget,
) {
    let arm = |id: u32| (id != 0).then_some(id);
    equal(
        report,
        format!("{label}.province"),
        arm(scope.province),
        saved.province,
    );
    equal(
        report,
        format!("{label}.state"),
        arm(scope.state),
        saved.state,
    );
    equal(
        report,
        format!("{label}.leader_province"),
        arm(scope.leader_province),
        saved.leader_province,
    );
    equal(
        report,
        format!("{label}.leader"),
        (scope.leader_reference != 0).then_some(scope.leader_reference),
        saved.leader,
    );
    compare_optional_token(
        report,
        &format!("{label}.building_template"),
        tokens,
        arm(scope.building_type),
        saved.building_template.as_deref(),
    );
    equal(
        report,
        format!("{label}.building_location"),
        (scope.building_type != 0).then_some(i64::from(scope.building_location as i32)),
        saved.building_location,
    );
    if scope.building_type != 0 {
        // The game writes a province location when bit 0 is
        // set, else location -1 and the country when the override is
        // positive, else a state location. Only those branches fix bit 0.
        let province = scope.building_flags & engine::cbuilding_status::PROVINCE_FLAG != 0;
        let wrong = match saved.building_location {
            Some(-1) => province,
            Some(_) => !province && scope.building_country > 0,
            None => false,
        };
        if wrong {
            report.difference(
                format!("{label}.building_flags"),
                scope.building_flags,
                format!(
                    "a status the writer saves with location {:?} (override {})",
                    saved.building_location, scope.building_country
                ),
            );
        }
        report.limitation(
            "the saved building reference writes a location without saying whether it is a province or a state, so the building location flags are checked only where the writer's branch fixes bit 0 (location -1, or a positive country override); a clear bit 0 with a state location and bit 1 and the other bits are not saved",
            format!("{label}.building_flags"),
        );
    }
}

/// `RaidTargets` against the saved `type`, `target` and `detected` fields.
pub(super) fn compare_raid_targets(
    frame: &DecodedFrame,
    pack: &saved::Pack,
    tokens: &TokenNames<'_>,
    report: &mut Report,
) -> io::Result<()> {
    let expected = pack
        .raids
        .as_ref()
        .ok_or_else(|| invalid("save lacks raids root; absence does not certify empty graph"))?;
    let expected_children: BTreeMap<_, _> = expected
        .iter()
        .flat_map(|parent| parent.instances.iter().map(|raid| (raid.id, raid)))
        .collect();
    let mut seen = BTreeSet::new();
    let mut count = 0;
    for row in complete::<tables::RaidTargets>(frame)? {
        let id = (u64::from(row.id) << 32) | u64::from(row.reference_type);
        if !seen.insert(id) {
            return Err(invalid("duplicate raid target reference"));
        }
        let Some(saved) = expected_children.get(&id) else {
            missing(report, "RaidTargets", id, "absent from save");
            continue;
        };
        let label = format!("RaidTarget {id}");
        equal(
            report,
            format!("{label}.detected"),
            row.detected != 0,
            saved.detected,
        );
        compare_optional_token(
            report,
            &format!("{label}.type"),
            tokens,
            (row.raid_type != 0).then_some(row.raid_type),
            saved.raid_type.as_deref(),
        );
        compare_scope(
            report,
            &label,
            tokens,
            &row.scope,
            &saved.target.clone().unwrap_or_default(),
        );
        count += 1;
    }
    for id in expected_children.keys() {
        if !seen.contains(id) {
            missing(report, "RaidTargets", id, "absent from replay");
        }
    }
    report.compared("RaidTargets", count);
    report.limitation(
        "the building country override and the leader's captor are not saved",
        "RaidTargets",
    );
    Ok(())
}

/// `RaidCooldowns` against each parent's saved `target_cooldowns`, in order.
pub(super) fn compare_raid_cooldowns(
    frame: &DecodedFrame,
    pack: &saved::Pack,
    tokens: &TokenNames<'_>,
    report: &mut Report,
) -> io::Result<()> {
    let expected = pack
        .raids
        .as_ref()
        .ok_or_else(|| invalid("save lacks raids root; absence does not certify empty graph"))?;
    let saved: BTreeMap<(u32, u32), &saved::RaidCooldown> = expected
        .iter()
        .flat_map(|parent| {
            parent
                .cooldowns
                .iter()
                .enumerate()
                .map(move |(ordinal, cooldown)| ((parent.ordinal, ordinal as u32), cooldown))
        })
        .collect();
    let mut seen = BTreeSet::new();
    for row in complete::<tables::RaidCooldowns>(frame)? {
        let key = (row.owner_slot, row.ordinal);
        seen.insert(key);
        let Some(cooldown) = saved.get(&key) else {
            missing(report, "RaidCooldowns", key, "absent from save");
            continue;
        };
        let label = format!("RaidCooldown {key:?}");
        compare_token(
            report,
            &format!("{label}.type"),
            tokens,
            row.raid_type,
            &cooldown.raid_type,
        );
        compare_scope(report, &label, tokens, &row.scope, &cooldown.target);
        equal(report, format!("{label}.days"), row.days, cooldown.days);
    }
    for key in saved.keys() {
        if !seen.contains(key) {
            missing(report, "RaidCooldowns", key, "absent from replay");
        }
    }
    report.compared("RaidCooldowns", seen.len());
    report.limitation(
        "the cooldown target is written by the raid-target writer, which saves the province, state, leader and leader_province arms and the building's template and location but not the building's raw country override (saved only as a tag in the location -1 branch) or the leader's captor",
        "RaidCooldowns.scope.building_country, RaidCooldowns.scope.leader_country",
    );
    Ok(())
}

/// `RaidTargetStatuses` against `raids.targets`, in order.
pub(super) fn compare_raid_target_statuses(
    frame: &DecodedFrame,
    pack: &saved::Pack,
    tokens: &TokenNames<'_>,
    report: &mut Report,
) -> io::Result<()> {
    let expected = pack
        .raid_targets
        .as_ref()
        .ok_or_else(|| invalid("save lacks raids.targets; absence does not certify no targets"))?;
    let rows = complete::<tables::RaidTargetStatuses>(frame)?;
    // The native ordinal is the row key and the saved entry's position.
    let mut seen = BTreeSet::new();
    for row in rows {
        if !seen.insert(row.ordinal) {
            return Err(invalid("duplicate raid target status ordinal"));
        }
        let Some(saved) = expected.get(row.ordinal as usize) else {
            missing(
                report,
                "RaidTargetStatuses",
                row.ordinal,
                "absent from save",
            );
            continue;
        };
        let label = format!("RaidTargetStatus {}", row.ordinal);
        equal(
            report,
            format!("{label}.valid"),
            row.valid != 0,
            saved.valid,
        );
        compare_scope(report, &label, tokens, &row.scope, &saved.target);
        equal(
            report,
            format!("{label}.detected"),
            &row.detected,
            &saved.detected,
        );
    }
    for ordinal in 0..expected.len() as u32 {
        if !seen.contains(&ordinal) {
            missing(report, "RaidTargetStatuses", ordinal, "absent from replay");
        }
    }
    report.compared(
        "RaidTargetStatuses",
        seen.iter()
            .filter(|&&ordinal| (ordinal as usize) < expected.len())
            .count(),
    );
    report.limitation(
        "the building country override and the leader's captor are not saved",
        "RaidTargetStatuses",
    );
    Ok(())
}

/// The content graph and geometry the raid path search runs on.
pub(in crate::oracle) type RaidGraph = (
    installed_content::province_graph::ProvinceGraph,
    installed_content::province_geometry::ProvinceGeometry,
);

/// `RaidPathDistances` has no save counterpart: the memo is session state.
/// Each entry must be the result of the content's naval or land raid search
/// for its pair, since only those searches fill it.
pub(in crate::oracle) fn compare_raid_paths(
    frame: &DecodedFrame,
    (graph, geometry): &RaidGraph,
    report: &mut Report,
) -> io::Result<()> {
    use installed_content::province_graph::{RaidPath, raid_distance};
    let rows = complete::<tables::RaidPathDistances>(frame)?;
    let mut compared = 0;
    for row in rows {
        let recorded = (row.ok != 0).then_some(row.distance);
        let label = format!("RaidPathDistance {}->{}", row.source, row.target);
        let searches = [RaidPath::Naval, RaidPath::Land].map(|path| {
            (
                path,
                raid_distance(graph, geometry, row.source, row.target, path),
            )
        });
        match path_verdict(recorded, &searches) {
            PathVerdict::Match => compared += 1,
            PathVerdict::Difference => {
                compared += 1;
                report.difference(label, recorded, searches);
            }
            PathVerdict::Undecided => report.limitation(
                "the content raid search cannot decide this pair, and no decided search reproduces the entry",
                format!("{label}: {searches:?}"),
            ),
        }
    }
    report.compared("RaidPathDistances", compared);
    report.limitation(
        "the raid path memo is not saved; entries are checked against the content's naval and land searches, not the save",
        "RaidPathDistances",
    );
    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum PathVerdict {
    Match,
    Difference,
    Undecided,
}

/// A memo entry matches when one decided search reproduces it, whatever the
/// other search decides. It differs only when both searches decided and
/// neither reproduces it: an entry can be the result of the search the
/// content cannot decide, so an undecidable search certifies nothing.
pub(super) fn path_verdict(
    recorded: Option<i64>,
    searches: &[(
        installed_content::province_graph::RaidPath,
        Result<Option<i64>, String>,
    )],
) -> PathVerdict {
    if searches
        .iter()
        .any(|(_, result)| result.as_ref() == Ok(&recorded))
    {
        PathVerdict::Match
    } else if searches.iter().all(|(_, result)| result.is_ok()) {
        PathVerdict::Difference
    } else {
        PathVerdict::Undecided
    }
}

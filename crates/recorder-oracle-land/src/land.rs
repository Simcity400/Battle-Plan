use super::*;
use content_reader_parser::parser::{Item, Value};

fn equal<T: std::fmt::Debug + PartialEq>(
    report: &mut Report,
    label: impl AsRef<str>,
    actual: T,
    expected: T,
) {
    if actual != expected {
        report.difference(label, actual, expected);
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct History {
    hour: i32,
    province: u32,
    attacker: Vec<u8>,
    defender: Vec<u8>,
    kind: i32,
}
#[derive(Default)]
struct Division {
    manpower: Vec<(Vec<u8>, i32)>,
}
#[derive(Default)]
pub struct Saved {
    history: Vec<History>,
    divisions: BTreeMap<u64, Division>,
    losses: BTreeMap<u64, [replay_data::rows::LandSideLosses; 2]>,
    allow_zero: BTreeMap<u64, [Option<bool>; 2]>,
}
fn field<'a>(items: &'a [Item], key: &[u8]) -> io::Result<Option<&'a Value>> {
    let mut found = items.iter().filter(|i| i.key.as_deref() == Some(key));
    let result = found.next().map(|i| &i.value);
    if found.next().is_some() {
        return Err(invalid(format!(
            "duplicate land save field {}",
            String::from_utf8_lossy(key)
        )));
    }
    Ok(result)
}
fn required<'a>(items: &'a [Item], key: &[u8]) -> io::Result<&'a Value> {
    field(items, key)?.ok_or_else(|| {
        invalid(format!(
            "missing land save field {}",
            String::from_utf8_lossy(key)
        ))
    })
}
fn block(v: &Value) -> io::Result<&[Item]> {
    v.items().ok_or_else(|| invalid("land save block expected"))
}
fn scalar(v: &Value) -> io::Result<&[u8]> {
    v.scalar()
        .ok_or_else(|| invalid("land save scalar expected"))
}
fn number<T: std::str::FromStr>(v: &Value) -> io::Result<T> {
    std::str::from_utf8(scalar(v)?)
        .ok()
        .and_then(|v| v.parse().ok())
        .ok_or_else(|| invalid("land save integer invalid"))
}
fn handle(v: &Value) -> io::Result<u64> {
    let i = block(v)?;
    let id: u32 = number(required(i, b"id")?)?;
    let kind: u32 = number(required(i, b"type")?)?;
    Ok(u64::from(id) << 32 | u64::from(kind))
}
fn division_handle(items: &[Item]) -> io::Result<u64> {
    let mut copies = items.iter().filter(|i| i.key.as_deref() == Some(b"id"));
    let first = handle(
        &copies
            .next()
            .ok_or_else(|| invalid("missing land save division id"))?
            .value,
    )?;
    for copy in copies {
        if handle(&copy.value)? != first {
            return Err(invalid("land save division id copies differ"));
        }
    }
    Ok(first)
}

fn saved_losses(side: &[Item]) -> io::Result<(replay_data::rows::LandSideLosses, Option<bool>)> {
    let log = block(required(side, b"log")?)?;
    let stats = block(required(log, b"combat_side_data")?)?;
    let manpower = field(stats, b"manpower_lost")?
        .map(number::<i32>)
        .transpose()?
        .unwrap_or(0);
    if manpower < 0 {
        return Err(invalid("saved manpower loss is not positive"));
    }
    let mut equipment = Vec::new();
    let mut allow_zero = None;
    if let Some(pool) = field(stats, b"equipment_lost")? {
        let pool = block(pool)?;
        allow_zero = Some(match scalar(required(pool, b"allow_zero_entries")?)? {
            b"yes" => true,
            b"no" => false,
            _ => return Err(invalid("invalid loss pool allow_zero_entries")),
        });
        for entry in pool
            .iter()
            .filter(|i| i.key.as_deref() == Some(b"equipment"))
        {
            let entry = block(&entry.value)?;
            equipment.push(replay_data::rows::LandEquipmentLoss {
                variant: handle(required(entry, b"id")?)?,
                amount_raw: Decimal::parse(scalar(required(entry, b"amount")?)?)?.raw,
            });
        }
    }
    Ok((
        replay_data::rows::LandSideLosses {
            manpower,
            equipment,
        },
        allow_zero,
    ))
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Comparison {
    Divisions,
    Losses,
    History,
    #[cfg(test)]
    All,
}
pub fn read(items: &[Item], comparison: Comparison) -> io::Result<Saved> {
    let mut out = Saved::default();
    if comparison != Comparison::Divisions {
        let combats = block(required(items, b"combat")?)?;
        if comparison != Comparison::History {
            for entry in combats.iter().filter(|i| {
                matches!(
                    i.key.as_deref(),
                    Some(b"land_combat" | b"border_war_combat")
                )
            }) {
                let fields = block(&entry.value)?;
                let id = handle(required(fields, b"id")?)?;
                let attacker = saved_losses(block(required(fields, b"attacker")?)?)?;
                let defender = saved_losses(block(required(fields, b"defender")?)?)?;
                if out.losses.insert(id, [attacker.0, defender.0]).is_some() {
                    return Err(invalid("duplicate saved combat losses"));
                }
                out.allow_zero.insert(id, [attacker.1, defender.1]);
            }
        }
        if comparison != Comparison::Losses
            && let Some(history) = field(combats, b"history")?
            && let Some(entries) = field(block(history)?, b"history")?
        {
            for entry in block(entries)? {
                if entry.key.is_some() {
                    return Err(invalid("keyed combat history entry"));
                }
                let i = block(&entry.value)?;
                out.history.push(History {
                    hour: save::GameDate::parse(scalar(required(i, b"end_date")?)?)?
                        .engine_hour()?,
                    province: number(required(i, b"location")?)?,
                    attacker: scalar(required(i, b"attacker")?)?.to_vec(),
                    defender: scalar(required(i, b"defender")?)?.to_vec(),
                    kind: number(required(i, b"type")?)?,
                });
            }
        }
    }
    if comparison != Comparison::Losses
        && comparison != Comparison::History
        && let Some(countries) = field(items, b"countries")?
    {
        read_division_countries(block(countries)?.iter(), &mut out)?;
    }
    Ok(out)
}

pub fn read_projected_divisions(
    document: &save::SaveDocument,
    frame: &DecodedFrame,
) -> io::Result<Saved> {
    let countries = country_items(document, frame)?;
    let mut out = Saved::default();
    read_division_countries(countries.into_iter(), &mut out)?;
    Ok(out)
}

fn read_division_countries<'a>(
    countries: impl Iterator<Item = &'a Item>,
    out: &mut Saved,
) -> io::Result<()> {
    for country in countries {
        if let Some(units) = field(block(&country.value)?, b"units")? {
            for army in block(units)?
                .iter()
                .filter(|i| i.key.as_deref() == Some(b"division"))
            {
                let i = block(&army.value)?;
                let id = division_handle(i)?;
                let mut division = Division::default();
                let manpower = block(required(i, b"army_manpower")?)?;
                let values = block(required(manpower, b"army_manpower_value")?)?;
                for v in values.iter().filter(|i| i.key.as_deref() == Some(b"value")) {
                    let v = block(&v.value)?;
                    division.manpower.push((
                        scalar(required(v, b"tag")?)?.to_vec(),
                        number(required(v, b"value")?)?,
                    ));
                }
                if out.divisions.insert(id, division).is_some() {
                    return Err(invalid("duplicate land save division"));
                }
            }
        }
    }
    Ok(())
}

pub fn compare_losses(frame: &DecodedFrame, saved: &Saved, report: &mut Report) -> io::Result<()> {
    let rows = frame.complete::<tables::LandCombatProgress>()?;
    let mut seen = BTreeSet::new();
    for row in rows {
        if !seen.insert(row.combat) {
            return Err(invalid("duplicate hourly combat losses"));
        }
        let Some(expected) = saved.losses.get(&row.combat) else {
            report.difference(
                format!("LandCombatProgress losses {}", row.combat),
                "present",
                "absent",
            );
            continue;
        };
        let manpower = [
            report.recorded(
                &row.attacker_manpower_loss,
                "LandCombatProgress.attacker_manpower_loss",
                row.combat.to_string(),
            ),
            report.recorded(
                &row.defender_manpower_loss,
                "LandCombatProgress.defender_manpower_loss",
                row.combat.to_string(),
            ),
        ];
        let pools = [
            report.recorded(
                &row.attacker_equipment_losses,
                "LandCombatProgress.attacker_equipment_losses",
                row.combat.to_string(),
            ),
            report.recorded(
                &row.defender_equipment_losses,
                "LandCombatProgress.defender_equipment_losses",
                row.combat.to_string(),
            ),
        ];
        for side in 0..2 {
            if let Some(&manpower) = manpower[side] {
                equal(
                    report,
                    format!("combat {} side {side} manpower_lost", row.combat),
                    manpower.max(0),
                    expected[side].manpower,
                );
                if manpower < 0 {
                    report.limitation("the game omits nonpositive manpower; the save cannot certify its negative magnitude", format!("combat {} side {side}", row.combat));
                }
            }
            if let Some(pool) = pools[side] {
                if let Some(allow_zero) = saved.allow_zero[&row.combat][side] {
                    let projected = pool
                        .iter()
                        .filter(|e| allow_zero || e.amount_raw != 0)
                        .cloned()
                        .collect::<Vec<_>>();
                    equal(
                        report,
                        format!("combat {} side {side} equipment_lost", row.combat),
                        &projected,
                        &expected[side].equipment,
                    );
                } else {
                    report.limitation("the game gates equipment_lost on archetype allocations, not variant entries; an omitted wrapper cannot certify an empty variant pool", format!("combat {} side {side}", row.combat));
                }
            }
        }
    }
    for id in saved.losses.keys().filter(|id| !seen.contains(id)) {
        report.difference(
            format!("LandCombatProgress losses {id}"),
            "absent",
            "present",
        );
    }
    report.compared("LandCombatProgress casualties", rows.len());
    report.limitation("the game certifies counters of combats live at the save hour through LandCombatProgress; ended history omits casualty pools, so a later save cannot certify LandCombatResults terminal amounts or flush/deletion coverage", "terminal losses require matched final combat-window evidence and hook tests");
    Ok(())
}

pub fn compare_divisions(
    frame: &DecodedFrame,
    saved: &Saved,
    report: &mut Report,
) -> io::Result<()> {
    let countries = country_context(frame)?;
    let rows = frame.complete::<tables::DivisionLandInputs>()?;
    let mut seen = BTreeSet::new();
    for row in rows {
        if !seen.insert(row.division) {
            return Err(invalid("duplicate DivisionLandInputs"));
        }
        let Some(expected) = saved.divisions.get(&row.division) else {
            report.difference(
                format!("DivisionLandInputs {}", row.division),
                "present",
                "absent",
            );
            continue;
        };
        let mut manpower = Vec::new();
        for entry in &row.manpower {
            if entry.amount > 0 {
                let tag = if entry.country == 0 {
                    None
                } else {
                    let country = countries
                        .links
                        .get(&entry.country)
                        .ok_or_else(|| invalid("unknown division manpower country reference"))?;
                    (country.id == entry.country).then(|| country.tag.clone())
                };
                if tag.is_none() {
                    report.limitation(
                        "the game saves the raw tag string; an alias/null contributor's spelling is not captured by Countries or CountryLinks. Amount and position are still compared",
                        format!("division {} positive contributor {} raw country {}", row.division, manpower.len(), entry.country),
                    );
                }
                manpower.push((tag, entry.amount));
            }
        }
        let saved_manpower = expected
            .manpower
            .iter()
            .enumerate()
            .map(|(n, (tag, amount))| {
                let tag = if manpower.get(n).is_some_and(|(tag, _)| tag.is_none()) {
                    None
                } else {
                    Some(tag.clone())
                };
                (tag, *amount)
            })
            .collect::<Vec<_>>();
        equal(
            report,
            format!("division {} manpower", row.division),
            &manpower,
            &saved_manpower,
        );
    }
    equal(
        report,
        "DivisionLandInputs census",
        seen,
        saved.divisions.keys().copied().collect::<BTreeSet<_>>(),
    );
    report.compared("DivisionLandInputs", rows.len());
    report.limitation("the manpower writer omits nonpositive contributors; their values and positions cannot be certified","DivisionLandInputs.manpower");
    report.limitation("The unit writer does not serialize naval-transfer membership; that hourly operand is unverified","DivisionLandInputs.naval_transfer");
    Ok(())
}

fn multiset(values: impl IntoIterator<Item = History>) -> BTreeMap<History, usize> {
    let mut out = BTreeMap::new();
    for v in values {
        *out.entry(v).or_default() += 1;
    }
    out
}
fn retained_span_count(spans: &[(i32, i32)], manager_hour: i32) -> usize {
    let oldest =
        i64::from(manager_hour) - i64::from(engine_land_inputs::ccombat_history::RETENTION_HOURS);
    spans
        .iter()
        .filter(|&&(from, to)| {
            i64::from(to) >= oldest && from < manager_hour
        })
        .count()
}
fn compare_certified_history(
    captured: Vec<History>,
    spans: &[(i32, i32)],
    saved: &[History],
    manager_hour: i32,
    report: &mut Report,
) -> io::Result<usize> {
    if spans.iter().any(|&(from, to)| from >= to) || spans.windows(2).any(|w| w[0].1 > w[1].0) {
        return Err(invalid("land event coverage overlaps or is invalid"));
    }
    if captured.iter().any(|e| {
        !spans
            .iter()
            .any(|&(from, to)| e.hour >= from && e.hour <= to)
    }) {
        return Err(invalid("land event outside certified history span"));
    }
    let steps = |a: i32, b: i32| spans.iter().any(|&(from, to)| from <= a && b <= to);
    let certified = |hour: i32| {
        steps(hour.saturating_sub(1), hour)
            && (hour == manager_hour || steps(hour, hour.saturating_add(1)))
    };
    let oldest =
        i64::from(manager_hour) - i64::from(engine_land_inputs::ccombat_history::RETENTION_HOURS);
    let retained = |e: &History| i64::from(e.hour) >= oldest;
    let outside = captured.iter().filter(|e| !retained(e)).count();
    if outside != 0 {
        report.limitation(
            "combat history retention: the game prunes results older than H_u - 168 (engine literal 0xA8); the save cannot certify these recorded results",
            format!("{outside} results before hour {oldest}; last manager update H_u={manager_hour}"),
        );
    }
    if captured.iter().any(|e| e.hour > manager_hour) {
        return Err(invalid("land result after last manager update"));
    }
    let saved = multiset(
        saved
            .iter()
            .filter(|e| e.kind == 1 && retained(e) && e.hour <= manager_hour)
            .cloned(),
    );
    let recorded = multiset(captured.into_iter().filter(retained));
    let count = recorded.values().sum();
    let whole = |map: &BTreeMap<History, usize>| {
        map.iter()
            .filter(|(e, _)| certified(e.hour))
            .map(|(e, n)| (e.clone(), *n))
            .collect::<BTreeMap<_, _>>()
    };
    equal(
        report,
        "LandCombatResults saved history multiset",
        whole(&recorded),
        whole(&saved),
    );
    for (e, &n) in recorded.iter().filter(|(e, _)| !certified(e.hour)) {
        let held = saved.get(e).copied().unwrap_or(0);
        if held < n {
            report.difference(
                format!(
                    "LandCombatResults recorded result missing from the saved history at hour {}",
                    e.hour
                ),
                n,
                held,
            );
        }
    }
    let mut unseen = 0;
    for (e, &n) in saved.iter().filter(|(e, _)| !certified(e.hour)) {
        unseen += n.saturating_sub(recorded.get(e).copied().unwrap_or(0));
    }
    if unseen != 0 {
        report.limitation(
            "a land result stamped with an hour whose drains are not complete on both sides of its read is only partly observed: every recorded one must be in the save, but a saved one there may never have been observed",
            format!("{unseen} saved results"),
        );
    }
    Ok(count)
}
pub fn compare_events(
    replay: &Replay,
    frame: &DecodedFrame,
    saved: &Saved,
    report: &mut Report,
) -> io::Result<()> {
    const SELECTED: [TableId; 3] = [
        TableId::LandCombatResults,
        TableId::DestroyedDivisions,
        TableId::LandEventCoverage,
    ];
    let mut captured = Vec::new();
    let mut spans = Vec::new();
    let mut destroyed = 0;
    let mut blocks = replay
        .index
        .iter()
        .filter(|b| b.branch == frame.branch && b.first_hour <= frame.hour)
        .collect::<Vec<_>>();
    blocks.sort_by_key(|b| b.first_hour);
    for b in blocks {
        replay.visit_blocks(
            &[&b],
            &SELECTED,
            |_, _, hour_frame| {
            if hour_frame.hour > frame.hour {
                return Ok(false);
            }
            let decoded = Ok::<&replay_frame::frame::DecodedFrame, std::io::Error>(hour_frame)?;
            let coverage = decoded.complete::<tables::LandEventCoverage>()?;
            let [c] = coverage else {
                return Err(invalid("land event coverage must have one row"));
            };
            if !c.complete {
                report.limitation("land events before attachment, across loads/gaps/recovery or failed observation are unknown",format!("hours {}..{} epoch {}",c.from_hour,c.to_hour,c.epoch));
                return Ok(true);
            }
            if c.to_hour != decoded.hour || c.from_hour >= c.to_hour {
                return Err(invalid("land event coverage bounds invalid"));
            }
            spans.push((c.from_hour, c.to_hour));
            for result in decoded.complete::<tables::LandCombatResults>()? {
                if result.end_hour < c.from_hour || result.end_hour > c.to_hour {
                    return Err(invalid("land event outside its observed interval"));
                }
                captured.push(History {
                    hour: result.end_hour,
                    province: result.province,
                    attacker: result.history_tags[0].clone(),
                    defender: result.history_tags[1].clone(),
                    kind: 1,
                });
            }
            destroyed += decoded.complete::<tables::DestroyedDivisions>()?.len();
                Ok(true)
            },
            || Ok(()),
        )?;
    }
    let compared = compare_certified_history(captured, &spans, &saved.history, frame.hour, report)?;
    for w in spans.windows(2).filter(|w| w[0].1 < w[1].0) {
        report.limitation(
            "no certified land event observations across this recording gap",
            format!("hours {}..{}", w[0].1, w[1].0),
        );
    }
    report.compared("LandCombatResults", compared);
    report.compared("LandEventCoverage", retained_span_count(&spans, frame.hour));
    report.limitation("The combat-history writer saves location, attacker, defender, end_date and type, but no winner, commander, full country sets, start date, duration or raw outcome inputs","LandCombatResults");
    report.limitation("the game writes UNIT_KILL_HISTORY on the dying army, gated by unit-death processing; it deletes that army. No corresponding surviving killer-unit entry is proved; this event has no certified save comparator",format!("DestroyedDivisions: {destroyed} observed events, unverified"));
    Ok(())
}

pub fn compare_damage(replay: &Replay, frame: &DecodedFrame, report: &mut Report) -> io::Result<()> {
    const SELECTED: [TableId; 5] = [
        TableId::LandDivisionDamage,
        TableId::LandCombatResultObservers,
        TableId::LandCombatEdges,
        TableId::DestroyedDivisions,
        TableId::LandEventCoverage,
    ];
    let mut recorded = Recorded::default();
    let mut blocks = replay
        .index
        .iter()
        .filter(|b| b.branch == frame.branch && b.first_hour <= frame.hour)
        .collect::<Vec<_>>();
    blocks.sort_by_key(|b| b.first_hour);
    for b in blocks {
        replay.visit_blocks(
            &[&b],
            &SELECTED,
            |_, _, decoded| {
                if decoded.hour > frame.hour {
                    return Ok(false);
                }
                let [coverage] = decoded.complete::<tables::LandEventCoverage>()? else {
                    return Err(invalid("land event coverage must have one row"));
                };
                let read = (
                    decoded.complete::<tables::LandDivisionDamage>(),
                    decoded.complete::<tables::LandCombatResultObservers>(),
                    decoded.complete::<tables::LandCombatEdges>(),
                    decoded.complete::<tables::DestroyedDivisions>(),
                );
                match read {
                    (Ok(damage), Ok(observers), Ok(edges), Ok(destroyed)) if coverage.complete => {
                        recorded.damage.extend_from_slice(damage);
                        recorded.observers.extend_from_slice(observers);
                        for edge in edges {
                            if edge.kind == replay_data::rows::LandCombatEdgesRow::OPENED {
                                recorded.opened.entry(edge.combat).or_insert(decoded.hour);
                            } else if let Some(division) = edge.division {
                                recorded.exits.insert(edge.id, (division, edge.pending_strength_raw));
                            }
                        }
                        for row in destroyed.iter().filter(|row| row.triggering_combat.is_some()) {
                            recorded.wiped.extend(row.combats.iter().chain(row.triggering_combat.as_ref()).map(|&combat| (combat, row.division)));
                        }
                        for observer in observers {
                            recorded.ended.insert(observer.combat, decoded.hour);
                        }
                    }
                    (damage, ..) => {
                        recorded.unrecorded.insert(decoded.hour);
                        if let Err(error) = damage {
                            report.limitation("division damage was not recorded for this hour", format!("hour {}: {error}", decoded.hour));
                        }
                    }
                }
                Ok(true)
            },
            || Ok(()),
        )?;
    }
    check_damage(&recorded, report);
    Ok(())
}

#[derive(Default)]
struct Recorded {
    damage: Vec<replay_data::rows::LandDivisionDamageRow>,
    observers: Vec<replay_data::rows::CombatLogObserverRow>,
    opened: BTreeMap<u64, i32>,
    ended: BTreeMap<u64, i32>,
    wiped: BTreeSet<(u64, u64)>,
    unrecorded: BTreeSet<i32>,
    exits: BTreeMap<u64, (u64, Option<i64>)>,
}

fn check_damage(recorded: &Recorded, report: &mut Report) {
    use replay_data::rows::CombatLogObserverKind as K;
    let whole = |combat: u64| match (recorded.opened.get(&combat), recorded.ended.get(&combat)) {
        (Some(&opened), Some(&ended)) => recorded.unrecorded.range(opened..=ended).next().is_none(),
        _ => false,
    };
    let mut groups = BTreeMap::new();
    let mut expected = BTreeMap::<(u64, u64, u64), i64>::new();
    for row in &recorded.observers {
        if row.kind == K::Activity as u8 {
            groups.insert((row.combat, row.attacker, row.item), row.identity);
        }
    }
    for row in recorded.observers.iter().filter(|row| row.kind == K::ActivityDamage as u8) {
        let Some(&group) = groups.get(&(row.combat, row.attacker, row.item)) else {
            report.difference(format!("LandCombatResultObservers combat {} activity {}", row.combat, row.item), "damage without its activity", "an activity naming its army");
            continue;
        };
        *expected.entry((row.combat, group, row.identity)).or_default() += row.a;
    }
    let mut actual = BTreeMap::<(u64, u64, u64), i64>::new();
    let mut armyless = 0;
    for row in &recorded.damage {
        match row.army {
            Some(army) => *actual.entry((row.combat, army, row.target)).or_default() += row.strength_raw,
            None => armyless += 1,
        }
    }
    let (mut compared, mut partial) = (0, BTreeSet::new());
    for key in expected.keys().chain(actual.keys()).copied().collect::<BTreeSet<_>>() {
        let (combat, army, target) = key;
        if !recorded.ended.contains_key(&combat) {
            continue;
        }
        if !whole(combat) {
            partial.insert(combat);
            continue;
        }
        if recorded.wiped.contains(&(combat, target)) {
            continue;
        }
        compared += 1;
        let (recorded_damage, game) = (actual.get(&key).copied().unwrap_or(0), expected.get(&key).copied().unwrap_or(0));
        if recorded_damage != game {
            report.difference(
                format!("LandDivisionDamage combat {combat} army {army} target {target} strength"),
                recorded_damage,
                format!("{game}, the game's own activity damage for that army and target"),
            );
        }
    }
    report.compared("LandDivisionDamage per army and target", compared);
    let mut dealt = BTreeMap::<(u64, u64), i64>::new();
    for row in &recorded.damage {
        *dealt.entry((row.id >> 16, row.target)).or_default() += row.strength_raw;
    }
    for (&(exit, target), &strength) in &dealt {
        match recorded.exits.get(&exit) {
            None => report.difference(format!("LandDivisionDamage target {target} exit {exit}"), "damage published", "only when its target leaves a combat"),
            Some((division, _)) if *division != target => report.difference(format!("LandDivisionDamage exit {exit}"), target, format!("{division}, the division that left")),
            Some((_, Some(pool))) if strength > *pool => report.difference(
                format!("LandDivisionDamage exit {exit} target {target} strength"),
                strength,
                format!("at most {pool}, the strength damage pending on it when it left"),
            ),
            Some(_) => {}
        }
    }
    report.compared("LandDivisionDamage within its target's pending strength damage", dealt.len());
    if !partial.is_empty() {
        report.limitation("a battle not recorded from its opening to its end has damage from before or between recorded hours; it is not compared", format!("{} battles", partial.len()));
    }
    if armyless > 0 {
        report.limitation("the game logs no activity for a division in no army, so its damage has nothing to compare with", format!("{armyless} rows"));
    }
    report.limitation("a division destroyed inside its battle never leaves it, so its damage is never published; the game's activity for it is not compared", "LandDivisionDamage");
}

pub fn compare_edges(
    replay: &Replay,
    frame: &DecodedFrame,
    killed: &BTreeMap<u64, i32>,
    report: &mut Report,
) -> io::Result<()> {
    const SELECTED: [TableId; 2] = [TableId::LandCombatEdges, TableId::LandEventCoverage];
    let mut rows = Vec::new();
    let mut blocks = replay
        .index
        .iter()
        .filter(|b| b.branch == frame.branch && b.first_hour <= frame.hour)
        .collect::<Vec<_>>();
    blocks.sort_by_key(|b| b.first_hour);
    for b in blocks {
        replay.visit_blocks(
            &[&b],
            &SELECTED,
            |_, _, decoded| {
                if decoded.hour > frame.hour {
                    return Ok(false);
                }
                let [coverage] = decoded.complete::<tables::LandEventCoverage>()? else {
                    return Err(invalid("land event coverage must have one row"));
                };
                if !coverage.complete {
                    return Ok(true);
                }
                match decoded.complete::<tables::LandCombatEdges>() {
                    Ok(edges) => rows.extend_from_slice(edges),
                    Err(error) => report.limitation(
                        "land combat edges were not recorded for this hour",
                        format!("hour {}: {error}", decoded.hour),
                    ),
                }
                Ok(true)
            },
            || Ok(()),
        )?;
    }
    check_edges(&rows, killed, report);
    Ok(())
}

fn check_edges(rows: &[replay_data::rows::LandCombatEdgesRow], killed: &BTreeMap<u64, i32>, report: &mut Report) {
    let mut latest: BTreeMap<u64, i32> = BTreeMap::new();
    for row in rows {
        let Some(division) = row.division else { continue };
        if let (Some(before), Some(after)) = (row.casualties_before, row.casualties_after)
            && after < before
        {
            report.difference(
                format!("LandCombatEdges {} division {division} casualties at exit", row.id),
                after,
                format!("at least {before}, the count before its loss flush"),
            );
        }
        for value in [row.casualties_before, row.casualties_after].into_iter().flatten() {
            let held = latest.entry(division).or_insert(value);
            *held = (*held).max(value);
        }
    }
    for (&division, &recorded) in &latest {
        let Some(&saved) = killed.get(&division) else { continue };
        if saved < recorded {
            report.difference(
                format!("LandCombatEdges division {division} casualties"),
                recorded,
                format!("at most {saved}, the saved lifetime casualties"),
            );
        }
    }
    report.compared("LandCombatEdges casualties", latest.len());
    report.limitation(
        "the save keeps each division's lifetime casualties, not the count at each combat edge; it bounds the recorded counts but cannot certify one edge or an opening roster",
        "LandCombatEdges",
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn division_damage_summed_per_army_must_equal_the_games_activity_damage_for_whole_battles() {
        use replay_data::rows::{CombatLogObserverKind as K, CombatLogObserverRow, LandDivisionDamageRow};
        let activity = |combat, attacker, item, group| CombatLogObserverRow { combat, attacker, kind: K::Activity as u8, item, identity: group, ..Default::default() };
        let damage = |combat, attacker, item, target, a| CombatLogObserverRow { combat, attacker, kind: K::ActivityDamage as u8, item, identity: target, a, ..Default::default() };
        let row = |combat, target, attacker, army, strength_raw| LandDivisionDamageRow { id: target << 16, combat, target, attacker, army, strength_raw, ..Default::default() };
        let exits = BTreeMap::from([(5, (5, Some(2_000))), (6, (6, Some(40))), (1, (1, None))]);
        let mut recorded = Recorded {
            observers: vec![activity(9, true, 0, 70), damage(9, true, 0, 5, 1_500), damage(9, true, 0, 6, 40), activity(9, false, 0, 71), damage(9, false, 0, 1, 300)],
            damage: vec![row(9, 5, 2, Some(70), 1_000), row(9, 5, 3, Some(70), 500), row(9, 6, 2, Some(70), 40), row(9, 1, 5, Some(71), 300), row(9, 1, 6, None, 77)],
            opened: BTreeMap::from([(9, 10)]),
            ended: BTreeMap::from([(9, 20)]),
            exits,
            ..Default::default()
        };
        let mut report = Report::default();
        check_damage(&recorded, &mut report);
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        recorded.damage[1].strength_raw = 499;
        recorded.damage.remove(2);
        let mut wrong = Report::default();
        check_damage(&recorded, &mut wrong);
        assert_eq!(wrong.differences.len(), 2, "a lost hit and a missing target are both differences: {:?}", wrong.differences);
        recorded.wiped.insert((9, 6));
        recorded.unrecorded.insert(15);
        let mut gapped = Report::default();
        check_damage(&recorded, &mut gapped);
        assert!(gapped.differences.is_empty(), "a battle with an unrecorded hour is not compared: {:?}", gapped.differences);
        recorded.exits.insert(5, (5, Some(1_000)));
        recorded.exits.remove(&6);
        let mut overdrawn = Report::default();
        check_damage(&recorded, &mut overdrawn);
        assert_eq!(overdrawn.differences.len(), 1, "more damage than the target's pending strength damage is a difference: {:?}", overdrawn.differences);
    }
    #[test]
    fn edge_casualties_above_the_saved_lifetime_count_or_falling_at_an_exit_are_differences() {
        use replay_data::rows::LandCombatEdgesRow;
        let edge = |id, division, before, after| LandCombatEdgesRow {
            id,
            kind: LandCombatEdgesRow::EXITED,
            division: Some(division),
            casualties_before: before,
            casualties_after: after,
            ..Default::default()
        };
        let killed = BTreeMap::from([(5, 812), (6, 40)]);
        let mut report = Report::default();
        check_edges(&[edge(1, 5, Some(300), Some(812)), edge(2, 6, Some(40), None)], &killed, &mut report);
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        let mut wrong = Report::default();
        check_edges(&[edge(1, 5, Some(300), Some(900)), edge(2, 6, Some(41), Some(40))], &killed, &mut wrong);
        assert_eq!(wrong.differences.len(), 3, "{:?}", wrong.differences);
    }
    fn read(bytes: &[u8]) -> io::Result<Saved> {
        super::read(
            &content_reader_parser::parser::parse(bytes)?.items,
            Comparison::All,
        )
    }
    #[test]
    fn land_save_comparisons_refuse_only_their_own_missing_fields() {
        let bytes = br#"combat={land_combat={id={id=7 type=62} attacker={} defender={}}
            history={history={{location=9 attacker=A defender=B end_date=1936.1.2.1 type=1}}}}
            countries={A={units={division={id={id=9 type=4713} max_supply=2
                army_current_supply_ratio=1 army_manpower={army_manpower_value={}}}}}}"#;
        let doc = content_reader_parser::parser::parse(bytes).unwrap();
        assert_eq!(
            super::read(&doc.items, Comparison::Divisions)
                .unwrap()
                .divisions
                .len(),
            1
        );
        assert_eq!(
            super::read(&doc.items, Comparison::History)
                .unwrap()
                .history
                .len(),
            1
        );
        assert!(
            super::read(&doc.items, Comparison::Losses)
                .err()
                .unwrap()
                .to_string()
                .contains("missing land save field log")
        );
        let bytes = br#"combat={land_combat={id={id=7 type=62}
            attacker={log={combat_side_data={}}} defender={log={combat_side_data={}}}}}
            countries={A={units={division={id={id=9 type=4713}}}}}"#;
        let doc = content_reader_parser::parser::parse(bytes).unwrap();
        assert!(
            super::read(&doc.items, Comparison::Divisions)
                .err()
                .unwrap()
                .to_string()
                .contains("missing land save field army_manpower")
        );
        assert_eq!(
            super::read(&doc.items, Comparison::Losses)
                .unwrap()
                .losses
                .len(),
            1
        );
        assert!(
            super::read(&doc.items, Comparison::History)
                .unwrap()
                .history
                .is_empty()
        );
        let doc = content_reader_parser::parser::parse(b"countries={}").unwrap();
        assert!(super::read(&doc.items, Comparison::Divisions).is_ok());
        assert!(super::read(&doc.items, Comparison::History).is_err());
        assert!(super::read(&doc.items, Comparison::Losses).is_err());
    }
    fn loss_frame(row: Option<replay_data::rows::LandCombatProgressRow>) -> DecodedFrame {
        let mut tables = std::array::from_fn(|_| {
            TableView::Unavailable {
                reason: "fixture omitted".into(),
            }
            .into()
        });
        tables[TableId::LandCombatProgress.index()] =
            TableView::Complete(TableRows::LandCombatProgress(row.into_iter().collect())).into();
        DecodedFrame {
            branch: 1,
            hour: 80,
            tables,
            metrics: Default::default(),
        }
    }
    #[test]
    fn casualty_save_oracle_compares_live_manpower_and_native_variant_pool_projection() {
        let bytes = br#"combat={land_combat={id={id=7 type=62}
            attacker={log={combat_side_data={manpower_lost=83 equipment_lost={equipment={id={id=9 type=70} amount=1.23456} allow_zero_entries=no}}}}
            defender={log={combat_side_data={equipment_lost={equipment={id={id=10 type=70} amount=0} allow_zero_entries=yes}}}}
        }}"#;
        let saved = read(bytes).unwrap();
        let mut row = replay_data::rows::LandCombatProgressRow {
            combat: 7 << 32 | 62,
            attacker_manpower_loss: 83.into(),
            attacker_equipment_losses: vec![
                replay_data::rows::LandEquipmentLoss {
                    variant: 9 << 32 | 70,
                    amount_raw: 123_456,
                },
                replay_data::rows::LandEquipmentLoss {
                    variant: 10 << 32 | 70,
                    amount_raw: 0,
                },
            ]
            .into(),
            defender_equipment_losses: vec![replay_data::rows::LandEquipmentLoss {
                variant: 10 << 32 | 70,
                amount_raw: 0,
            }]
            .into(),
            ..Default::default()
        };
        let mut report = Report::default();
        compare_losses(&loss_frame(Some(row.clone())), &saved, &mut report).unwrap();
        assert!(report.differences.is_empty());
        *row.attacker_manpower_loss.require_mut("fixture").unwrap() += 1;
        row.attacker_equipment_losses
            .require_mut("fixture")
            .unwrap()[0]
            .amount_raw -= 1;
        let mut mismatch = Report::default();
        compare_losses(&loss_frame(Some(row)), &saved, &mut mismatch).unwrap();
        assert_eq!(mismatch.differences.len(), 2);
        let mut absent = Report::default();
        compare_losses(&loss_frame(None), &saved, &mut absent).unwrap();
        assert_eq!(absent.differences.len(), 1);
        assert!(read(b"combat={land_combat={id={id=7 type=62} attacker={} defender={}}}").is_err());
    }
    #[test]
    fn casualty_save_oracle_keeps_not_recorded_fields_separate_from_zero() {
        use replay_data::{rows::LandCombatProgressRow, rows::Recorded};
        let saved = read(b"combat={land_combat={id={id=7 type=62} attacker={log={combat_side_data={manpower_lost=83}}} defender={log={combat_side_data={}}}}}").unwrap();
        let mut row = LandCombatProgressRow {
            combat: 7 << 32 | 62,
            attacker_manpower_loss: Recorded::NotRecorded,
            defender_manpower_loss: Recorded::NotRecorded,
            attacker_equipment_losses: Recorded::NotRecorded,
            defender_equipment_losses: Recorded::NotRecorded,
            ..Default::default()
        };
        let mut report = Report::default();
        compare_losses(&loss_frame(Some(row.clone())), &saved, &mut report).unwrap();
        assert!(report.differences.is_empty());
        assert_eq!(
            report
                .limitations
                .keys()
                .filter(|name| name.ends_with("not recorded"))
                .count(),
            4
        );
        row.defender_manpower_loss = 1.into();
        let mut mismatch = Report::default();
        compare_losses(&loss_frame(Some(row)), &saved, &mut mismatch).unwrap();
        assert_eq!(
            mismatch.differences.len(),
            1,
            "available fields still compare"
        );
    }
    #[test]
    fn casualty_save_omissions_and_ended_history_are_explicit_limitations() {
        let saved = read(b"combat={border_war_combat={id={id=7 type=62} attacker={log={combat_side_data={}}} defender={log={combat_side_data={}}}}}").unwrap();
        let row = replay_data::rows::LandCombatProgressRow {
            combat: 7 << 32 | 62,
            attacker_manpower_loss: (-7).into(),
            attacker_equipment_losses: vec![replay_data::rows::LandEquipmentLoss {
                variant: 9 << 32 | 70,
                amount_raw: 123_456,
            }]
            .into(),
            ..Default::default()
        };
        let mut report = Report::default();
        compare_losses(&loss_frame(Some(row)), &saved, &mut report).unwrap();
        assert!(report.differences.is_empty());
        assert!(
            report
                .limitations
                .keys()
                .any(|k| k.contains("negative magnitude"))
        );
        assert!(
            report
                .limitations
                .keys()
                .any(|k| k.contains("omitted wrapper"))
        );
        assert!(
            report
                .limitations
                .keys()
                .any(|k| k.contains("ended history"))
        );
        let ended = read(b"combat={history={history={{location=9 attacker=A defender=B end_date=1936.1.2.1 type=1}}}}").unwrap();
        let mut report = Report::default();
        compare_losses(&loss_frame(None), &ended, &mut report).unwrap();
        assert!(report.differences.is_empty());
        assert!(
            ended.losses.is_empty(),
            "ended history is not zero terminal losses"
        );
    }
    #[test]
    fn history_and_manpower_parse_native_structure_and_keep_duplicates() {
        let bytes=br#"combat={ history={ history={ { location=7 attacker=AAA defender=BBB end_date=1936.1.2.1 type=1 } { location=7 attacker=AAA defender=BBB end_date=1936.1.2.1 type=1 } } } } countries={ AAA={ units={ division={ id={id=9 type=4713} max_supply=2.00000 army_current_supply_ratio=1.23456 army_manpower={ army_manpower_value={ value={tag=AAA value=12} value={tag=BBB value=19} } } } } } }"#;
        let saved = read(bytes).unwrap();
        assert_eq!(saved.history.len(), 2);
        assert_eq!(*multiset(saved.history).values().next().unwrap(), 2);
        let division = &saved.divisions[&(9 << 32 | 4713)];
        assert_eq!(
            division.manpower,
            [(b"AAA".to_vec(), 12), (b"BBB".to_vec(), 19)]
        );
        assert!(read(b"countries={AAA={units={division={id={id=9 type=4713}}}}}").is_err());
    }
    #[test]
    fn land_divisions_leave_the_saved_grace_to_division_status() {
        let division = br#"combat={} countries={ opaque={ units={ division={
            id={id=9 type=4713} army_manpower={army_manpower_value={}}
        } } } }"#;
        let saved = read(division).unwrap();
        assert!(saved.divisions[&(9 << 32 | 4713)].manpower.is_empty());
        let malformed = String::from_utf8(division.to_vec())
            .unwrap()
            .replace("army_manpower={", "max_supply=bad army_manpower={");
        assert!(read(malformed.as_bytes()).is_ok());
    }
    #[test]
    fn history_is_two_way_duplicate_sensitive_and_excludes_unknown_hours() {
        let event = History {
            hour: 100,
            province: 9,
            attacker: b"opaque-A".to_vec(),
            defender: b"opaque-B".to_vec(),
            kind: 1,
        };
        let mut later = event.clone();
        later.hour = 102;
        let saved = vec![event.clone(), event.clone(), later];
        let mut report = Report::default();
        assert_eq!(
            compare_certified_history(
                vec![event.clone(), event.clone()],
                &[(99, 100)],
                &saved,
                100,
                &mut report
            )
            .unwrap(),
            2
        );
        assert!(report.differences.is_empty());
        for (recorded, saved) in [
            (vec![event.clone()], vec![event.clone(), event.clone()]),
            (vec![event.clone(), event.clone()], vec![event.clone()]),
        ] {
            let mut report = Report::default();
            compare_certified_history(recorded, &[(99, 100)], &saved, 100, &mut report).unwrap();
            assert_eq!(report.differences.len(), 1);
        }
        assert!(
            compare_certified_history(vec![event.clone()], &[], &saved, 100, &mut Report::default())
                .is_err()
        );
        let mut before = event.clone();
        before.hour = 99;
        let mut report = Report::default();
        assert_eq!(
            compare_certified_history(vec![before.clone()], &[(99, 100)], &[], 100, &mut report)
                .unwrap(),
            1
        );
        assert_eq!(
            report.differences.len(),
            1,
            "hour 99 before its read is unobserved, yet a result recorded there must be in the save"
        );
        let mut report = Report::default();
        compare_certified_history(
            vec![before.clone()],
            &[(99, 100)],
            &[before.clone(), before.clone()],
            100,
            &mut report,
        )
        .unwrap();
        assert!(
            report.differences.is_empty() && !report.limitations.is_empty(),
            "the recorded one is found; the other saved one may never have been observed"
        );
        let mut report = Report::default();
        assert_eq!(
            compare_certified_history(
                vec![before.clone(), event.clone()],
                &[(98, 99), (99, 100)],
                &[before, event.clone()],
                100,
                &mut report
            )
            .unwrap(),
            2,
            "drains on both sides of its read make hour 99 whole"
        );
        assert!(report.differences.is_empty());
        let mut report = Report::default();
        let gap_end = History {
            hour: 98,
            ..event.clone()
        };
        compare_certified_history(vec![], &[(97, 98), (99, 100)], &[gap_end], 100, &mut report)
            .unwrap();
        assert!(
            report.differences.is_empty(),
            "without the drain after its read, hour 98's later commands are unobserved"
        );
        assert!(
            compare_certified_history(
                vec![History { hour: 98, ..event }],
                &[(99, 100)],
                &[],
                100,
                &mut Report::default()
            )
            .is_err(),
            "a result older than its drain's first hour"
        );
        assert!(
            compare_certified_history(
                vec![],
                &[(99, 100), (99, 101)],
                &saved,
                100,
                &mut Report::default()
            )
            .is_err()
        );
        assert!(
            read(b"countries={}").is_err(),
            "absent combat root cannot certify zero events"
        );
    }
    #[test]
    fn duplicate_division_ids_must_all_agree() {
        let bytes = br#"combat={} countries={ opaque={ units={ division={
            id={id=37 type=4713} seed=91 id={id=37 type=4713}
            max_supply=7 army_current_supply_ratio=7
            army_manpower={army_manpower_value={value={tag=opaque value=23}}}
        } } } }"#;
        assert_eq!(read(bytes).unwrap().divisions.len(), 1);
        let unequal = String::from_utf8(bytes.to_vec())
            .unwrap()
            .replace("seed=91 id={id=37", "seed=91 id={id=38");
        assert!(read(unequal.as_bytes()).is_err());
    }
    #[test]
    #[ignore = "requires BATTLEPLAN_LAND_SAVE and BATTLEPLAN_LAND_EXPECTATIONS"]
    fn installed_save_divisions_and_history_match_expectations() {
        let path = std::path::PathBuf::from(
            std::env::var_os("BATTLEPLAN_LAND_SAVE").or_else(|| std::env::var_os("HOI4_LAND_SAVE")).expect("set BATTLEPLAN_LAND_SAVE to a text save"),
        );
        let expectations_path = std::path::PathBuf::from(
            std::env::var_os("BATTLEPLAN_LAND_EXPECTATIONS").or_else(|| std::env::var_os("HOI4_LAND_EXPECTATIONS"))
                .expect("set BATTLEPLAN_LAND_EXPECTATIONS to the expected-value JSON file"),
        );
        let expected: serde_json::Value = serde_json::from_slice(
            &std::fs::read(expectations_path).expect("read land expectations"),
        )
        .expect("parse land expectations");
        let saved = read(&std::fs::read(path).unwrap()).unwrap();
        assert!(!saved.divisions.is_empty());
        let id = u32::try_from(expected["division_id"].as_u64().expect("division_id"))
            .expect("division_id fits u32");
        let kind = u32::try_from(expected["division_type"].as_u64().expect("division_type"))
            .expect("division_type fits u32");
        let division = &saved.divisions[&(u64::from(id) << 32 | u64::from(kind))];
        let manpower: Vec<_> = expected["manpower"]
            .as_array()
            .expect("manpower array")
            .iter()
            .map(|entry| {
                (
                    entry[0].as_str().expect("manpower tag").as_bytes().to_vec(),
                    i32::try_from(entry[1].as_i64().expect("manpower amount"))
                        .expect("manpower amount fits i32"),
                )
            })
            .collect();
        assert_eq!(division.manpower, manpower);
        assert_eq!(
            saved.history.len(),
            usize::try_from(expected["history_count"].as_u64().expect("history_count"))
                .expect("history_count fits usize")
        );
        let hour = |date: &[u8]| save::GameDate::parse(date).unwrap().engine_hour().unwrap();
        let first_date = hour(
            expected["first_date"]
                .as_str()
                .expect("first_date")
                .as_bytes(),
        );
        let last_date = hour(
            expected["last_date"]
                .as_str()
                .expect("last_date")
                .as_bytes(),
        );
        assert_eq!(saved.history.iter().map(|e| e.hour).min(), Some(first_date));
        assert_eq!(saved.history.iter().map(|e| e.hour).max(), Some(last_date));
        println!(
            "LAND_SAVE divisions={} history={} first={} last={}",
            saved.divisions.len(),
            saved.history.len(),
            first_date,
            last_date
        );
    }
    #[test]
    fn retention_lower_bound_is_inclusive_and_old_results_are_a_limitation() {
        let event = |hour| History {
            hour,
            province: 9,
            attacker: b"A".to_vec(),
            defender: b"B".to_vec(),
            kind: 1,
        };
        let mut report = Report::default();
        assert_eq!(
            compare_certified_history(
                vec![event(831), event(832), event(1000)],
                &[(830, 1000)],
                &[event(832), event(1000)],
                1000,
                &mut report
            )
            .unwrap(),
            2
        );
        assert!(report.differences.is_empty());
        assert_eq!(report.limitations.len(), 1);
        for (captured, saved) in [(vec![], vec![event(832)]), (vec![event(832)], vec![])] {
            let mut report = Report::default();
            compare_certified_history(captured, &[(830, 1000)], &saved, 1000, &mut report).unwrap();
            assert_eq!(report.differences.len(), 1);
        }
    }
    #[test]
    fn coverage_count_includes_only_spans_intersecting_retention() {
        assert_eq!(retained_span_count(&[(800, 831)], 1000), 0);
        assert_eq!(retained_span_count(&[(831, 832)], 1000), 1);
        assert_eq!(retained_span_count(&[(832, 833)], 1000), 1);
        assert_eq!(retained_span_count(&[(830, 1000)], 1000), 1);
        assert_eq!(retained_span_count(&[(1000, 1001)], 1000), 0);
        assert_eq!(
            retained_span_count(&[(800, 831), (831, 832), (832, 1000)], 1000),
            2
        );
    }
    #[test]
    fn division_projection_keeps_positive_order_and_reports_manpower_difference() {
        let (mut frame, _, _, _) = super::super::fixtures::fixture();
        let id = 9 << 32 | 4713;
        let row = replay_data::rows::DivisionLandInputsRow {
            division: id,
            manpower: vec![
                replay_data::rows::DivisionManpowerContributor {
                    country: 101,
                    amount: 2,
                },
                replay_data::rows::DivisionManpowerContributor {
                    country: 102,
                    amount: -7,
                },
                replay_data::rows::DivisionManpowerContributor {
                    country: 101,
                    amount: 9,
                },
            ],
            ..Default::default()
        };
        *frame.tables[TableId::DivisionLandInputs.index()] =
            TableView::Complete(TableRows::DivisionLandInputs(vec![row.clone()]));
        let saved = Saved {
            divisions: BTreeMap::from([(
                id,
                Division {
                    manpower: vec![(b"GER".to_vec(), 2), (b"GER".to_vec(), 9)],
                },
            )]),
            ..Default::default()
        };
        let mut report = Report::default();
        compare_divisions(&frame, &saved, &mut report).unwrap();
        assert!(report.differences.is_empty());
        assert!(!report.limitations.is_empty());
        let mut changed = row;
        changed.manpower.reverse();
        *frame.tables[TableId::DivisionLandInputs.index()] =
            TableView::Complete(TableRows::DivisionLandInputs(vec![changed]));
        let mut report = Report::default();
        compare_divisions(&frame, &saved, &mut report).unwrap();
        assert_eq!(report.differences.len(), 1);
        *frame.tables[TableId::DivisionLandInputs.index()] = TableView::Unavailable {
            reason: "native read failed".into(),
        };
        assert!(compare_divisions(&frame, &saved, &mut Report::default()).is_err());
    }
    #[test]
    fn manpower_alias_spellings_are_limits_but_ordered_amounts_still_compare() {
        let (mut frame, _, _, _) = super::super::fixtures::fixture();
        let id = 9 << 32 | 4713;
        let mut row = replay_data::rows::DivisionLandInputsRow {
            division: id,
            manpower: vec![
                replay_data::rows::DivisionManpowerContributor {
                    country: 102,
                    amount: 7,
                },
                replay_data::rows::DivisionManpowerContributor {
                    country: 0,
                    amount: 9,
                },
            ],
            ..Default::default()
        };
        let saved = Saved {
            divisions: BTreeMap::from([(
                id,
                Division {
                    manpower: vec![
                        (b"old-opaque-tag".to_vec(), 7),
                        (b"null-opaque-tag".to_vec(), 9),
                    ],
                },
            )]),
            ..Default::default()
        };
        let put = |frame: &mut DecodedFrame, row| {
            *frame.tables[TableId::DivisionLandInputs.index()] =
                TableView::Complete(TableRows::DivisionLandInputs(vec![row]));
        };
        put(&mut frame, row.clone());
        let mut report = Report::default();
        compare_divisions(&frame, &saved, &mut report).unwrap();
        assert!(report.differences.is_empty());
        assert!(
            report
                .limitations
                .keys()
                .any(|s| s.contains("alias/null contributor"))
        );
        row.manpower.swap(0, 1);
        put(&mut frame, row.clone());
        let mut report = Report::default();
        compare_divisions(&frame, &saved, &mut report).unwrap();
        assert_eq!(report.differences.len(), 1);
        row.manpower[0].country = 101;
        row.manpower.swap(0, 1);
        put(&mut frame, row.clone());
        let mut report = Report::default();
        compare_divisions(&frame, &saved, &mut report).unwrap();
        assert_eq!(
            report.differences.len(),
            1,
            "a known raw tag must compare exactly"
        );
        row.manpower[0].country = 777;
        put(&mut frame, row);
        assert!(compare_divisions(&frame, &saved, &mut Report::default()).is_err());
    }
}

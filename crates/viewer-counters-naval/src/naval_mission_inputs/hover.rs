use replay_data::rows::{ModifierBreakdownNode, NavalMissionLedgerRow};
use super::{BTreeMap, BTreeSet, Fact, HashMap, Hour, ModifierDefinitionRow, TokenError, data, derive, io, keys, naval_missions, tables};
#[cfg(test)]
use super::{CountryIntelRow, NativeInputs};
use content_reader_localisation::{localisation::Argument, localisation::Localisation};
use viewer_foundation::tooltip_text::{self, Tooltip};

pub mod convoy;
pub mod patrol;
mod source;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(in crate) enum Control {
    Button,
    Status,
    Pride,
}

pub(in crate) type Hovers = BTreeMap<(u32, u32, Control), Prepared>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Value {
    Text(String),
    Stored(Vec<u8>, &'static str),
    Int(i32),
    Float(u32),
    Fixed(i64),
    Percent(i64),
    Key(&'static str),
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Op {
    Line(bool, usize, &'static str, Vec<(&'static str, Value)>),
    Newline(bool),
    Gap(bool, usize),
    Refused(&'static str, String),
    ModifierTree(ModifierDefinitionRow, Vec<ModifierBreakdownNode>),
    Raw(bool, &'static str, Value),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(in crate) struct Prepared(Vec<Op>);

impl Prepared {
    pub fn line(
        &mut self,
        body: bool,
        gap: usize,
        key: &'static str,
        args: Vec<(&'static str, Value)>,
    ) {
        self.0.push(Op::Line(body, gap, key, args));
    }
    pub fn refuse(&mut self, key: &'static str, reason: String) {
        self.0.push(Op::Refused(key, reason));
    }
    pub fn newline(&mut self, body: bool) {
        self.0.push(Op::Newline(body));
    }
    pub fn raw(&mut self, body: bool, name: &'static str, value: Value) {
        self.0.push(Op::Raw(body, name, value));
    }
    fn gap(&mut self, body: bool, count: usize) {
        self.0.push(Op::Gap(body, count));
    }
    pub(in crate) fn render(
        &self,
        loc: &Localisation,
        recorded: &str,
        labels: Option<&viewer_runtime::recorded_labels::RecordedLabels>,
    ) -> Tooltip {
        let mut out = Tooltip::default();
        for op in &self.0 {
            match op {
                Op::Line(body, gap, key, args) => {
                    let rendered = args
                        .iter()
                        .map(|(_, value)| value.text(loc, recorded))
                        .collect::<Fact<Vec<_>>>();
                    let text = rendered.and_then(|text| {
                        let arguments: Vec<_> = args
                            .iter()
                            .zip(&text)
                            .map(|((name, value), text)| {
                                (
                                    *name,
                                    match value {
                                        Value::Int(value) => Argument::Int32(*value),
                                        Value::Float(value) => Argument::Float32(*value),
                                        Value::Fixed(value) => Argument::Fixed5(*value),
                                        _ => Argument::Text(text),
                                    },
                                )
                            })
                            .collect();
                        tooltip_text::localise(loc, key, &arguments)
                    });
                    out.line(*body, *gap, key, text);
                }
                Op::Newline(body) => {
                    if *body {
                        out.body.push('\n')
                    } else {
                        out.title.push('\n')
                    }
                }
                Op::Gap(body, count) => tooltip_text::ensure_newlines(
                    if *body { &mut out.body } else { &mut out.title },
                    *count,
                ),
                Op::Refused(key, reason) => out.refuse(key, reason.clone()),
                Op::ModifierTree(definition, nodes) => {
                    match source::render(definition, nodes, loc, recorded, labels) {
                        Ok(text) => out.title.push_str(&text),
                        Err(reason) => out.refuse("CONVOY_ESCORT_EFFICIENCY sources", reason),
                    }
                }
                Op::Raw(body, name, value) => match value.text(loc, recorded) {
                    Ok(text) => {
                        if *body {
                            out.body.push_str(&text)
                        } else {
                            out.title.push_str(&text)
                        }
                    }
                    Err(reason) => out.refuse(name, reason),
                },
            }
        }
        out
    }
}

impl Value {
    fn text(&self, loc: &Localisation, recorded: &str) -> Fact<String> {
        match self {
            Self::Text(text) => Ok(text.clone()),
            Self::Stored(bytes, what) => {
                dashboard_text::scripted_text::stored_text(loc, recorded, bytes, what)
            }
            Self::Percent(raw) => Ok(percent(*raw)),
            Self::Key(key) => tooltip_text::localise(loc, key, &[]),
            Self::Float(_) | Self::Int(_) | Self::Fixed(_) => Ok(String::new()),
        }
    }
}

fn percent(raw: i64) -> String {
    let text = content_reader_parser::fixed5::fixed5_percent(raw.clamp(0, 100_000), 0, true);
    if raw < 75_000 {
        format!("\u{11}R{text}\u{11}!")
    } else if raw < 100_000 {
        format!("\u{11}Y{text}\u{11}!")
    } else {
        text
    }
}

fn mission(kind: u32) -> &'static str {
    match kind {
        0 => "NAVAL_MISSION_NAME_HOLD",
        1 => "NAVAL_MISSION_NAME_PATROL",
        2 => "NAVAL_MISSION_NAME_STRIKE_FORCE",
        3 => "NAVAL_MISSION_NAME_CONVOY_RAIDING",
        4 => "NAVAL_MISSION_NAME_CONVOY_ESCORT",
        5 => "NAVAL_MISSION_NAME_MINES_PLANTING",
        6 => "NAVAL_MISSION_NAME_MINES_SWEEPING",
        7 => "NAVAL_MISSION_NAME_TRAINING",
        8 => "NAVAL_MISSION_NAME_RESERVE_FLEET",
        9 => "NAVAL_MISSION_NAME_NAVAL_INVASION_SUPPORT",
        _ => "",
    }
}

fn name(hour: &Hour<'_>, handle: u64) -> Fact<Value> {
    let row = hour
        .native
        .task_force(handle)
        .ok_or_else(|| format!("task force {handle:#x} has no cached name row"))?;
    let bytes = row
        .name
        .require("TaskForceCounterInputs.name")?
        .as_ref()
        .ok_or("task force cached name read was refused")?;
    Ok(Value::Stored(bytes.clone(), "task force cached name"))
}

fn assigned(out: &mut Prepared, kind: u32, total: u32, gap: usize) {
    out.line(
        false,
        gap,
        "CURRENT_ASSIGNED_TASK_FORCE_FOR_MISSION",
        vec![
            ("MISSION", Value::Key(mission(kind))),
            ("NUM", Value::Int(total as i32)),
        ],
    );
}
fn active_summary(out: &mut Prepared, active: u32, total: u32, gap: usize) {
    let key = if active == total {
        "CURRENT_ALL_ACTIVE_TASK_FORCE_FOR_MISSION"
    } else if active == 0 {
        "CURRENT_NO_ACTIVE_TASK_FORCE_FOR_MISSION"
    } else {
        "CURRENT_ACTIVE_TASK_FORCE_FOR_MISSION"
    };
    out.line(false, gap, key, vec![("NUM", Value::Int(active as i32))]);
}
fn aggression(hour: &Hour<'_>, ledger: &NavalMissionLedgerRow, kind: u32, out: &mut Prepared) {
    for &handle in &ledger.task_forces {
        let result = (|| {
            if hour.force(handle)?.mission != kind {
                return Ok(None);
            }
            let counter = hour
                .native
                .task_force(handle)
                .ok_or("task force engagement rule is unavailable")?;
            if counter.engagement_rule != 0 {
                return Ok(None);
            }
            name(hour, handle).map(Some)
        })();
        match result {
            Ok(Some(value)) => {
                out.line(
                    false,
                    0,
                    "AGRESSIVE_MISSION_WITH_NO_AGGRESSIVENESS",
                    vec![("NAME", value)],
                );
                out.gap(false, 1);
            }
            Ok(None) => {}
            Err(reason) => out.refuse("AGRESSIVE_MISSION_WITH_NO_AGGRESSIVENESS", reason),
        }
    }
}

pub(crate) fn prepare(
    hour: &mut Hour<'_>,
    ledger: &NavalMissionLedgerRow,
    entries: &[naval_missions::MissionEntry],
    out: &mut Hovers,
) {
    for entry in entries {
        let mut button = Prepared::default();
        button.line(
            false,
            0,
            "HEADER",
            vec![("KEY", Value::Key(mission(entry.kind)))],
        );
        if entry.kind == 1 {
            patrol::prepare(hour, ledger, &mut button);
        }
        button.line(true, 0, "RMB_TO_SELECT_ALL", vec![]);
        out.insert((ledger.region, entry.kind, Control::Button), button);
        let mut status = Prepared::default();
        if let Err(reason) = detail(hour, ledger, entry, &mut status) {
            status.refuse("naval mission status", reason);
        }
        out.insert((ledger.region, entry.kind, Control::Status), status);
    }
    let mut pride = Prepared::default();
    if let Err(reason) = pride_hover(hour, &mut pride) {
        pride.refuse("SPECIFIC_NAVY_CONTAINS_PRIDE_OF_FLEET", reason);
    }
    for entry in entries {
        out.insert((ledger.region, entry.kind, Control::Pride), pride.clone());
    }
}

fn pride_hover(hour: &Hour<'_>, out: &mut Prepared) -> Fact<()> {
    let owner = hour
        .ids
        .get(&hour.ours)
        .ok_or("viewing navy country is unavailable")?;
    let country = data::<tables::CountryCounterInputs>(hour.frame)?
        .iter()
        .find(|row| row.country == *owner)
        .ok_or("viewing navy pride reference is unavailable")?;
    if country.pride_of_the_fleet == 0 {
        return Ok(());
    }
    let Some(ship) = data::<tables::Ships>(hour.frame)?
        .iter()
        .find(|row| row.id == country.pride_of_the_fleet)
    else {
        return Ok(());
    };
    let bytes = country
        .pride_name
        .require("CountryCounterInputs.pride_name")?
        .as_ref()
        .ok_or("pride ship cached name read was refused")?;
    out.line(
        false,
        0,
        "SPECIFIC_NAVY_CONTAINS_PRIDE_OF_FLEET",
        vec![
            ("NAVY_NAME", name(hour, ship.task_force)?),
            (
                "NAME",
                Value::Stored(bytes.clone(), "pride ship cached name"),
            ),
        ],
    );
    Ok(())
}

fn detail(
    hour: &mut Hour<'_>,
    ledger: &NavalMissionLedgerRow,
    entry: &naval_missions::MissionEntry,
    out: &mut Prepared,
) -> Fact<()> {
    let kind = entry.kind;
    match kind {
        1 => {
            let active = entry
                .active
                .map_err(str::to_owned)
                .and_then(|value| value.ok_or("patrol active count is unavailable".into()));
            if active.as_ref().is_ok_and(|&value| value < 1) {
                out.line(false, 0, "REGION_WITH_NO_PATROL", vec![]);
                out.newline(false);
            }
            out.line(
                false,
                2,
                "CURRENT_ASSIGNED_TASK_FORCE_FOR_PATROL",
                vec![("NUM", Value::Int(entry.total as i32))],
            );
            match active {
                Ok(active) => active_summary(out, active, entry.total, 1),
                Err(reason) => out.refuse("patrol active status", reason),
            }
        }
        2 | 9 => {
            let active = entry
                .active
                .map_err(str::to_owned)
                .and_then(|value| value.ok_or("naval mission active count is unavailable".into()));
            if active.as_ref().is_ok_and(|&value| value < entry.total) {
                out.line(
                    false,
                    0,
                    if kind == 2 {
                        "CURRENT_ACTIVE_TASK_FORCE_FOR_DOMINANCE_WARNING"
                    } else {
                        "CURRENT_ACTIVE_TASK_FORCE_FOR_NAVAL_INVASION_SUPPORT_WARNING"
                    },
                    vec![],
                );
                out.gap(false, 1);
            }
            aggression(hour, ledger, kind, out);
            assigned(out, kind, entry.total, 2);
            let active = match active {
                Ok(value) => value,
                Err(reason) => {
                    out.refuse("naval mission active status", reason);
                    return Ok(());
                }
            };
            if kind == 2 && active < entry.total {
                out.line(
                    false,
                    1,
                    if active == 0 {
                        "CURRENT_NO_ACTIVE_TASK_FORCE_FOR_DOMINANCE"
                    } else {
                        "CURRENT_ACTIVE_TASK_FORCE_FOR_DOMINANCE"
                    },
                    vec![("NUM", Value::Int(active as i32))],
                );
            } else if kind == 9 && active < entry.total {
                out.line(
                    false,
                    1,
                    "CURRENT_NO_ACTIVE_TASK_FORCE_FOR_NAVAL_INVASION_SUPPORT",
                    vec![],
                );
                out.gap(false, 1);
            } else {
                active_summary(out, active, entry.total, 1);
                if kind == 9 {
                    out.gap(false, 1);
                }
            }
        }
        3 => {
            aggression(hour, ledger, kind, out);
            let mut values = Vec::new();
            for &handle in &ledger.task_forces {
                if hour.force(handle)?.mission == 3 {
                    values.push(hour.raiding(handle)?);
                }
            }
            let efficiency = derive::raiding_ledger(values);
            if efficiency < 100_000 {
                out.line(false, 0, "TOO_MANY_REGIONS_FOR_CONVOY_RAID_WARNING", vec![]);
            }
            assigned(out, kind, entry.total, 2);
            active_summary(out, entry.total, entry.total, 1);
            out.line(
                false,
                2,
                "CONVOY_RAIDING_EFFICIENCY_IN_REGION",
                vec![("VALUE", Value::Percent(efficiency))],
            );
            out.gap(false, 2);
            for &handle in &ledger.task_forces {
                if hour.force(handle)?.mission != 3 {
                    continue;
                }
                let force = hour.force(handle)?;
                if force.fleet == 0 {
                    continue;
                }
                let input = hour.input(handle)?;
                if input.range_end.wrapping_sub(input.range_begin) == 0 {
                    continue;
                }
                let count = hour.siblings(handle, 3)?.len() as i32;
                let ratio = derive::raid_region_ratio(
                    hour.combined(handle, keys::NAVAL_COORDINATION)?,
                    &hour.content.efficiencies,
                );
                let covered = (i64::from(count).wrapping_mul(100_000).wrapping_mul(ratio)
                    / 100_000)
                    .wrapping_mul(input.raiding_factor.wrapping_add(100_000))
                    / 100_000;
                out.line(
                    false,
                    0,
                    "CONVOY_RAIDING_TASK_FORCE_EFFICIENCY_DESC",
                    vec![
                        ("TASK_FORCE", name(hour, handle)?),
                        ("NUM", Value::Int(count)),
                        ("EFFICIENCY", Value::Percent(hour.raiding(handle)?)),
                        ("REGIONS", Value::Fixed(covered)),
                    ],
                );
                out.newline(false);
            }
        }
        4 => escort(hour, ledger, entry, out)?,
        _ => {
            assigned(out, kind, entry.total, 0);
            active_summary(out, entry.total, entry.total, 0);
        }
    }
    Ok(())
}

fn escort(
    hour: &mut Hour<'_>,
    ledger: &NavalMissionLedgerRow,
    entry: &naval_missions::MissionEntry,
    out: &mut Prepared,
) -> Fact<()> {
    let mut fleets = BTreeSet::new();
    let mut selected = Vec::new();
    let mut best = -100_000_i64;
    let mut convoys = -1_i32;
    for &handle in &ledger.task_forces {
        let force = hour.force(handle)?;
        if force.mission != 4 || !fleets.insert(force.fleet) {
            continue;
        }
        let efficiency = hour.escort(handle)?;
        best = best.max(efficiency);
        convoys = convoys.max(hour.input(handle)?.convoys);
        selected.push((handle, efficiency));
    }
    let efficiency = if best == -100_000 || convoys == -1 {
        -100_000
    } else {
        best
    };
    let presence = derive::presence(ledger.ring, ledger.positive, ledger.total);
    if (efficiency as u64) < 100_000 {
        out.line(false, 0, "TOO_MANY_CONVOYS_FOR_CONVOY_ESCORT", vec![]);
        out.newline(false);
    }
    if presence >= 0
        && presence
            < hour
                .content
                .efficiencies
                .convoy_escort_presence_warning_threshold
    {
        out.line(false, 0, "NOT_ENOUGH_TASK_FORCES_FOR_CONVOY_ESCORT", vec![]);
        out.newline(false);
    }
    out.line(
        false,
        2,
        "CONVOY_ESCORT_TASK_FORCES_IN_REGION_COUNT",
        vec![("COUNT", Value::Int(entry.total as i32))],
    );
    out.line(
        false,
        1,
        "CONVOY_ESCORT_MISSION_EFFECTIVENESS",
        vec![(
            "VALUE",
            Value::Percent(presence.wrapping_mul(efficiency) / 100_000),
        )],
    );
    out.line(
        false,
        1,
        "CONVOY_ESCORT_EFFICIENCY_IN_REGION",
        vec![("VALUE", Value::Percent(efficiency))],
    );
    out.line(
        false,
        1,
        "CONVOY_ESCORT_PRESENCE_IN_REGION",
        vec![("VALUE", Value::Percent(presence))],
    );
    out.gap(false, 2);
    if efficiency != -100_000 {
        if convoys < 1 {
            out.line(
                false,
                0,
                "CONVOY_ESCORT_EFFICIENCY_NO_CONVOY_IN_FLEETS_AOO",
                vec![],
            );
        } else {
            out.line(
                false,
                0,
                "CONVOY_ESCORT_EFFICIENCY_CONVOYS_IN_AREA_OF_OPERATION",
                vec![("VALUE", Value::Int(convoys))],
            );
        }
        out.gap(false, 2);
        for (handle, value) in selected {
            if let Err(reason) = escort_force(hour, handle, value, out) {
                out.refuse("CONVOY_ESCORT_EFFICIENCY_FLEET_DESC", reason);
            }
        }
        out.gap(false, 2);
        if let Err(reason) = source::prepare_ledger(hour, ledger, out) {
            out.refuse("CONVOY_ESCORT_EFFICIENCY sources", reason);
        }
    }
    out.gap(false, 2);
    let hours = format!(
        "\u{11}{}{value}\u{11}!",
        if ledger.positive == ledger.total {
            'Y'
        } else {
            'R'
        },
        value = ledger.positive
    );
    out.line(
        false,
        0,
        "CONVOY_ESCORT_PRESENCE_IN_REGION_EXPANDED",
        vec![
            ("HOURS_WITH", Value::Text(hours)),
            ("HOURS_TOTAL", Value::Int(ledger.total)),
            ("PERCENT", Value::Percent(presence)),
        ],
    );
    for key in [
        "TOO_MANY_CONVOYS_FOR_CONVOY_ESCORT_DETAILED",
        "TOO_MANY_CONVOYS_FOR_CONVOY_ESCORT_REMEDY",
        "NOT_ENOUGH_TASK_FORCES_FOR_CONVOY_ESCORT_DETAILED",
        "NOT_ENOUGH_TASK_FORCES_FOR_CONVOY_ESCORT_REMEDY",
    ] {
        out.line(true, 2, key, vec![]);
    }
    Ok(())
}

fn escort_force(hour: &mut Hour<'_>, handle: u64, efficiency: i64, out: &mut Prepared) -> Fact<()> {
    let input = hour.input(handle)?;
    let force = hour.force(handle)?;
    let mut ships = 0_u32;
    let mut capacity = 0_i64;
    let siblings = hour.siblings(handle, 4)?;
    let count = siblings.len() as i32;
    for sibling in siblings {
        ships = ships.wrapping_add(hour.ships(sibling.id)?.len() as u32);
        capacity = capacity.wrapping_add(derive::escort_capacity(
            hour.combined(sibling.id, keys::NAVAL_COORDINATION)?,
            &hour.content.efficiencies,
        ));
    }
    let forces = i64::from(count).wrapping_mul(100_000);
    let average = if forces == 0 {
        0
    } else if (capacity as u64).wrapping_add(0x7fff_ffff_ffff_fffe) < 0xffff_ffff_ffff_fffd {
        capacity.wrapping_mul(100_000) / forces
    } else {
        0xffff_ffff
    };
    let factor = hour
        .country_value(input.mission_owner, keys::CONVOY_ESCORT_EFFICIENCY)?
        .wrapping_add(100_000);
    let convoy = i64::from(ships as i32)
        .wrapping_mul(average)
        .wrapping_mul(factor)
        / 100_000;
    let region = (derive::escort_region_ratio(
        hour.combined(handle, keys::NAVAL_COORDINATION)?,
        &hour.content.efficiencies,
    )
    .wrapping_mul(forces)
        / 100_000)
        .wrapping_mul(factor)
        / 100_000;
    let fleet = hour
        .fleets
        .as_ref()
        .map_err(Clone::clone)?
        .get(&force.fleet)
        .ok_or("escort fleet cached name is unavailable")?;
    let bytes = fleet
        .name
        .require("Fleets.name")?
        .as_ref()
        .ok_or("escort fleet cached name read was refused")?;
    out.line(
        false,
        0,
        "CONVOY_ESCORT_EFFICIENCY_FLEET_DESC",
        vec![
            ("FLEET", Value::Stored(bytes.clone(), "fleet cached name")),
            ("NUM", Value::Int(ships as i32)),
            ("COVERED_CONVOY", Value::Fixed(convoy)),
            ("NUM_TASKFORCES", Value::Int(count)),
            ("COVERED_REGION", Value::Fixed(region)),
            ("EFFICIENCY", Value::Percent(efficiency)),
        ],
    );
    out.gap(false, 1);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{Localisation, Prepared, percent};
    #[test]
    fn efficiency_colour_uses_unclamped_value() {
        assert_eq!(percent(-1), "\u{11}R0%\u{11}!");
        assert_eq!(percent(74_999), "\u{11}R74%\u{11}!");
        assert_eq!(percent(75_000), "\u{11}Y75%\u{11}!");
        assert_eq!(percent(100_001), "100%");
    }
    #[test]
    fn append_and_ensure_newlines_are_distinct() {
        let mut text = Prepared::default();
        text.line(false, 0, "A", vec![]);
        text.newline(false);
        text.gap(false, 2);
        text.line(false, 0, "B", vec![]);
        assert_eq!(
            text.render(&Localisation::default(), "english", None).title,
            "A\n\nB"
        );
    }
}

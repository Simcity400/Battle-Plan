//! Standing orders alongside existing detailed task-force hovers.
use super::*;
fn policy_text(
    row: &replay_data::rows::NavalRepairTaskForceRow,
    loc: &Localisation,
) -> Result<String, String> {
    let key = format!("NAVY_REPAIR_POLICY_{}", row.repair_mode);
    loc.get(&key)
        .ok_or_else(|| format!("Naval repair policy label {key} is unavailable"))?;
    localise(loc, &key, &[])
}
fn settings_text(row: &replay_data::rows::NavalRepairTaskForceRow) -> String {
    let flag = |value| if value != 0 { "On" } else { "Off" };
    let target = if row.repair_target == 0 {
        String::new()
    } else {
        format!("Repair target: province {}\n", row.repair_target)
    };
    format!(
        "Split for repair: {}\nAutomatic reinforcement: {}\n{target}",
        flag(row.repair_split),
        flag(row.auto_reinforcement)
    )
}
pub(super) fn append(context: &Context<'_>, members: &[u64], out: &mut Tooltip) {
    let settings = match context.frame.complete::<tables::NavalRepairTaskForces>() {
        Ok(rows) => rows,
        Err(why) => {
            refuse(out, "naval repair settings", why.to_string());
            return;
        }
    };
    let tasks = context.frame.complete::<tables::TaskForces>();
    let fleets = context.frame.complete::<tables::NavalRepairFleets>();
    for id in members {
        let setting = settings
            .binary_search_by_key(id, |row| row.task_force)
            .map(|index| &settings[index])
            .map_err(|_| "Naval repair settings are absent for this task force".to_owned());
        match setting {
            Ok(row) => {
                super::append(
                    out,
                    context,
                    true,
                    "naval repair priority",
                    policy_text(row, context.loc)
                        .map(|priority| format!("Repair priority: {priority}\n")),
                );
                out.body.push_str(&settings_text(row));
            }
            Err(why) => refuse(out, "naval repair settings", why),
        }
        if let (Ok(tasks), Ok(fleets)) = (&tasks, &fleets)
            && let Ok(index) = tasks.binary_search_by_key(id, |row| row.id)
            && let Ok(fleet) = fleets.binary_search_by_key(&tasks[index].fleet, |row| row.fleet)
        {
            out.body.push_str(if fleets[fleet].automated_homebase != 0 {
                "Automatic home base: On\n"
            } else {
                "Automatic home base: Off\n"
            });
        }
        match context
            .frame
            .complete::<tables::NavalTaskForceComposition>()
        {
            Err(why) => refuse(out, "naval intended composition", why.to_string()),
            Ok(rows) => {
                for row in rows
                    .iter()
                    .filter(|row| row.task_force == *id && row.kind == 0)
                {
                    let class = context
                        .content
                        .ok_or_else(|| "Naval class content unavailable".to_owned())
                        .and_then(|content| content.sub_units.as_ref().map_err(Clone::clone))
                        .and_then(|units| {
                            units
                                .name(row.sub_unit as u32)
                                .ok_or_else(|| "Naval class definition unavailable".to_owned())
                        })
                        .and_then(|name| name)
                        .and_then(|name| localise(context.loc, name, &[]));
                    super::append(
                        out,
                        context,
                        true,
                        "naval intended composition",
                        class.map(|class| {
                            format!(
                                "Intended {class}: {} (role preference {})\n",
                                row.amount, row.role
                            )
                        }),
                    );
                }
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn naval_management_hover_uses_recorded_policy_labels_and_independent_controls() {
        let loc = Localisation::default();
        let row = replay_data::rows::NavalRepairTaskForceRow {
            repair_mode: 72,
            repair_split: 1,
            auto_reinforcement: 0,
            repair_target: 9,
            ..Default::default()
        };
        assert!(policy_text(&row, &loc).is_err());
        let flags = settings_text(&row);
        assert!(flags.contains("Split for repair: On"));
        assert!(flags.contains("Automatic reinforcement: Off"));
        assert!(flags.contains("province 9"));
    }
}

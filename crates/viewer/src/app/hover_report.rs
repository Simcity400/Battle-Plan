
use viewer_combat::picker::Control;
use viewer_runtime::unit_handle::Unit;

pub(crate) fn describe(control: Option<&Control>, unit: Option<Unit>) -> Option<String> {
    if let Some(control) = control {
        return Some(match control {
            Control::Slot(slot) => format!("button:slot:{slot}"),
            Control::All => "button:all".to_owned(),
            Control::Toggle(name) => format!("button:toggle:{name}"),
            Control::Entry(token) => format!("button:entry:{token}"),
            Control::SlotEntry(slot) => format!("button:slot-entry:{slot}"),
            Control::Default => "button:default".to_owned(),
            Control::FindView => "button:find-view".to_owned(),
            Control::Minimap(control) => format!("button:minimap:{}", control.name()),
            Control::Find(control) => format!("button:find:{}", control.name()),
            Control::Window => "button:window".to_owned(),
        });
    }
    unit.map(|unit| match unit {
        Unit::Division(id) => format!("division:{id}"),
        Unit::RailwayGun(id) => format!("railway_gun:{id}"),
        Unit::TaskForce(id) => format!("task_force:{id}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_picker_takes_the_pointer_then_a_counter() {
        assert_eq!(describe(Some(&Control::Slot(2)), Some(Unit::Division(7))).as_deref(), Some("button:slot:2"));
        assert_eq!(describe(None, Some(Unit::Division(7))).as_deref(), Some("division:7"));
        assert_eq!(describe(None, None), None);
    }
}

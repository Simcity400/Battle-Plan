use replay_data::rows::{Recorded, tables};
use replay_frame::frame::DecodedFrame;
use std::io;
use viewer_runtime::unit_handle::Unit;

const TANK: u64 = 0x4;
const VEHICLE: u64 = 0x18;
pub const RECENT: f64 = 0.4;

pub const FLEET: &str = "select_fleet";

pub const COMBAT_VOICE_WITHHELD: &str = "selection voice-over: the in-combat voice choice is withheld, as the side value the game compares is not resolved";

pub fn division_effects(mask: u64, separate_counter: bool) -> impl Iterator<Item = &'static str> {
    let unit = if mask & TANK != 0 {
        "select_tank"
    } else if mask & VEHICLE != 0 {
        "select_vehicle"
    } else {
        "select_army"
    };
    separate_counter.then_some("select_general").into_iter().chain([unit])
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Queued {
    pub effects: Vec<&'static str>,
    pub voice: Option<Result<String, &'static str>>,
    pub refused: Vec<String>,
}

pub fn queued(
    frame: &DecodedFrame,
    added: &[Unit],
    ours: u32,
    mask: &dyn Fn(&str) -> Option<u64>,
) -> io::Result<Queued> {
    let mut queued = Queued::default();
    if added.iter().any(|unit| matches!(unit, Unit::RailwayGun(_))) {
        queued.effects.push("select_railway_gun");
    }
    if added.iter().any(|unit| matches!(unit, Unit::TaskForce(_))) {
        queued.effects.push(FLEET);
    }
    let divisions: Vec<u64> = added
        .iter()
        .filter_map(|unit| match unit {
            Unit::Division(handle) => Some(*handle),
            _ => None,
        })
        .collect();
    let Some(&first) = divisions.first() else {
        return Ok(queued);
    };
    let rows = frame.complete::<tables::Divisions>()?;
    let icons = frame.complete::<tables::DivisionTemplateIcons>()?;
    let inputs = frame.complete::<tables::ArmyIconInputs>()?;
    for handle in divisions {
        let Some(division) = rows.iter().find(|row| row.id == handle) else {
            queued.refused.push(format!("selection sound: division {handle} has no Divisions row"));
            continue;
        };
        let Some(name) = icons
            .iter()
            .find(|row| row.template == division.template)
            .map(|row| String::from_utf8_lossy(&row.priority_unit_name).into_owned())
        else {
            queued.refused.push(format!("selection sound: division {handle}'s template has no DivisionTemplateIcons row"));
            continue;
        };
        let Some(mask) = mask(&name) else {
            queued.refused.push(format!("selection sound: sub-unit {name} has no type mask"));
            continue;
        };
        let Some(separate) = inputs.iter().find(|row| row.division == handle).map(|row| row.separate_counter != 0) else {
            queued.refused.push(format!("selection sound: division {handle} has no ArmyIconInputs row"));
            continue;
        };
        for effect in division_effects(mask, separate) {
            if !queued.effects.contains(&effect) {
                queued.effects.push(effect);
            }
        }
    }
    queued.voice = Some(voice(frame, first, ours));
    Ok(queued)
}

fn voice(frame: &DecodedFrame, division: u64, ours: u32) -> Result<String, &'static str> {
    let combats = frame.complete::<tables::LandCombats>().map_err(|_| "selection voice-over: LandCombats is not recorded at this hour")?;
    if combats.iter().any(|combat| combat.attackers.contains(&division) || combat.defenders.contains(&division)) {
        return Err(COMBAT_VOICE_WITHHELD);
    }
    let countries = frame.complete::<tables::Countries>().map_err(|_| "selection voice-over: Countries is not recorded at this hour")?;
    let country = countries.iter().find(|row| row.canonical_slot == ours).ok_or("selection voice-over: the viewing country has no Countries row")?;
    let boundary = frame
        .complete::<tables::ReservedCountryNaming>()
        .ok()
        .and_then(|rows| match rows.first()?.dynamic_tag_boundary {
            Recorded::Value(Some(boundary)) => Some(boundary),
            _ => None,
        })
        .ok_or("selection voice-over: the country database's dynamic-tag boundary is not recorded")?;
    let tag = if i64::from(ours) >= i64::from(boundary) {
        let appearance = frame
            .complete::<tables::CountryAppearance>()
            .map_err(|_| "selection voice-over: CountryAppearance is not recorded at this hour")?
            .iter()
            .find(|row| row.country == country.id)
            .ok_or("selection voice-over: the viewing country has no CountryAppearance row")?;
        match (&appearance.original_tag_reference, &appearance.original_tag) {
            (Recorded::Value(Some(reference)), Recorded::Value(Some(original))) if *reference > 0 => original.as_slice(),
            (Recorded::Value(Some(_)), _) => country.tag.as_slice(),
            _ => return Err("selection voice-over: the dynamic country's original tag is not recorded"),
        }
    } else {
        country.tag.as_slice()
    };
    let tag = std::str::from_utf8(tag).map_err(|_| "selection voice-over: the speaking country's tag is not UTF-8")?;
    Ok(format!("{tag}_infantry_idle"))
}

#[derive(Default)]
pub struct Recent(Vec<(String, f64)>);

impl Recent {
    pub fn admit<'a>(&mut self, now: f64, queued: impl IntoIterator<Item = &'a str>) -> Vec<&'a str> {
        self.0.retain(|(_, started)| now - started < RECENT);
        let mut played: Vec<&'a str> = Vec::new();
        for effect in queued {
            if played.contains(&effect) || self.0.iter().any(|(recent, _)| recent == effect) {
                continue;
            }
            self.0.push((effect.to_owned(), now));
            played.push(effect);
        }
        played
    }
}

#[derive(Default)]
pub struct VoiceGate(Option<f64>);

impl VoiceGate {
    pub fn admit(&mut self, now: f64, cool_down: f64) -> bool {
        if self.0.is_some_and(|last| now - last < cool_down) {
            return false;
        }
        self.0 = Some(now);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_data::rows::{
        ArmyIconInputRow, CountryAppearanceRow, CountryRow, DivisionRow, DivisionTemplateIconRow,
        LandCombatRow, ReservedCountryNamingRow, TableRows,
    };
    use viewer_foundation::test_support::{test_frame, test_put as put};

    #[test]
    fn the_type_mask_chooses_the_effect_and_a_separate_counter_adds_the_general() {
        let effects = |mask, separate| division_effects(mask, separate).collect::<Vec<_>>();
        assert_eq!(effects(0x4, false), ["select_tank"]);
        assert_eq!(effects(0x8, false), ["select_vehicle"]);
        assert_eq!(effects(0x10, false), ["select_vehicle"]);
        assert_eq!(effects(0x1, false), ["select_army"]);
        assert_eq!(effects(0x0, true), ["select_general", "select_army"]);
    }

    fn frame(combat: bool, reference: i32, boundary: i32) -> DecodedFrame {
        let mut frame = test_frame(42);
        put(&mut frame, TableRows::Divisions(vec![
            DivisionRow { id: 1, template: 10, ..Default::default() },
            DivisionRow { id: 2, template: 20, ..Default::default() },
        ]));
        put(&mut frame, TableRows::DivisionTemplateIcons(vec![
            DivisionTemplateIconRow { template: 10, priority_unit_name: b"infantry".to_vec(), ..Default::default() },
            DivisionTemplateIconRow { template: 20, priority_unit_name: b"light_armor".to_vec(), ..Default::default() },
        ]));
        put(&mut frame, TableRows::ArmyIconInputs(vec![
            ArmyIconInputRow { division: 1, separate_counter: 0, ..Default::default() },
            ArmyIconInputRow { division: 2, separate_counter: 1, ..Default::default() },
        ]));
        put(&mut frame, TableRows::LandCombats(vec![LandCombatRow {
            attackers: if combat { vec![1] } else { Vec::new() },
            ..Default::default()
        }]));
        put(&mut frame, TableRows::Countries(vec![CountryRow { id: 5, canonical_slot: 3, tag: b"GER".to_vec(), ..Default::default() }]));
        put(&mut frame, TableRows::CountryAppearance(vec![CountryAppearanceRow {
            country: 5,
            original_tag_reference: Recorded::Value(Some(reference)),
            original_tag: Recorded::Value(Some(b"POL".to_vec())),
            ..Default::default()
        }]));
        put(&mut frame, TableRows::ReservedCountryNaming(vec![ReservedCountryNamingRow {
            dynamic_tag_boundary: Recorded::Value(Some(boundary)),
            ..Default::default()
        }]));
        frame
    }

    fn masks(name: &str) -> Option<u64> {
        match name {
            "infantry" => Some(0x1),
            "light_armor" => Some(0x4),
            _ => None,
        }
    }

    #[test]
    fn each_added_unit_queues_its_own_callbacks_effects_once() {
        let added = [Unit::Division(1), Unit::Division(2), Unit::RailwayGun(7), Unit::TaskForce(9), Unit::TaskForce(8)];
        let queued = queued(&frame(false, 0, 100), &added, 3, &masks).unwrap();
        assert_eq!(queued.effects, ["select_railway_gun", FLEET, "select_army", "select_general", "select_tank"]);
        assert_eq!(queued.voice, Some(Ok("GER_infantry_idle".to_owned())));
        assert!(queued.refused.is_empty());
        let none = super::queued(&frame(false, 0, 100), &[Unit::TaskForce(9)], 3, &masks).unwrap();
        assert_eq!((none.effects, none.voice), (vec![FLEET], None));
    }

    #[test]
    fn a_dynamic_country_speaks_with_its_original_voice_and_combat_withholds_it() {
        let in_combat = queued(&frame(true, 0, 100), &[Unit::Division(1)], 3, &masks).unwrap();
        assert_eq!(in_combat.voice, Some(Err(COMBAT_VOICE_WITHHELD)));
        let voice = |reference, boundary| queued(&frame(false, reference, boundary), &[Unit::Division(1)], 3, &masks).unwrap().voice;
        assert_eq!(voice(4, 100), Some(Ok("GER_infantry_idle".to_owned())), "a static country keeps its own tag");
        assert_eq!(voice(4, 3), Some(Ok("POL_infantry_idle".to_owned())), "a dynamic one takes its original's");
        assert_eq!(voice(0, 3), Some(Ok("GER_infantry_idle".to_owned())), "without an original it keeps its own");
        let unknown = queued(&frame(false, 0, 100), &[Unit::Division(2)], 3, &|_| None).unwrap();
        assert!(unknown.effects.is_empty());
        assert_eq!(unknown.refused, ["selection sound: sub-unit light_armor has no type mask"]);
    }

    #[test]
    fn the_queue_plays_an_effect_once_a_frame_and_skips_it_for_four_tenths_of_a_second() {
        let mut recent = Recent::default();
        assert_eq!(recent.admit(10.0, ["select_army", "select_army", "select_tank"]), ["select_army", "select_tank"]);
        assert!(recent.admit(10.39, ["select_army"]).is_empty());
        assert_eq!(recent.admit(10.41, ["select_army"]), ["select_army"]);
    }

    #[test]
    fn a_voice_waits_out_its_cooldown_from_the_last_start() {
        let mut gate = VoiceGate::default();
        assert!(gate.admit(1.0, 2.8));
        assert!(!gate.admit(3.7, 2.8));
        assert!(gate.admit(3.9, 2.8));
        assert!(!gate.admit(4.0, 2.8));
    }
}

use super::{Content, Element, links};
use crate::binding::{self, Bindings};
use crate::state_view::{paths_named, withhold_named};
use replay_frame::frame::DecodedFrame;
use viewer_foundation::perspective::Vision;

const BARS: [&str; 3] = ["progress", "progress_enemy", "neutral_progress"];

#[derive(Debug, PartialEq, Eq)]
struct Bar {
    disabled: bool,
    friendly: bool,
    enemy: bool,
    neutral: bool,
    percent: i32,
}

fn bar(friendly: i64, enemy: i64, minimum: i64) -> Bar {
    let present = enemy > 0;
    let floored = enemy.max(minimum);
    let disabled = friendly == 0 && !present;
    let enemy_only = friendly == 0 && present;
    let (part, other) = if enemy_only { (floored, minimum) } else { (friendly, floored) };
    let total = part.wrapping_add(other);
    let share = if total == 0 {
        0
    } else if (part as u64).wrapping_add(0x7fff_ffff_ffff_fffe) < 0xffff_ffff_ffff_fffd {
        part.wrapping_mul(100_000) / total
    } else {
        0xffff_ffff
    };
    Bar {
        disabled,
        friendly: !disabled && present && !enemy_only,
        enemy: !disabled && enemy_only,
        neutral: !disabled && !present,
        percent: crate::fixed::round(share.wrapping_mul(100)),
    }
}

fn flag_shown(bar: &Bar, enemy_present: bool, control: u8) -> bool {
    !bar.disabled && (enemy_present || control == 1)
}

pub(super) fn bind(content: &Content, frame: &DecodedFrame, vision: &Vision, root: &Element, out: &mut Bindings, region: u32) {
    let read = (|| {
        let context = content.air_context.as_ref().ok_or("the naval dominance content is unavailable")?;
        let links = links(frame)?;
        let (snapshot, control, minimum) =
            viewer_counter_products::map::province_tooltips::region_dominance(context, frame, vision.ours(), region, &links)?;
        let shown = bar(snapshot.ratio_friendly, snapshot.raw_enemy, minimum);
        let flag = if flag_shown(&shown, snapshot.raw_enemy > 0, control) {
            let leader = snapshot
                .rows
                .iter()
                .find(|row| row.region_slot == 0)
                .ok_or("the region's leading dominance entry is not recorded")?;
            Some(binding::country_flag(&binding::flags(content, frame).map_err(|e| e.to_string())?, leader.country)?)
        } else {
            None
        };
        Ok::<_, String>((shown, flag))
    })();
    let (shown, flag) = match read {
        Ok(read) => read,
        Err(why) => {
            let names = ["progress_disabled", BARS[0], BARS[1], BARS[2], "flag"];
            withhold_named(out, root, &names, &format!("naval region view dominance: {why}"));
            return;
        }
    };
    for (name, visible) in [("progress_disabled", shown.disabled), (BARS[0], shown.friendly), (BARS[1], shown.enemy), (BARS[2], shown.neutral)] {
        for path in paths_named(root, name) {
            let widget = out.values.entry(path).or_default();
            widget.hidden = !visible;
            if BARS.contains(&name) {
                widget.progress = Some(shown.percent);
            }
        }
    }
    for path in paths_named(root, "flag") {
        match &flag {
            Some(flag) => out.bind(&path, binding::FLAG, Ok(flag.clone())),
            None => binding::hide(&mut out.values, &path),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_side_disables_the_bar() {
        let shown = bar(0, 0, 5_000);
        assert!(shown.disabled && !shown.friendly && !shown.enemy && !shown.neutral);
        assert!(!flag_shown(&shown, false, 1));
    }

    #[test]
    fn a_lone_friendly_side_fills_against_the_minimum() {
        let shown = bar(30_000, 0, 10_000);
        assert!(shown.neutral && !shown.friendly && !shown.enemy);
        assert_eq!(shown.percent, 75);
        assert!(flag_shown(&shown, false, 1));
        assert!(!flag_shown(&shown, false, 0));
    }

    #[test]
    fn a_lone_enemy_fills_its_bar_against_the_minimum() {
        let shown = bar(0, 30_000, 10_000);
        assert!(shown.enemy && !shown.friendly && !shown.neutral);
        assert_eq!(shown.percent, 75);
        assert!(flag_shown(&shown, true, 2));
    }

    #[test]
    fn both_sides_fill_the_friendly_bar_with_its_share() {
        let shown = bar(10_000, 20_000, 40_000);
        assert!(shown.friendly && !shown.enemy && !shown.neutral);
        assert_eq!(shown.percent, 20);
        let half = bar(1, 2, 0);
        assert_eq!(half.percent, 33);
    }
}

use super::{Content, Element, enemies, links, own_raw, percent, text};
use crate::binding::Bindings;
use replay_data::rows::tables;
use replay_frame::frame::DecodedFrame;
use std::collections::{BTreeMap, HashMap};
use viewer_runtime::count_math;
use viewer_foundation::perspective::Vision;

const SHIP_SALT: u32 = 0x8b;

struct Ledgers {
    ships: HashMap<u64, i32>,
    ledgers: BTreeMap<u32, Vec<u64>>,
    ids: BTreeMap<u32, u32>,
}

impl Ledgers {
    fn read(frame: &DecodedFrame, region: u32) -> Result<Self, String> {
        let mut ships = HashMap::new();
        for ship in frame.complete::<tables::Ships>().map_err(|e| e.to_string())? {
            *ships.entry(ship.task_force).or_insert(0_i32) += 1;
        }
        let ledgers = frame
            .complete::<tables::NavalMissionLedgers>()
            .map_err(|e| e.to_string())?
            .iter()
            .filter(|row| row.region == region)
            .map(|row| (row.owner, row.task_forces.clone()))
            .collect();
        let ids = frame
            .complete::<tables::Countries>()
            .map_err(|e| e.to_string())?
            .iter()
            .map(|row| (row.canonical_slot, row.id))
            .collect();
        Ok(Self { ships, ledgers, ids })
    }

    fn count(&self, slot: u32) -> i32 {
        let Some(id) = self.ids.get(&slot) else {
            return 0;
        };
        self.ledgers
            .get(id)
            .into_iter()
            .flatten()
            .filter(|&&force| force != 0)
            .fold(0_i32, |sum, force| sum.wrapping_add(self.ships.get(force).copied().unwrap_or(0)))
    }
}

pub(super) fn range_text([low, high]: [i32; 2]) -> String {
    if low == high { low.to_string() } else { format!("{low}-{high}") }
}

pub(super) fn intel(frame: &DecodedFrame, vision: &Vision, links: &BTreeMap<u32, u32>, target: u32, cap: i64) -> Result<i64, String> {
    if vision.shares_vision_or_fog(target).map_err(|why| why.clause())? {
        return Ok(100_000);
    }
    let lane = frame
        .complete::<tables::CountryIntel>()
        .map_err(|e| e.to_string())?
        .iter()
        .find(|row| row.target_index == target && links.get(&row.owner_country) == Some(&vision.ours()))
        .map_or(0, |row| row.civilian);
    Ok(count_math::ratio(lane, cap))
}

fn enemy_ships(content: &Content, frame: &DecodedFrame, vision: &Vision, ledgers: &Ledgers) -> Result<[i32; 2], String> {
    let [cap, minimum, maximum, range] = content.assets.naval_region_ship_intel.clone()?;
    let links = links(frame)?;
    let mut sum = [0_i32; 2];
    for raw in enemies(frame, vision)? {
        let slot = if raw == 0 { 0 } else { *links.get(&raw).ok_or_else(|| format!("enemy {raw} has no CountryLinks row"))? };
        let count = ledgers.count(slot);
        let ratio = intel(frame, vision, &links, slot, cap)?;
        let [low, high] = count_math::count_range_seeded(count, ratio, SHIP_SALT, frame.hour, [minimum, maximum, range]);
        sum = [sum[0].wrapping_add(low), sum[1].wrapping_add(high)];
    }
    Ok(sum)
}

fn air_shares(friendly: i64, enemy: i64) -> [i64; 2] {
    let total = friendly.wrapping_add(enemy);
    let share = |part: i64| {
        if total <= 0 {
            0
        } else if (part as u64).wrapping_add(0x7fff_ffff_ffff_fffe) < 0xffff_ffff_ffff_fffd {
            part.wrapping_mul(100_000) / total
        } else {
            0xffff_ffff
        }
    };
    [share(friendly), share(enemy)]
}

pub(super) fn bind(content: &Content, frame: &DecodedFrame, vision: &Vision, root: &Element, out: &mut Bindings, region: u32) {
    let ledgers = Ledgers::read(frame, region);
    let own = ledgers.as_ref().map_err(Clone::clone).map(|ledgers| ledgers.count(vision.ours()).to_string());
    text(out, root, "ship_text", own.map_err(|why| format!("naval region view ships: {why}")));
    let enemy = ledgers
        .as_ref()
        .map_err(Clone::clone)
        .and_then(|ledgers| enemy_ships(content, frame, vision, ledgers))
        .map(range_text);
    text(out, root, "enemy_ship_text", enemy.map_err(|why| format!("naval region view enemy ships: {why}")));
    let shares = (|| -> Result<[i64; 2], String> {
        let raw = own_raw(frame, vision)?;
        let score = frame
            .complete::<tables::RaidAirScores>()
            .map_err(|e| e.to_string())?
            .iter()
            .find(|row| row.region == region && row.country == raw)
            .map_or((0, 0), |row| (row.friendly, row.enemy));
        Ok(air_shares(score.0, score.1))
    })();
    for (name, side) in [("plane_text", 0), ("enemy_plane_text", 1)] {
        text(out, root, name, shares.as_ref().map(|shares| percent(shares[side])).map_err(|why| format!("naval region view air shares: {why}")));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registrations_count_their_task_force_ships_each_time() {
        let ledgers = Ledgers {
            ships: HashMap::from([(7, 3), (8, 2)]),
            ledgers: BTreeMap::from([(40, vec![7, 8, 7, 0, 9])]),
            ids: BTreeMap::from([(4, 40), (5, 50)]),
        };
        assert_eq!(ledgers.count(4), 8);
        assert_eq!(ledgers.count(5), 0);
        assert_eq!(ledgers.count(6), 0);
    }

    #[test]
    fn estimates_print_one_number_or_both_bounds() {
        assert_eq!(range_text([4, 4]), "4");
        assert_eq!(range_text([3, 6]), "3-6");
    }

    #[test]
    fn air_shares_split_the_score_and_stay_zero_without_one() {
        assert_eq!(air_shares(30_000, 10_000), [75_000, 25_000]);
        assert_eq!(air_shares(0, 0), [0, 0]);
        assert_eq!(percent(air_shares(1, 2)[0]), "33%");
    }
}

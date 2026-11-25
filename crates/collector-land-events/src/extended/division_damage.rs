use engine_allocation as allocation;
use engine_land_inputs::land_edges as edges;
use replay_data::rows::LandDivisionDamageRow;
use std::collections::HashMap;

pub(crate) const HIT_UNREADABLE: &str = "a division damage hit could not be read";
pub(crate) const EXIT_UNREADABLE: &str = "a division's combat exit could not be read, so its damage could not be closed";
const FRAMING: &str = "division damage record framing invalid";
const OVERFLOW: &str = "division damage total overflow";
const ATTACKERS: &str = "a division was damaged by more attackers than a row id can number";
const ALLOCATION: &str = "collector allocation failed";

type Attackers = Vec<(u64, u64, i64)>;
type Targets = HashMap<u64, Attackers>;

pub(crate) struct Fold {
    open: Option<HashMap<u64, Targets>>,
}

impl Fold {
    pub(crate) const fn new() -> Self {
        Self { open: None }
    }

    pub(crate) fn clear(&mut self) {
        self.open = None;
    }

    pub(crate) fn apply(
        &mut self,
        words: &[u64],
        first: u64,
        to_hour: i32,
        out: &mut Vec<LandDivisionDamageRow>,
    ) -> Result<(), &'static str> {
        out.clear();
        let result = self.fold(words, first, to_hour, out);
        if result.is_err() {
            self.clear();
            out.clear();
        }
        result
    }

    fn fold(
        &mut self,
        words: &[u64],
        first: u64,
        to_hour: i32,
        out: &mut Vec<LandDivisionDamageRow>,
    ) -> Result<(), &'static str> {
        let open = self.open.get_or_insert_with(HashMap::new);
        let mut ended = Vec::new();
        let mut destroyed = Vec::new();
        let mut at = 0;
        while at < words.len() {
            let size = usize::try_from(words[at]).map_err(|_| FRAMING)?;
            let end = at
                .checked_add(size)
                .filter(|&end| size >= 2 && end <= words.len())
                .ok_or(FRAMING)?;
            let record = &words[at + 1..end];
            match record[0] {
                edges::HIT => {
                    let hit = edges::Hit::decode(record).ok_or(FRAMING)?;
                    if hit.withheld != 0 {
                        return Err(HIT_UNREADABLE);
                    }
                    allocation::reserve_map(open, 1).map_err(|_| ALLOCATION)?;
                    let targets = open.entry(hit.combat).or_default();
                    allocation::reserve_map(targets, 1).map_err(|_| ALLOCATION)?;
                    let attackers = targets.entry(hit.target).or_default();
                    match attackers
                        .iter_mut()
                        .find(|(attacker, army, _)| *attacker == hit.attacker && *army == hit.army)
                    {
                        Some((_, _, total)) => *total = total.checked_add(hit.strength).ok_or(OVERFLOW)?,
                        None => {
                            allocation::reserve_push_batch(attackers, 1).map_err(|_| ALLOCATION)?;
                            attackers.push((hit.attacker, hit.army, hit.strength));
                        }
                    }
                }
                edges::EXITED => {
                    let &[_, combat, hour, withheld, _, division, ..] = record else {
                        return Err(FRAMING);
                    };
                    if withheld as u8 & (edges::COMBAT_UNREADABLE | edges::DIVISION_UNREADABLE) != 0 {
                        return Err(EXIT_UNREADABLE);
                    }
                    let Some(targets) = open.get_mut(&combat) else {
                        at = end;
                        continue;
                    };
                    if let Some(attackers) = targets.remove(&division) {
                        let hour = edges::unpack(hour).unwrap_or(to_hour);
                        let id = first.wrapping_add(at as u64) << 16;
                        allocation::reserve_push_batch(out, attackers.len()).map_err(|_| ALLOCATION)?;
                        for (ordinal, (attacker, army, strength)) in attackers.into_iter().enumerate() {
                            let ordinal = u16::try_from(ordinal).map_err(|_| ATTACKERS)?;
                            out.push(LandDivisionDamageRow {
                                id: id | u64::from(ordinal),
                                hour,
                                combat,
                                target: division,
                                attacker,
                                army: (army != 0).then_some(army),
                                strength_raw: strength,
                            });
                        }
                    }
                    if targets.is_empty() {
                        open.remove(&combat);
                    }
                }
                0 | 2 => {
                    allocation::reserve_push_batch(&mut ended, 1).map_err(|_| ALLOCATION)?;
                    ended.push(*record.get(1).ok_or(FRAMING)?);
                }
                1 | 7 => {
                    let division = record.get(if record[0] == 1 { 1 } else { 2 }).ok_or(FRAMING)?;
                    allocation::reserve_push_batch(&mut destroyed, 1).map_err(|_| ALLOCATION)?;
                    destroyed.push(*division);
                }
                _ => {}
            }
            at = end;
        }
        for combat in ended {
            open.remove(&combat);
        }
        if !destroyed.is_empty() {
            open.retain(|_, targets| {
                targets.retain(|target, _| !destroyed.contains(target));
                !targets.is_empty()
            });
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn open_targets(&self) -> usize {
        self.open.as_ref().map_or(0, |open| open.values().map(HashMap::len).sum())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BATTLE: u64 = 0x8000_0002_0000_003e;
    const OTHER: u64 = 0x8000_0002_0000_003f;
    const FIRST: u64 = 0x8000_0001_0000_1269;
    const SECOND: u64 = 0x8000_0001_0000_126a;
    const THIRD: u64 = 0x8000_0001_0000_126b;
    const GROUP: u64 = 0x8000_0004_0000_0010;

    fn hit(combat: u64, attacker: u64, target: u64, army: u64, strength: i64) -> Vec<u64> {
        let mut words = vec![edges::HIT_WORDS as u64];
        edges::Hit { combat, attacker, target, army, strength, withheld: 0 }.copy(&mut |word| words.push(word));
        words
    }

    fn exit(combat: u64, division: u64, hour: i32, withheld: u8) -> Vec<u64> {
        let some = |value| edges::optional(Some(value));
        vec![edges::EXIT_WORDS as u64, edges::EXITED, combat, some(hour), u64::from(withheld), 1, division, some(0), some(80), some(4000), 9_000_000]
    }

    fn folded(fold: &mut Fold, records: &[Vec<u64>], first: u64) -> Result<Vec<LandDivisionDamageRow>, &'static str> {
        let words = records.concat();
        let mut out = Vec::new();
        fold.apply(&words, first, 90, &mut out).map(|()| out)
    }

    fn row(id: u64, combat: u64, target: u64, attacker: u64, army: Option<u64>, strength_raw: i64) -> LandDivisionDamageRow {
        LandDivisionDamageRow { id, hour: 81, combat, target, attacker, army, strength_raw }
    }

    #[test]
    fn hits_fold_across_hours_per_attacker_and_army_and_close_when_the_target_leaves() {
        let mut fold = Fold::new();
        let hour = folded(&mut fold, &[hit(BATTLE, FIRST, THIRD, GROUP, 100), hit(BATTLE, SECOND, THIRD, 0, 30), hit(OTHER, FIRST, SECOND, GROUP, 5)], 0).unwrap();
        assert!(hour.is_empty(), "nothing closes before the target leaves");
        let records = [hit(BATTLE, FIRST, THIRD, GROUP, 50), hit(BATTLE, FIRST, THIRD, 7, 1), exit(BATTLE, THIRD, 81, 0)];
        let start = (2 * edges::HIT_WORDS) as u64;
        let rows = folded(&mut fold, &records, 1000).unwrap();
        let id = (1000 + start) << 16;
        assert_eq!(
            rows,
            [
                row(id, BATTLE, THIRD, FIRST, Some(GROUP), 150),
                row(id | 1, BATTLE, THIRD, SECOND, None, 30),
                row(id | 2, BATTLE, THIRD, FIRST, Some(7), 1),
            ]
        );
        assert_eq!(fold.open_targets(), 1, "the other combat's target stays open");
        assert!(folded(&mut fold, &[exit(BATTLE, THIRD, 82, 0)], 2000).unwrap().is_empty(), "a closed target has nothing left");
    }

    #[test]
    fn a_combat_end_or_destruction_drops_open_targets_only_after_the_hours_exits() {
        let mut fold = Fold::new();
        let result = vec![4, 0, BATTLE, 0];
        let rows = folded(&mut fold, &[hit(BATTLE, FIRST, THIRD, GROUP, 9), hit(BATTLE, FIRST, SECOND, GROUP, 4), result, exit(BATTLE, THIRD, 81, 0)], 0).unwrap();
        assert_eq!(rows.iter().map(|row| (row.target, row.strength_raw)).collect::<Vec<_>>(), [(THIRD, 9)], "the end's own exits still close");
        assert_eq!(fold.open_targets(), 0, "the target that never left is dropped with its combat");
        let wipe = vec![5, 7, 81, SECOND, 0];
        folded(&mut fold, &[hit(OTHER, FIRST, SECOND, GROUP, 4), hit(OTHER, SECOND, FIRST, GROUP, 4), wipe], 0).unwrap();
        assert_eq!(fold.open_targets(), 1, "only the destroyed division's damage is dropped");
    }

    #[test]
    fn an_unreadable_hit_or_exit_refuses_the_hour_and_forgets_everything_open() {
        let mut fold = Fold::new();
        folded(&mut fold, &[hit(BATTLE, FIRST, THIRD, GROUP, 9)], 0).unwrap();
        let mut unreadable = hit(BATTLE, FIRST, 0, GROUP, 9);
        unreadable[7] = u64::from(edges::TARGET_UNREADABLE);
        assert_eq!(folded(&mut fold, &[unreadable], 0), Err(HIT_UNREADABLE));
        assert_eq!(fold.open_targets(), 0);
        folded(&mut fold, &[hit(BATTLE, FIRST, THIRD, GROUP, 9)], 0).unwrap();
        assert_eq!(folded(&mut fold, &[hit(BATTLE, SECOND, THIRD, GROUP, 2), exit(BATTLE, 0, 81, edges::DIVISION_UNREADABLE)], 0), Err(EXIT_UNREADABLE));
        assert_eq!(fold.open_targets(), 0);
        let rows = folded(&mut fold, &[exit(BATTLE, THIRD, 81, 0)], 0).unwrap();
        assert!(rows.is_empty(), "damage from before a refused hour is never published");
        assert_eq!(folded(&mut fold, &[vec![9, edges::HIT]], 0), Err(FRAMING));
    }
}

//! The daily facts written at each finished day: the day index, the
//! dictionaries it names (tag sets, rosters, operands, theatres) and each
//! active region's representatives' pairs and running activity.
use super::*;
use crate::products::air_superiority::daily::{Day, Operands, Pair, Roster, Theatres};
#[cfg(any(test, feature = "test-support"))]
use crate::products::air_superiority::daily::{self, Group};
use dashboard_base::prepared::facts::{Checkpoint, Facts, Limits, MemoryBudget, ReadPages};
#[cfg(any(test, feature = "test-support"))]
use dashboard_base::prepared::facts::{Namespace, Status};

// The column names the writer opens; a reader restores them from its checkpoints.
#[cfg(any(test, feature = "test-support"))]
const FAMILY: &str = "air-superiority";
#[cfg(any(test, feature = "test-support"))]
const DAYS: &str = "air-superiority-days";
#[cfg(any(test, feature = "test-support"))]
const SETS: &str = "air-superiority-sets";
#[cfg(any(test, feature = "test-support"))]
const ROSTERS: &str = "air-superiority-rosters";
#[cfg(any(test, feature = "test-support"))]
const OPERANDS: &str = "air-superiority-operands";
#[cfg(any(test, feature = "test-support"))]
const THEATRES: &str = "air-superiority-theatres";
#[cfg(any(test, feature = "test-support"))]
const PAIRS: &str = "air-superiority-pairs";
#[cfg(any(test, feature = "test-support"))]
const ACTIVITY: &str = "air-superiority-activity";

fn limits() -> Limits {
    Limits {
        write_bytes: 1 << 20,
        cache_bytes: 0,
        page_bytes: 64 << 10,
    }
}

/// Every column's checkpoint, as the last flush committed it.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct Checkpoints {
    pub days: Checkpoint,
    pub sets: Checkpoint,
    pub rosters: Checkpoint,
    pub operands: Checkpoint,
    pub theatres: Checkpoint,
    pub pairs: Checkpoint,
    pub activity: Checkpoint,
}

/// The columns, open for appending or reading.
pub(super) struct Columns {
    pub days: Facts<(), Day>,
    pub sets: Facts<(), Vec<String>>,
    pub rosters: Facts<(), Roster>,
    pub operands: Facts<(), Operands>,
    pub theatres: Facts<(), Theatres>,
    pub pairs: Facts<u32, Vec<Pair>>,
    pub activity: Facts<u32, u128>,
}

impl Columns {
    #[cfg(any(test, feature = "test-support"))]
    fn new(branch: u64, budget: &MemoryBudget) -> io::Result<Self> {
        let open = |column| Namespace::new(FAMILY, branch, column);
        Ok(Self {
            days: Facts::new(open(DAYS)?, budget.clone(), limits())?,
            sets: Facts::new(open(SETS)?, budget.clone(), limits())?,
            rosters: Facts::new(open(ROSTERS)?, budget.clone(), limits())?,
            operands: Facts::new(open(OPERANDS)?, budget.clone(), limits())?,
            theatres: Facts::new(open(THEATRES)?, budget.clone(), limits())?,
            pairs: Facts::new(open(PAIRS)?, budget.clone(), limits())?,
            activity: Facts::new(open(ACTIVITY)?, budget.clone(), limits())?,
        })
    }

    /// The columns `saved` checkpoints, read through `source`.
    pub fn restore<R: ReadPages>(
        source: &mut R,
        saved: &Checkpoints,
        budget: &MemoryBudget,
        cache_bytes: usize,
    ) -> io::Result<Self> {
        let limits = Limits {
            cache_bytes,
            ..limits()
        };
        Ok(Self {
            days: Facts::restore(source, &saved.days, budget.clone(), limits)?,
            sets: Facts::restore(source, &saved.sets, budget.clone(), limits)?,
            rosters: Facts::restore(source, &saved.rosters, budget.clone(), limits)?,
            operands: Facts::restore(source, &saved.operands, budget.clone(), limits)?,
            theatres: Facts::restore(source, &saved.theatres, budget.clone(), limits)?,
            pairs: Facts::restore(source, &saved.pairs, budget.clone(), limits)?,
            activity: Facts::restore(source, &saved.activity, budget.clone(), limits)?,
        })
    }
}

/// A tag set's slot in the operands, each deduplicated against its last.
#[cfg(any(test, feature = "test-support"))]
#[derive(Clone, Copy)]
pub(super) enum Slot {
    Present,
    Witnessed,
    Invalid,
}

/// What the facts writer carries from day to day: each dictionary's last
/// entry and next id, every region's running activity and the columns'
/// checkpoints; and, until the next flush, the facts of the days finished
/// since the last.
#[derive(Default, Serialize, Deserialize)]
pub(super) struct Ledger {
    checkpoints: Option<Checkpoints>,
    next: [u32; 4],
    sets: [Option<(Arc<BTreeSet<String>>, u32)>; 3],
    roster: Option<(Roster, Vec<String>, u32)>,
    operands: Option<(Operands, u32)>,
    theatres: Option<(Theatres, u32)>,
    totals: BTreeMap<u32, u128>,
    #[cfg(any(test, feature = "test-support"))]
    #[serde(skip)]
    columns: Option<Box<Columns>>,
    #[cfg(any(test, feature = "test-support"))]
    #[serde(skip)]
    pending: Pending,
    #[cfg(any(test, feature = "test-support"))]
    #[serde(skip)]
    held: Option<Box<Held>>,
}

/// The composition of the last day's operands and roster, by their ids,
/// with the roster's representatives in tag order.
#[cfg(any(test, feature = "test-support"))]
struct Held {
    operands: u32,
    roster: Option<u32>,
    day: daily::DayOperands,
    layout: Option<daily::Layout>,
    representatives: Vec<String>,
}

#[cfg(any(test, feature = "test-support"))]
#[derive(Default)]
struct Pending {
    sets: Vec<(u32, Vec<String>)>,
    rosters: Vec<(u32, Roster)>,
    operands: Vec<(u32, Operands)>,
    theatres: Vec<(u32, Theatres)>,
    days: Vec<(i32, Day)>,
    pairs: Vec<(i32, u32, Vec<Pair>)>,
    activity: Vec<(i32, u32, u128)>,
}

#[cfg(any(test, feature = "test-support"))]
fn next(counter: &mut u32) -> io::Result<u32> {
    let id = *counter;
    *counter = id
        .checked_add(1)
        .ok_or_else(|| invalid("Air fact dictionary id overflow"))?;
    Ok(id)
}

impl Ledger {
    /// What a query reads of a saved ledger: its columns' checkpoints.
    pub fn read(checkpoints: Option<Checkpoints>) -> Self {
        Self {
            checkpoints,
            ..Default::default()
        }
    }

    /// The id of `tags` in `slot`: the slot's last set's when equal.
    #[cfg(any(test, feature = "test-support"))]
    pub fn set(&mut self, slot: Slot, tags: &Arc<BTreeSet<String>>) -> io::Result<u32> {
        if let Some((set, id)) = &self.sets[slot as usize]
            && (Arc::ptr_eq(set, tags) || set == tags)
        {
            return Ok(*id);
        }
        let id = next(&mut self.next[0])?;
        self.pending.sets.push((id, tags.iter().cloned().collect()));
        self.sets[slot as usize] = Some((Arc::clone(tags), id));
        Ok(id)
    }

    /// The id of `roster` assigned under faction `names`: the last roster's
    /// when both are equal.
    #[cfg(any(test, feature = "test-support"))]
    pub fn roster(&mut self, roster: Roster, names: Vec<String>) -> io::Result<u32> {
        if let Some((held, held_names, id)) = &self.roster
            && *held == roster
            && *held_names == names
        {
            return Ok(*id);
        }
        let id = next(&mut self.next[1])?;
        self.pending.rosters.push((id, roster.clone()));
        self.roster = Some((roster, names, id));
        Ok(id)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn operands(&mut self, operands: Operands) -> io::Result<u32> {
        if let Some((held, id)) = &self.operands
            && *held == operands
        {
            return Ok(*id);
        }
        let id = next(&mut self.next[2])?;
        self.pending.operands.push((id, operands.clone()));
        self.operands = Some((operands, id));
        Ok(id)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn theatres(&mut self, theatres: Theatres) -> io::Result<u32> {
        if let Some((held, id)) = &self.theatres
            && *held == theatres
        {
            return Ok(*id);
        }
        let id = next(&mut self.next[3])?;
        self.pending.theatres.push((id, theatres.clone()));
        self.theatres = Some((theatres, id));
        Ok(id)
    }

    /// A finished day, with each active region's pairs and activity.
    #[cfg(any(test, feature = "test-support"))]
    pub fn day(&mut self, day: i32, fact: Day, regions: Vec<(u32, Vec<Pair>, u128)>) {
        for (region, pairs, activity) in regions {
            let total = self.totals.entry(region).or_default();
            *total = total.saturating_add(activity);
            self.pending.activity.push((day, region, *total));
            self.pending.pairs.push((day, region, pairs));
        }
        self.pending.days.push((day, fact));
    }

    /// Bytes held for the next flush.
    #[cfg(any(test, feature = "test-support"))]
    pub fn bytes(&self) -> usize {
        let pending = &self.pending;
        let tags = |tags: &[String]| tags.iter().map(String::capacity).sum::<usize>();
        pending.sets.iter().map(|(_, set)| tags(set)).sum::<usize>()
            + pending
                .rosters
                .iter()
                .flat_map(|(_, roster)| &roster.groups)
                .map(|group| tags(&group.members) + size_of::<Group>())
                .sum::<usize>()
            + pending.operands.len() * size_of::<Operands>()
            + pending
                .theatres
                .iter()
                .map(|(_, theatres)| theatres.regions.len() * 32)
                .sum::<usize>()
            + pending.days.len() * size_of::<Day>()
            + pending
                .pairs
                .iter()
                .map(|(_, _, pairs)| pairs.len() * size_of::<Pair>() + 32)
                .sum::<usize>()
            + pending.activity.len() * 32
    }

    /// Append the pending facts and checkpoint every column.
    #[cfg(any(test, feature = "test-support"))]
    pub fn flush(&mut self, writer: &mut store::Writer, branch: u64) -> io::Result<()> {
        let budget = MemoryBudget::preparation();
        if self.columns.is_none() {
            self.columns = Some(Box::new(match &self.checkpoints {
                Some(saved) => Columns::restore(writer, saved, &budget, 0)?,
                None => Columns::new(branch, &budget)?,
            }));
        }
        let columns = self.columns.as_mut().expect("opened columns");
        let pending = std::mem::take(&mut self.pending);
        let id =
            |id: u32| i32::try_from(id).map_err(|_| invalid("Air fact dictionary id overflow"));
        for (at, set) in pending.sets {
            let at = id(at)?;
            columns
                .sets
                .append(writer, (), at, at, Status::Value(set))?;
        }
        for (at, roster) in pending.rosters {
            let at = id(at)?;
            columns
                .rosters
                .append(writer, (), at, at, Status::Value(roster))?;
        }
        for (at, operands) in pending.operands {
            let at = id(at)?;
            columns
                .operands
                .append(writer, (), at, at, Status::Value(operands))?;
        }
        for (at, theatres) in pending.theatres {
            let at = id(at)?;
            columns
                .theatres
                .append(writer, (), at, at, Status::Value(theatres))?;
        }
        for (day, fact) in pending.days {
            columns
                .days
                .append(writer, (), day, day, Status::Value(fact))?;
        }
        for (day, region, pairs) in pending.pairs {
            columns
                .pairs
                .append(writer, region, day, day, Status::Value(pairs))?;
        }
        for (day, region, total) in pending.activity {
            columns
                .activity
                .append(writer, region, day, day, Status::Value(total))?;
        }
        self.checkpoints = Some(Checkpoints {
            days: columns.days.checkpoint(writer, 0)?,
            sets: columns.sets.checkpoint(writer, 0)?,
            rosters: columns.rosters.checkpoint(writer, 0)?,
            operands: columns.operands.checkpoint(writer, 0)?,
            theatres: columns.theatres.checkpoint(writer, 0)?,
            pairs: columns.pairs.checkpoint(writer, 0)?,
            activity: columns.activity.checkpoint(writer, 0)?,
        });
        Ok(())
    }

    pub fn checkpoints(&self) -> Option<&Checkpoints> {
        self.checkpoints.as_ref()
    }
}

/// A finished day as the observer holds it at the day's sample hour.
#[cfg(any(test, feature = "test-support"))]
pub(super) struct Sample<'a> {
    pub hour: i32,
    pub operands: &'a scan::ScoreOperands,
    pub auxiliary: Option<&'a scan::Auxiliary>,
    pub theatre_read: Option<i32>,
    /// The groups the hour assigns, None where it assigns no sides.
    pub roster: Option<&'a [Group]>,
    /// The groups' faction names, which start another roster when renamed.
    pub names: &'a [String],
    /// Visits each region's scores of a country, by region.
    pub pairs: &'a dyn Fn(&str, &mut dyn FnMut(u32, (i64, i64))) -> io::Result<()>,
    pub active: &'a BTreeSet<u32>,
}

#[cfg(any(test, feature = "test-support"))]
fn tags(set: &BTreeSet<String>) -> daily::Tags {
    set.iter().cloned().collect()
}

#[cfg(any(test, feature = "test-support"))]
impl Ledger {
    /// Record the finished day of `sample`.
    pub fn finish(&mut self, sample: Sample<'_>) -> io::Result<()> {
        let operands = sample.operands;
        let fact = Operands {
            identity_known: operands.identity_known,
            present: self.set(Slot::Present, &operands.present)?,
            witnessed: self.set(Slot::Witnessed, &operands.witnessed)?,
            invalid: self.set(Slot::Invalid, &operands.invalid)?,
            complete: operands.complete,
            unresolved: operands.unresolved,
        };
        let present = fact.present;
        let operands_id = self.operands(fact)?;
        let theatres = match sample.auxiliary {
            Some(auxiliary) => Theatres::read(auxiliary.theatres_known, &auxiliary.theatres),
            None => Theatres {
                known: false,
                regions: vec![],
            },
        };
        let theatres = self.theatres(theatres)?;
        let roster = match sample.roster.filter(|_| operands.identity_known) {
            Some(groups) => Some(self.roster(
                Roster {
                    present,
                    groups: groups.to_vec(),
                },
                sample.names.to_vec(),
            )?),
            None => None,
        };
        if self
            .held
            .as_ref()
            .is_none_or(|held| held.operands != operands_id || held.roster != roster)
        {
            let day = daily::DayOperands {
                identity_known: operands.identity_known,
                present: tags(&operands.present),
                witnessed: tags(&operands.witnessed),
                invalid: tags(&operands.invalid),
                complete: operands.complete,
                unresolved: operands.unresolved,
            };
            let layout = match (roster, sample.roster) {
                (Some(_), Some(groups)) => Some(daily::Layout::new(
                    groups.to_vec(),
                    Arc::clone(&day.present),
                )?),
                _ => None,
            };
            let mut representatives = Vec::new();
            if let Some(layout) = &layout {
                for position in 0..layout.len() {
                    match layout.side(position) {
                        daily::Side::Lone(tag) => representatives.push(tag.clone()),
                        daily::Side::Group(group) => match &group.leader {
                            Some(leader) => representatives.push(leader.clone()),
                            None if group.factions.is_empty() => {
                                representatives.extend(group.members.iter().cloned())
                            }
                            None => {}
                        },
                    }
                }
            }
            representatives.sort_unstable();
            self.held = Some(Box::new(Held {
                operands: operands_id,
                roster,
                day,
                layout,
                representatives,
            }));
        }
        let held = self.held.as_ref().expect("held composition");
        let compose = daily::Compose::new(held.layout.as_ref(), &held.day, daily::View::Sides);
        // Only a side's representative supplies its read: no other country's pairs are kept.
        let mut by_region: BTreeMap<u32, Vec<Pair>> = sample
            .active
            .iter()
            .map(|region| (*region, Vec::new()))
            .collect();
        for tag in &held.representatives {
            (sample.pairs)(tag, &mut |region, scores| {
                if let Some(pairs) = by_region.get_mut(&region) {
                    pairs.push((tag.clone(), scores));
                }
            })?;
        }
        let used = compose.uses(by_region.values().map(Vec::as_slice));
        let excluded = compose.excluded() as u64;
        let shows_all = compose.shows_all();
        let regions = by_region
            .into_iter()
            .map(|(region, pairs)| {
                let activity = compose.activity(&pairs);
                (region, pairs, activity)
            })
            .collect();
        let day = sample.hour.div_euclid(DAY_HOURS);
        self.day(
            day,
            Day {
                offset: sample.hour - day * DAY_HOURS,
                roster,
                operands: operands_id,
                theatres,
                theatre_age: sample.theatre_read.map(|read| sample.hour - read),
                excluded,
                used: (!shows_all).then_some(used),
            },
            regions,
        );
        Ok(())
    }
}

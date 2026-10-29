use super::{evidence::Joins, proposals, tuples::TupleRead, *};

mod projector;
mod rows;
pub use projector::Projector;

#[cfg(test)]
mod parity;

/// One identity read, shared with the emit; a country is its index into `countries`.
pub(super) struct Ids {
    joins: Joins,
    countries: Vec<DiplomacyCountry>,
    /// By raw id.
    index: Vec<(u32, u32)>,
    /// Each country's raw aliases, in `joins`' order.
    aliases: Vec<Box<[u32]>>,
    /// By scheduled slice.
    slices: Vec<Vec<u32>>,
    /// Every Countries row's id, ascending.
    canonical: Vec<u32>,
}
impl Ids {
    fn index_of(&self, raw: u32) -> Option<u32> {
        self.index
            .binary_search_by_key(&raw, |&(raw, _)| raw)
            .ok()
            .map(|at| self.index[at].1)
    }
    fn position(&self, country: &DiplomacyCountry) -> Option<u32> {
        self.countries
            .binary_search(country)
            .ok()
            .map(|at| at as u32)
    }
}
/// An opinion owner's known pairs by target index, and its rows refusing them.
pub(super) struct Values {
    /// Shared while the owner's targets do not change.
    targets: Arc<[u32]>,
    values: Box<[i32]>,
    unresolved: usize,
}
/// A raw tuple owner's rows, in key order.
pub(super) struct TupleRows {
    /// Each row's spelling; shared while no row's key, endpoints, start or spelling changes.
    identity: Arc<[Option<Arc<str>>]>,
    rows: Box<[RelationTupleRow]>,
}
/// The pacts a raw owner's rows were observed as, and at which read.
pub(super) struct TupleMemo {
    identity: Arc<[Option<Arc<str>>]>,
    keys: Vec<u64>,
    read: i32,
    settled: bool,
}
/// A selected opinion owner's index, supplying read and values.
type Opinion = (u32, Option<i32>, Arc<Values>);
/// A selected raw tuple owner, its supplying read and rows.
type TupleOwner = (u32, i32, Arc<TupleRows>);
struct Opinions {
    current: Vec<Opinion>,
    baseline: Vec<Opinion>,
    orphans: usize,
}
struct Tuples {
    countries: Vec<(u32, i32)>,
    baseline_countries: Vec<(u32, i32)>,
    owners: Vec<TupleOwner>,
    baseline_owners: Vec<TupleOwner>,
    orphans: Option<BTreeSet<u32>>,
}
enum QueueRead {
    Unavailable,
    Carried,
    Fresh(Arc<proposals::Queue>),
}
pub struct Hour {
    pub branch: u64,
    pub hour: i32,
    /// The identities when the hour read them again; otherwise the hour before's hold.
    ids: Option<Arc<Ids>>,
    refused: bool,
    /// Whether the ready tables carry the owners only a baseline reads.
    baseline: bool,
    opinions: Option<Opinions>,
    tuples: Option<Tuples>,
    queue: QueueRead,
}

/// `current` and `added`, whose indexes are not in `current`, in index order.
fn merged<T>(
    mut current: Vec<T>,
    added: impl IntoIterator<Item = T>,
    index: impl Fn(&T) -> u32,
) -> Vec<T> {
    current.extend(added);
    current.sort_unstable_by_key(index);
    current
}

impl Prepared {
    fn store_projected_opinion(&mut self, at: i32, ids: &Ids, (index, read, values): Opinion) {
        let country = &ids.countries[index as usize];
        let known = read.is_some() && values.unresolved == 0;
        let changed = self.owners.get(country).and_then(|owner| {
            let memo = owner.projected.as_ref()?;
            if owner.known == known && Arc::ptr_eq(memo, &values) {
                Some(false)
            } else if owner.known && known && Arc::ptr_eq(&memo.targets, &values.targets) {
                Some(true)
            } else {
                None
            }
        });
        let Some(changed) = changed else {
            let pairs: Vec<_> = values
                .targets
                .iter()
                .zip(values.values.iter())
                .map(|(&target, &value)| (&ids.countries[target as usize], value))
                .collect();
            let unresolved = values.unresolved;
            self.store_opinion_owner(at, country.clone(), read, &pairs, unresolved, Some(values));
            return;
        };
        // The same targets hold the same pairs, so only a changed value is observed.
        if changed {
            let (held, mut owner) = self
                .owners
                .remove_entry(country)
                .expect("held opinion owner");
            for ((target, pair), &value) in owner.pairs.iter_mut().zip(values.values.iter()) {
                if pair.value != value {
                    pair.value = value;
                    pair.key = Some(self.pair_number(
                        &held,
                        target,
                        pair.key,
                        at,
                        Some(i128::from(value)),
                        false,
                    ));
                }
            }
            owner.projected = Some(values);
            self.owners.insert(held, owner);
        }
        self.owners
            .get_mut(country)
            .expect("held opinion owner")
            .read = read;
        self.remember_opinion_deadline(country, read);
        self.country_number(
            Series::Owner,
            country,
            at,
            known.then(|| i128::from(read.expect("known supplying read"))),
            !known,
        );
        self.country_number(Series::Read, country, at, read.map(i128::from), false);
    }
    fn store_projected_tuples(
        &mut self,
        hour: i32,
        ids: &Ids,
        (raw, read, rows): TupleOwner,
    ) -> io::Result<()> {
        self.remember_raw_tuple_deadline(raw, read);
        let held = self
            .raw_tuple_reads
            .entry(raw)
            .or_insert(RawRead { read, memo: None });
        held.read = read;
        // Endpoints resolved at an earlier read stay resolved at a later one.
        let memo = held.memo.take().filter(|memo| {
            Arc::ptr_eq(&memo.identity, &rows.identity)
                && memo.read <= read
                && (memo.settled || memo.read == read)
        });
        let memo = if let Some(mut memo) = memo {
            for (&key, row) in memo.keys.iter().zip(rows.rows.iter()) {
                let pact = &mut self.pacts.get_mut(&key).expect("held pact").reply;
                pact.last_seen = read;
                pact.read_hour = read;
                pact.end_hour = date(row.end_hour);
                pact.cancelled = boolean(row.cancel);
                pact.retired = boolean(row.retired);
                pact.cancel_raw = row.cancel;
                pact.retired_raw = row.retired;
                *self
                    .catalogue
                    .kinds
                    .get_mut(&pact.kind)
                    .expect("held pact spelling") = row.type_id;
                self.dirty.insert(key);
            }
            memo.read = read;
            memo
        } else {
            let joins = &ids.joins;
            let observed = TupleRead {
                hour,
                read_hour: read,
                joins,
                tokens: None,
            };
            let mut keys = Vec::with_capacity(rows.rows.len());
            let mut found = BTreeSet::new();
            let mut settled = true;
            for (row, spelling) in rows.rows.iter().zip(rows.identity.iter()) {
                let key =
                    self.observe_pact(row, &observed, spelling.as_deref().map(str::to_owned))?;
                keys.push(key);
                found.insert(key);
                for raw in [
                    row.owner_country,
                    row.target_country,
                    row.first_country,
                    row.second_country,
                ] {
                    settled &= !joins.raw.contains_key(&raw)
                        || self.role_at_read(joins, raw, read).is_some();
                }
            }
            self.end_missing_pacts(raw, &found);
            TupleMemo {
                identity: Arc::clone(&rows.identity),
                keys,
                read,
                settled,
            }
        };
        self.raw_tuple_reads
            .get_mut(&raw)
            .expect("held raw tuple read")
            .memo = Some(memo);
        Ok(())
    }
    pub fn observe_projected(&mut self, hour: Hour) -> io::Result<()> {
        let _cpu = replay_frame::cpu::scope("emit", Some("diplomacy_state.hour"));
        let at = hour.hour;
        let continuous = self.begin_hour(at)?;
        if hour.refused {
            self.refuse_joins(at);
            return Ok(());
        }
        if self.segment_first == Some(at) && !hour.baseline {
            return Err(io::Error::other(
                "A projected hour lacks the baseline its continuation admits",
            ));
        }
        let read = hour.ids.is_some();
        if let Some(ids) = hour.ids {
            // A memo's roles were dated with the identities it was observed with.
            for held in self.raw_tuple_reads.values_mut() {
                held.memo = None;
            }
            self.ids = Some(ids);
        }
        let ids = self
            .ids
            .clone()
            .ok_or_else(|| io::Error::other("A projected hour lacks its identities"))?;
        if read {
            self.update_live_countries(&ids.joins, at, continuous);
        } else if !continuous {
            let live = std::mem::take(&mut self.live_countries);
            for country in &live {
                self.country_number(Series::Identity, country, at, Some(1), true);
            }
            self.live_countries = live;
        }
        self.observe_projected_opinions(at, &ids, hour.opinions);
        self.observe_projected_tuples(at, &ids, hour.tuples)?;
        self.observe_projected_queue(at, hour.queue)?;
        self.publish_unresolved(at, ids.joins.unresolved);
        Ok(())
    }
    fn observe_projected_opinions(&mut self, at: i32, ids: &Ids, opinions: Option<Opinions>) {
        let _cpu = replay_frame::cpu::scope("emit", Some("diplomacy_state.opinions"));
        self.number(Fixed::OpinionCoverage, at, opinions.as_ref().map(|_| 1));
        let Some(Opinions {
            current,
            baseline,
            orphans,
        }) = opinions
        else {
            self.close_opinion_reads(at);
            return;
        };
        self.expire_opinion_reads(at, |country| {
            ids.position(country).is_some_and(|country| {
                current
                    .binary_search_by_key(&country, |opinion| opinion.0)
                    .is_ok()
            })
        });
        self.orphan_opinions = orphans;
        let current = if self.segment_first == Some(at) {
            let owners = &self.owners;
            merged(
                current,
                baseline
                    .into_iter()
                    .filter(|opinion| !owners.contains_key(&ids.countries[opinion.0 as usize])),
                |opinion| opinion.0,
            )
        } else {
            current
        };
        for opinion in current {
            self.store_projected_opinion(at, ids, opinion);
        }
    }
    fn observe_projected_tuples(
        &mut self,
        at: i32,
        ids: &Ids,
        tuples: Option<Tuples>,
    ) -> io::Result<()> {
        let _cpu = replay_frame::cpu::scope("emit", Some("diplomacy_state.tuples"));
        self.number(Fixed::TupleCoverage, at, tuples.as_ref().map(|_| 1));
        let Some(Tuples {
            countries,
            baseline_countries,
            owners,
            baseline_owners,
            orphans,
        }) = tuples
        else {
            self.end_pacts(None, true);
            self.close_tuple_reads(at);
            return Ok(());
        };
        self.expire_tuple_reads(at, &ids.joins, |country| {
            ids.position(country).is_some_and(|country| {
                countries
                    .binary_search_by_key(&country, |read| read.0)
                    .is_ok()
            })
        });
        let (countries, owners) = if self.segment_first == Some(at) {
            (
                merged(countries, baseline_countries, |read| read.0),
                merged(owners, baseline_owners, |owner| owner.0),
            )
        } else {
            (countries, owners)
        };
        for (country, read) in countries {
            let country = &ids.countries[country as usize];
            self.country_number(Series::Tuples, country, at, Some(i128::from(read)), false);
            self.remember_tuple_deadline(country, read);
            match self.tuple_reads.get_mut(country) {
                Some(held) => *held = read,
                None => {
                    self.tuple_reads.insert(country.clone(), read);
                }
            }
        }
        if let Some(orphans) = orphans {
            self.orphan_tuple_owners = orphans;
        }
        self.expire_raw_tuple_reads(at, |raw| {
            owners.binary_search_by_key(&raw, |owner| owner.0).is_ok()
        });
        for owner in owners {
            self.store_projected_tuples(at, ids, owner)?;
        }
        Ok(())
    }
    fn observe_projected_queue(&mut self, at: i32, queue: QueueRead) -> io::Result<()> {
        let _cpu = replay_frame::cpu::scope("emit", Some("diplomacy_state.queue"));
        match queue {
            QueueRead::Unavailable => {
                self.suspend_queue();
                self.number(Fixed::QueueCoverage, at, None);
            }
            QueueRead::Carried => {
                self.number(Fixed::QueueCoverage, at, self.queue_read.map(i128::from))
            }
            QueueRead::Fresh(queue) => self.observe_queue(at, &queue, &mut || Ok(()))?,
        }
        Ok(())
    }
}

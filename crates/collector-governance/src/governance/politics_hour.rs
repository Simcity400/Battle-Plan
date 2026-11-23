use super::{CountryRow, LeaderReuse, LeaderStore, N, Observation, PartyStore, PoliticsReuse};
use crate::{
    party_changes, party_leader_changes,
    political_status_changes::{self, Counts},
    table_capture::grow_vec,
};
use engine_allocation as allocation;
use engine_native::{
    Memory,
    ccountry,
    memory::{RecordField, RecordLayout, RecordSources},
    walk::{Buffer, Outcome},
};
use engine_native_politics::{
    cpolitical_party::{self, LeaderRow, Row as PartyRow},
    cpolitical_status::{self, IdeaRow},
};
use std::ops::Range;

type Certified = (u64, u64);

const POPULARITY: [RecordField; 1] = [RecordField {
    source_offset: cpolitical_party::POPULARITY,
    destination_offset: 0,
    len: 8,
}];
const POPULARITY_LAYOUT: RecordLayout<'static> = match RecordLayout::new(&POPULARITY, 8) {
    Some(layout) => layout,
    None => panic!("popularity layout"),
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Key {
    structure: Certified,
    parties: Certified,
    leaders: Certified,
    scope: (usize, usize),
}

#[derive(Clone, Debug, Default)]
pub(super) struct Visit {
    pub slot: u32,
    pub pointer: usize,
    pub id: u32,
    pub politics: usize,
    pub parties: Range<usize>,
    pub party_text: Range<usize>,
    pub ideas: Range<usize>,
    pub idea_text: Range<usize>,
    pub leaders: Range<usize>,
}

impl Visit {
    fn within(&self, lens: &[usize; 6]) -> bool {
        let fits = |range: &Range<usize>, len: usize| range.start <= range.end && range.end <= len;
        self.politics < lens[0]
            && fits(&self.parties, lens[1])
            && fits(&self.party_text, lens[2])
            && fits(&self.ideas, lens[3])
            && fits(&self.idea_text, lens[4])
            && fits(&self.leaders, lens[5])
    }
}

pub(super) struct Tables<'t> {
    pub politics: &'t mut Vec<CountryRow<cpolitical_status::Row>>,
    pub parties: &'t mut Vec<PartyRow>,
    pub party_text: &'t mut Vec<u8>,
    pub party_countries: &'t mut Vec<u32>,
    pub ideas: &'t mut Vec<IdeaRow>,
    pub idea_text: &'t mut Vec<u8>,
    pub idea_countries: &'t mut Vec<u32>,
    pub leaders: &'t mut Vec<LeaderRow>,
}

#[derive(Default)]
struct Outputs {
    countries: Vec<Visit>,
    politics: Vec<CountryRow<cpolitical_status::Row>>,
    parties: Vec<PartyRow>,
    pointers: Vec<usize>,
    party_text: Vec<u8>,
    party_required: usize,
    ideas: Vec<IdeaRow>,
    idea_text: Vec<u8>,
    idea_required: usize,
    leaders: Vec<LeaderRow>,
}

impl Outputs {
    fn clear(&mut self) {
        self.countries.clear();
        self.politics.clear();
        self.parties.clear();
        self.pointers.clear();
        self.party_text.clear();
        self.ideas.clear();
        self.idea_text.clear();
        self.leaders.clear();
        self.party_required = 0;
        self.idea_required = 0;
    }

    fn measure(&self) -> usize {
        self.countries.capacity() * size_of::<Visit>()
            + self.politics.capacity() * size_of::<CountryRow<cpolitical_status::Row>>()
            + self.parties.capacity() * size_of::<PartyRow>()
            + self.pointers.capacity() * size_of::<usize>()
            + self.party_text.capacity()
            + self.ideas.capacity() * size_of::<IdeaRow>()
            + self.idea_text.capacity()
            + self.leaders.capacity() * size_of::<LeaderRow>()
    }
}

#[derive(Default)]
pub struct QuietWalk {
    held: Outputs,
    key: Option<Key>,
    popularity: Option<Certified>,
    visits: Vec<Visit>,
    pointers: Vec<usize>,
    country: usize,
    fresh: Vec<usize>,
    recorded: bool,
    gathered: Vec<u8>,
    counts: Counts,
}

impl QuietWalk {
    pub(super) fn begin(&mut self) {
        self.visits.clear();
        self.pointers.clear();
        self.fresh.clear();
        self.country = 0;
        self.recorded = true;
    }

    pub(super) fn start_country(&mut self) {
        self.country = self.pointers.len();
        self.fresh.clear();
    }

    pub(super) fn fresh_party(&mut self, party: usize) {
        self.recorded &= push(&mut self.fresh, party);
    }

    pub(super) fn read_parties(&mut self) {
        let stored =
            allocation::advisory(|| allocation::reserve(&mut self.pointers, self.fresh.len()));
        if stored.is_err() {
            self.recorded = false;
            return;
        }
        self.pointers.extend_from_slice(&self.fresh);
    }

    pub(super) fn carried_parties(&mut self, parties: impl ExactSizeIterator<Item = usize>) {
        let stored =
            allocation::advisory(|| allocation::reserve(&mut self.pointers, parties.len()));
        if stored.is_err() {
            self.recorded = false;
            return;
        }
        self.pointers.extend(parties);
    }

    pub(super) fn visit(&mut self, visit: Visit) {
        self.recorded &= visit.parties == (self.country..self.pointers.len());
        self.recorded &= push(&mut self.visits, visit);
    }

    pub(super) fn measure(&self) -> usize {
        self.held.measure()
            + self.visits.capacity() * size_of::<Visit>()
            + (self.pointers.capacity() + self.fresh.capacity()) * size_of::<usize>()
            + self.gathered.capacity()
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(super) fn counts(&self) -> Counts {
        self.counts
    }

    fn same_countries<M: Memory>(&self, countries: &ccountry::Reader<'_, M>) -> bool {
        let held = &self.held.countries;
        let mut next = 0;
        for slot in 1..countries.count() {
            match countries.at(slot) {
                Ok(None) => {}
                Ok(Some(country)) => {
                    if held.get(next).is_none_or(|visit| {
                        (visit.slot, visit.pointer, visit.id) != (slot, country.pointer, country.id)
                    }) {
                        return false;
                    }
                    next += 1;
                }
                Err(_) => return false,
            }
        }
        next == held.len()
    }

    fn refresh_popularity<M: Memory>(&mut self, memory: &M) -> bool {
        let Self { held, gathered, .. } = self;
        let Some(bytes) = held.pointers.len().checked_mul(8) else {
            return false;
        };
        if held.pointers.len() != held.parties.len() || !grow_vec(gathered, bytes) {
            return false;
        }
        let gathered = &mut gathered[..bytes];
        if memory.copy_records(
            RecordSources::Pointers(&held.pointers),
            &POPULARITY_LAYOUT,
            gathered,
        ) != held.pointers.len()
        {
            return false;
        }
        for (row, popularity) in held.parties.iter_mut().zip(gathered.chunks_exact(8)) {
            row.popularity = i64::from_le_bytes(popularity.try_into().unwrap());
        }
        true
    }

    fn publish(&self, slots: usize, tables: &mut Tables<'_>, o: &mut Observation) -> bool {
        let held = &self.held;
        let ready = grow_vec(tables.politics, slots)
            && tables.politics.len() >= held.politics.len()
            && grow(tables.parties, held.parties.len())
            && grow(tables.party_text, held.party_text.len())
            && grow_vec(tables.party_countries, held.parties.len())
            && grow(tables.ideas, held.ideas.len())
            && grow(tables.idea_text, held.idea_text.len())
            && grow(tables.idea_countries, held.ideas.len())
            && grow(tables.leaders, held.leaders.len());
        if !ready {
            return false;
        }
        tables.politics[..held.politics.len()].copy_from_slice(&held.politics);
        tables.parties[..held.parties.len()].copy_from_slice(&held.parties);
        tables.party_text[..held.party_text.len()].copy_from_slice(&held.party_text);
        tables.ideas[..held.ideas.len()].copy_from_slice(&held.ideas);
        tables.idea_text[..held.idea_text.len()].copy_from_slice(&held.idea_text);
        tables.leaders[..held.leaders.len()].copy_from_slice(&held.leaders);
        for country in &held.countries {
            tables.party_countries[country.parties.clone()].fill(country.id);
            tables.idea_countries[country.ideas.clone()].fill(country.id);
        }
        o.outcomes[5] = Outcome::Rows {
            len: held.politics.len(),
        };
        o.outcomes[6] = Outcome::Rows {
            len: held.parties.len(),
        };
        o.outcomes[7] = Outcome::Rows {
            len: held.ideas.len(),
        };
        o.outcomes[21] = Outcome::Rows {
            len: held.leaders.len(),
        };
        o.lens[2] = held.party_text.len();
        o.required[2] = held.party_required;
        o.lens[3] = held.idea_text.len();
        o.required[3] = held.idea_required;
        true
    }

    fn commit(
        &mut self,
        key: Option<Key>,
        popularity: Option<Certified>,
        tables: &Tables<'_>,
        o: &Observation,
    ) {
        self.key = None;
        self.popularity = None;
        self.held.clear();
        let lens = [
            o.outcomes[5].len(),
            o.outcomes[6].len(),
            o.lens[2],
            o.outcomes[7].len(),
            o.lens[3],
            o.outcomes[21].len(),
        ];
        let Some(key) = key.filter(|_| {
            self.recorded
                && self.pointers.len() == lens[1]
                && self.visits.iter().all(|visit| visit.within(&lens))
        }) else {
            return;
        };
        let held = &mut self.held;
        let stored = allocation::advisory(|| {
            allocation::reserve(&mut held.politics, lens[0])?;
            allocation::reserve(&mut held.parties, lens[1])?;
            allocation::reserve(&mut held.party_text, lens[2])?;
            allocation::reserve(&mut held.ideas, lens[3])?;
            allocation::reserve(&mut held.idea_text, lens[4])?;
            allocation::reserve(&mut held.leaders, lens[5])?;
            std::io::Result::Ok(())
        });
        if stored.is_err() {
            return;
        }
        std::mem::swap(&mut held.countries, &mut self.visits);
        std::mem::swap(&mut held.pointers, &mut self.pointers);
        held.politics.extend_from_slice(&tables.politics[..lens[0]]);
        held.parties.extend_from_slice(&tables.parties[..lens[1]]);
        held.party_text
            .extend_from_slice(&tables.party_text[..lens[2]]);
        held.ideas.extend_from_slice(&tables.ideas[..lens[3]]);
        held.idea_text
            .extend_from_slice(&tables.idea_text[..lens[4]]);
        held.leaders.extend_from_slice(&tables.leaders[..lens[5]]);
        held.party_required = o.required[2];
        held.idea_required = o.required[3];
        let _ = allocation::advisory(|| {
            allocation::resize_default(&mut self.gathered, lens[1].saturating_mul(8))
        });
        self.key = Some(key);
        self.popularity = popularity;
    }
}

fn grow<T: Default>(values: &mut Vec<T>, len: usize) -> bool {
    Buffer::Growing(values).ensure(len).unwrap_or(false)
}

fn push<T>(values: &mut Vec<T>, value: T) -> bool {
    if allocation::advisory(|| allocation::reserve(values, 1)).is_err() {
        return false;
    }
    values.push(value);
    true
}

pub struct PoliticsWalk<'a> {
    pub leaders: Option<&'a mut LeaderStore>,
    pub parties: Option<&'a mut PartyStore>,
    pub leader_changes: &'a dyn party_leader_changes::Changes,
    pub party_changes: &'a dyn party_changes::Changes,
    pub status: &'a dyn political_status_changes::Changes,
    pub force_full: bool,
}

pub(super) struct Hour<'a> {
    walk: PoliticsWalk<'a>,
    key: Option<Key>,
    popularity: Option<Certified>,
    usual: bool,
    quiet: bool,
    counts: Counts,
}

impl<'a> Hour<'a> {
    pub(super) fn start(
        walk: PoliticsWalk<'a>,
        base: usize,
        game_state: usize,
        enabled: &[bool; N],
    ) -> Self {
        let key = match (
            walk.status.structure(),
            walk.party_changes.quiet(),
            walk.leader_changes.quiet(),
        ) {
            (Some(structure), Some(parties), Some(leaders)) => Some(Key {
                structure,
                parties,
                leaders,
                scope: (base, game_state),
            }),
            _ => None,
        };
        Self {
            popularity: walk.status.popularity(),
            walk,
            key,
            usual: [5, 6, 7, 21].iter().all(|&index| enabled[index]),
            quiet: false,
            counts: Counts::default(),
        }
    }

    fn holds<M: Memory>(&self, countries: &ccountry::Reader<'_, M>) -> bool {
        let walk = &self.walk;
        let (Some(key), Some(_), Some(parties), Some(leaders)) = (
            self.key,
            self.popularity,
            walk.parties.as_deref(),
            walk.leaders.as_deref(),
        ) else {
            return false;
        };
        if !self.usual || walk.force_full {
            return false;
        }
        let (Some(quiet), Some((held, characters))) = (parties.quiet(), leaders.held()) else {
            return false;
        };
        if quiet.key != Some(key) || held != quiet.held.countries.len() {
            return false;
        }
        if !characters.iter().all(|&(character, certified)| {
            walk.leader_changes.character(character) == Some(certified)
        }) {
            return false;
        }
        quiet.same_countries(countries)
    }

    pub(super) fn quiet<M: Memory>(
        &mut self,
        memory: &M,
        countries: &ccountry::Reader<'_, M>,
        tables: &mut Tables<'_>,
        o: &mut Observation,
    ) -> bool {
        let holds = self.holds(countries);
        let (Some(popularity), Some(store), true) =
            (self.popularity, self.walk.parties.as_deref_mut(), holds)
        else {
            return false;
        };
        let quiet = store.quiet_mut();
        let refresh = quiet.popularity != Some(popularity);
        if refresh {
            if !quiet.refresh_popularity(memory) {
                return false;
            }
            quiet.popularity = Some(popularity);
        }
        if !quiet.publish(countries.count() as usize, tables, o) {
            return false;
        }
        self.quiet = true;
        self.counts.quiet = !refresh;
        self.counts.popularity = refresh;
        true
    }

    pub(super) fn election_cache_certified(&self) -> bool {
        self.usual && self.key.is_some()
    }

    pub(super) fn reuses(&mut self) -> (Option<LeaderReuse<'_>>, Option<PoliticsReuse<'_>>) {
        let walk = &mut self.walk;
        let (leader_changes, party_changes) = (walk.leader_changes, walk.party_changes);
        (
            walk.leaders
                .as_deref_mut()
                .map(|store| LeaderReuse::begin(store, leader_changes)),
            walk.parties
                .as_deref_mut()
                .map(|store| PoliticsReuse::begin(store, party_changes)),
        )
    }

    pub(super) fn finish(mut self, tables: &Tables<'_>, o: &Observation, census: bool) {
        if !self.quiet {
            self.counts.read = true;
            let complete = census
                && self.usual
                && [5, 6, 7, 21]
                    .iter()
                    .all(|&index| matches!(o.outcomes[index], Outcome::Rows { .. }));
            let leaders = self
                .walk
                .leaders
                .as_deref()
                .and_then(LeaderStore::held)
                .map(|(held, _)| held);
            if let Some(store) = self.walk.parties.as_deref_mut() {
                let quiet = store.quiet_mut();
                let key = self
                    .key
                    .filter(|_| complete && leaders == Some(quiet.visits.len()));
                quiet.commit(key, self.popularity, tables, o);
            }
        }
        if let Some(store) = self.walk.parties.as_deref_mut() {
            store.quiet_mut().counts = self.counts;
            store.remeasure();
        }
        political_status_changes::record(&self.counts);
    }
}

//! A quiet politics hour: Politics, PoliticalParties, ActiveIdeas and
//! PartyLeaders published whole from the latest complete walk, without
//! reading any country's political status, parties, ideas or leaders
//! ([`crate::political_status_changes`]).
//!
//! Every value the Politics walk publishes for a country, and what proves it
//! unchanged since the walk that read it:
//! - The country census: this hour's `countries.at(slot)` over every slot
//!   must list the walk's (slot, pointer, id) sequence; with the callback's
//!   authenticated census this reads nothing. The status pointer
//!   `ccountry::POLITICAL_STATUS`, the status's vtable and its owner backlink
//!   (`cpolitical_status::COUNTRY`)
//!   are written only by their constructors, and countries are reused,
//!   never rebuilt.
//! - The capital (`ccountry::CAPITAL`) and original capital: set
//!   capital and country appearance reset. The last election: set
//!   politics and the daily politics update's election-day store. The
//!   election frequency and elections byte: the politics setter and
//!   the party clear (below). The ruling party: force ruling party
//!   and set ruling party. The party array and count: the party build
//!   and clear, reached at runtime only from country appearance reset. The
//!   idea array and count: set ideas. Each moves the political status
//!   structure token.
//! - Each party's popularity (`cpolitical_party::POPULARITY`): add popularity, set
//!   popularity redistributing, the clamped setter's calls from the party
//!   popularity and popularities effects, force ruling party, country
//!   appearance reset and the daily politics update, which rewrites every
//!   living country's popularity once a day in one hour. Each moves the
//!   popularity token.
//! - A party's id, ideology token, tag, names and default flag: the party
//!   domain's token ([`crate::party_changes`]). A party's leader vector, and
//!   a leader's character link and id, written only on a new leader a hooked
//!   insert lists: the party-leader domain's token
//!   ([`crate::party_leader_changes`]).
//! - A leader's character identity and role key: that character's
//!   certificate in the character domain ([`crate::character_changes`]),
//!   whose token moves too often to use; each certificate the party-leader
//!   store holds is checked one by one, which reads no game memory.
//! - An idea's definition token: fixed by the idea database for the session.
//!
//! The walk's key is the structure, party and party-leader tokens and the
//! scope, taken before its first read or registration. An hour is quiet when
//! its key is the held walk's, the census lists the held countries, every
//! held leader's character keeps its certificate, the four tables are all
//! walked, no full refresh is forced and no `*_verify` feature
//! (`political_status_verify`, `party_verify`, `party_leader_verify`) is in
//! force. When the popularity token also equals the held one the hour reads
//! nothing; otherwise it gathers every held party's popularity afresh in one
//! batched guarded read and writes it into the held rows first (a failed
//! gather walks). Either way it then reads its sampled slice of countries
//! ([`crate::sampled_verify`], by country id) afresh, as the walk reads them,
//! and compares each with its held rows, popularity and ruling flag
//! included; a difference or a failed read is a missed write: it poisons
//! both domains and the hour walks every country. The held tables are
//! published into the hour's buffers exactly as a walk of the same memory
//! appends them, and the other tables of the country loop still run.
//!
//! `political_status_verify`, on during burn-in, never publishes a quiet
//! hour: every hour walks, and when the hour would have been quiet it
//! compares each country's walked rows with what the quiet hour would have
//! published, poisoning at a difference.
//!
//! The held tables live in the party store and follow the Governance job's
//! loan: a failed job forgets them with the store, and a walk that does not
//! complete all four tables, lacks a token, or whose countries the
//! party-leader store did not all hold, holds nothing. A quiet hour neither
//! begins nor finishes the party and party-leader stores, so the next walk
//! still carries per country from them. Allocation is worker-only and
//! advisory for the held copy: one that cannot be held only makes the next
//! hour walk.
use super::{CountryRow, LeaderReuse, LeaderStore, N, Observation, PartyStore, PoliticsReuse};
use crate::{
    party_changes, party_leader_changes,
    political_status_changes::{self, Counts},
    sampled_verify::Check,
    table_capture::grow_vec,
};
use engine::{
    Memory, allocation, ccountry,
    cpolitical_party::{self, LeaderRow, Row as PartyRow},
    cpolitical_status::{self, Fresh, FreshIdeas, FreshParties, IdeaRow},
    memory::{RecordField, RecordLayout, RecordSources},
    walk::{Buffer, Outcome},
};
use std::ops::Range;

type Certified = (u64, u64);

/// One party's popularity, gathered from its pointer.
const POPULARITY: [RecordField; 1] = [RecordField {
    source_offset: cpolitical_party::POPULARITY,
    destination_offset: 0,
    len: 8,
}];
const POPULARITY_LAYOUT: RecordLayout<'static> = match RecordLayout::new(&POPULARITY, 8) {
    Some(layout) => layout,
    None => panic!("popularity layout"),
};

/// What a walk's tables rest on, taken before its first read or
/// registration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Key {
    structure: Certified,
    parties: Certified,
    leaders: Certified,
    scope: (usize, usize),
}

/// One country a walk visited and its rows in each table's buffers; text
/// ranges index their arena.
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

/// The walk's politics buffers, as the Governance walk fills them.
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

impl Tables<'_> {
    fn rows(&self, visit: &Visit) -> CountryRows<'_> {
        CountryRows {
            politics: &self.politics[visit.politics],
            parties: &self.parties[visit.parties.clone()],
            party_text: &self.party_text[visit.party_text.clone()],
            party_base: visit.party_text.start,
            ideas: &self.ideas[visit.ideas.clone()],
            idea_text: &self.idea_text[visit.idea_text.clone()],
            idea_base: visit.idea_text.start,
            leaders: &self.leaders[visit.leaders.clone()],
        }
    }
}

/// One country's rows; party and idea text spans start at their base.
struct CountryRows<'r> {
    politics: &'r CountryRow<cpolitical_status::Row>,
    parties: &'r [PartyRow],
    party_text: &'r [u8],
    party_base: usize,
    ideas: &'r [IdeaRow],
    idea_text: &'r [u8],
    idea_base: usize,
    leaders: &'r [LeaderRow],
}

/// Whether two reads of a country publish the same rows, popularity and
/// ruling flag included.
fn same(held: &CountryRows<'_>, fresh: &CountryRows<'_>) -> bool {
    let party = |row: &PartyRow, base: usize| PartyRow {
        token_start: row.token_start.wrapping_sub(base),
        name_start: row.name_start.wrapping_sub(base),
        long_name_start: row.long_name_start.wrapping_sub(base),
        ..*row
    };
    let idea = |row: &IdeaRow, base: usize| IdeaRow {
        token_start: row.token_start.wrapping_sub(base),
        ..*row
    };
    held.politics == fresh.politics
        && held.party_text == fresh.party_text
        && held.idea_text == fresh.idea_text
        && held.leaders == fresh.leaders
        && held.parties.len() == fresh.parties.len()
        && held
            .parties
            .iter()
            .zip(fresh.parties)
            .all(|(a, b)| party(a, held.party_base) == party(b, fresh.party_base))
        && held.ideas.len() == fresh.ideas.len()
        && held
            .ideas
            .iter()
            .zip(fresh.ideas)
            .all(|(a, b)| idea(a, held.idea_base) == idea(b, fresh.idea_base))
}

/// The latest complete walk's four tables, in the walk's order.
#[derive(Default)]
struct Outputs {
    countries: Vec<Visit>,
    politics: Vec<CountryRow<cpolitical_status::Row>>,
    parties: Vec<PartyRow>,
    /// Each party row's `CPoliticalParty`.
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

    fn rows(&self, visit: &Visit) -> CountryRows<'_> {
        CountryRows {
            politics: &self.politics[visit.politics],
            parties: &self.parties[visit.parties.clone()],
            party_text: &self.party_text[visit.party_text.clone()],
            party_base: visit.party_text.start,
            ideas: &self.ideas[visit.ideas.clone()],
            idea_text: &self.idea_text[visit.idea_text.clone()],
            idea_base: visit.idea_text.start,
            leaders: &self.leaders[visit.leaders.clone()],
        }
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

/// A sampled country's fresh read and the popularity gather.
#[derive(Default)]
struct Scratch {
    politics: CountryRow<cpolitical_status::Row>,
    parties: Vec<PartyRow>,
    party_text: Vec<u8>,
    ideas: Vec<IdeaRow>,
    idea_text: Vec<u8>,
    leaders: Vec<LeaderRow>,
    popularity: Vec<u8>,
}

impl Scratch {
    /// One country read afresh as the walk reads it; `None` when a read
    /// fails or does not complete.
    fn read<M: Memory>(
        &mut self,
        memory: &M,
        base: usize,
        country: &Visit,
    ) -> Option<CountryRows<'_>> {
        let reader = cpolitical_status::Reader::new(memory, base, country.pointer).ok()?;
        let Outcome::Rows { len: leaders } = reader.party_leaders_carried(
            country.id,
            Buffer::Growing(&mut self.leaders),
            &mut Fresh,
        ) else {
            return None;
        };
        self.politics = CountryRow {
            country: country.id,
            row: reader.row().ok()?,
        };
        let parties = reader.parties_carried(
            country.id,
            Buffer::Growing(&mut self.parties),
            0,
            Buffer::Growing(&mut self.party_text),
            0,
            &mut FreshParties,
        );
        let Outcome::Rows { len: party_rows } = parties.outcome else {
            return None;
        };
        let ideas = reader.ideas_carried(
            country.id,
            Buffer::Growing(&mut self.ideas),
            Buffer::Growing(&mut self.idea_text),
            &mut FreshIdeas,
        );
        let Outcome::Rows { len: idea_rows } = ideas.outcome else {
            return None;
        };
        Some(CountryRows {
            politics: &self.politics,
            parties: &self.parties[..party_rows],
            party_text: &self.party_text[..parties.bytes_len],
            party_base: 0,
            ideas: &self.ideas[..idea_rows],
            idea_text: &self.idea_text[..ideas.bytes_len],
            idea_base: 0,
            leaders: &self.leaders[..leaders],
        })
    }

    fn measure(&self) -> usize {
        self.parties.capacity() * size_of::<PartyRow>()
            + self.party_text.capacity()
            + self.ideas.capacity() * size_of::<IdeaRow>()
            + self.idea_text.capacity()
            + self.leaders.capacity() * size_of::<LeaderRow>()
            + self.popularity.capacity()
    }
}

/// What the party store holds for a quiet politics hour: the latest complete
/// walk's tables and keys, and the walk being made.
#[derive(Default)]
pub struct QuietWalk {
    held: Outputs,
    /// The held walk's key; `None` when nothing is held.
    key: Option<Key>,
    /// The popularity token the held rows' popularity was read under.
    popularity: Option<Certified>,
    /// The walk being made: its countries and party pointers so far.
    visits: Vec<Visit>,
    pointers: Vec<usize>,
    /// The current country's first party pointer, and its fresh read's.
    country: usize,
    fresh: Vec<usize>,
    /// Every visit and pointer so far was recorded.
    recorded: bool,
    scratch: Scratch,
    /// The latest walk hour's use of the domains.
    counts: Counts,
}

impl QuietWalk {
    /// A walk begins.
    pub(super) fn begin(&mut self) {
        self.visits.clear();
        self.pointers.clear();
        self.fresh.clear();
        self.country = 0;
        self.recorded = true;
    }

    /// A country's PoliticalParties read is planned.
    pub(super) fn start_country(&mut self) {
        self.country = self.pointers.len();
        self.fresh.clear();
    }

    /// One party of a fresh read, in array order.
    pub(super) fn fresh_party(&mut self, party: usize) {
        self.recorded &= push(&mut self.fresh, party);
    }

    /// The fresh read completed: its parties are the country's.
    pub(super) fn read_parties(&mut self) {
        let stored =
            allocation::advisory(|| allocation::reserve(&mut self.pointers, self.fresh.len()));
        if stored.is_err() {
            self.recorded = false;
            return;
        }
        self.pointers.extend_from_slice(&self.fresh);
    }

    /// The country's parties were carried: the held read's pointers, which
    /// the live array still lists.
    pub(super) fn carried_parties(&mut self, parties: impl ExactSizeIterator<Item = usize>) {
        let stored =
            allocation::advisory(|| allocation::reserve(&mut self.pointers, parties.len()));
        if stored.is_err() {
            self.recorded = false;
            return;
        }
        self.pointers.extend(parties);
    }

    /// A visited country, once its four tables were appended. Its party
    /// pointers must be its PoliticalParties rows, one each.
    pub(super) fn visit(&mut self, visit: Visit) {
        self.recorded &= visit.parties == (self.country..self.pointers.len());
        self.recorded &= push(&mut self.visits, visit);
    }

    pub(super) fn measure(&self) -> usize {
        self.held.measure()
            + self.visits.capacity() * size_of::<Visit>()
            + (self.pointers.capacity() + self.fresh.capacity()) * size_of::<usize>()
            + self.scratch.measure()
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(super) fn counts(&self) -> Counts {
        self.counts
    }

    /// Whether this hour's census lists the held countries in order.
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

    /// Every held party's popularity, gathered afresh in one batched guarded
    /// read and written into the held rows; `false` when the gather failed,
    /// leaving them as they were.
    fn refresh_popularity<M: Memory>(&mut self, memory: &M) -> bool {
        let Self { held, scratch, .. } = self;
        let Some(bytes) = held.pointers.len().checked_mul(8) else {
            return false;
        };
        if held.pointers.len() != held.parties.len() || !grow_vec(&mut scratch.popularity, bytes) {
            return false;
        }
        let gathered = &mut scratch.popularity[..bytes];
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

    /// Reads each held country `check` covers afresh and compares it with
    /// its held rows: how many were compared, as an error at the first
    /// difference or failed read.
    fn sample<M: Memory>(&mut self, memory: &M, base: usize, check: Check) -> Result<u64, u64> {
        if !check.any() {
            return Ok(0);
        }
        let Self { held, scratch, .. } = self;
        let mut compared = 0;
        for country in &held.countries {
            if !check.covers(u64::from(country.id)) {
                continue;
            }
            compared += 1;
            let agrees = scratch
                .read(memory, base, country)
                .is_some_and(|fresh| same(&held.rows(country), &fresh));
            if !agrees {
                return Err(compared);
            }
        }
        Ok(compared)
    }

    /// Writes the held tables into the walk's buffers as a walk of the same
    /// memory appends them; `false` when a buffer cannot grow, before any
    /// outcome is set.
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

    /// Compares each visited country's walked rows with the held walk's:
    /// countries compared and those differing.
    fn compare(&self, tables: &Tables<'_>) -> (u64, u64) {
        let (mut verified, mut mismatches) = (0, 0);
        for (visit, held) in self.visits.iter().zip(&self.held.countries) {
            verified += 1;
            let identical = (visit.slot, visit.pointer, visit.id)
                == (held.slot, held.pointer, held.id)
                && same(&self.held.rows(held), &tables.rows(visit));
            mismatches += u64::from(!identical);
        }
        if self.visits.len() != self.held.countries.len() {
            mismatches += 1;
        }
        (verified, mismatches)
    }

    /// Holds this walk's tables for the next hour under `key`, or nothing.
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
        // The next popularity gather's scratch; a refusal only grows it then.
        let _ = allocation::advisory(|| {
            allocation::resize_default(&mut self.scratch.popularity, lens[1].saturating_mul(8))
        });
        self.key = Some(key);
        self.popularity = popularity;
    }
}

fn grow<T: Default>(values: &mut Vec<T>, len: usize) -> bool {
    Buffer::Growing(values).ensure(len).unwrap_or(false)
}

/// Appends without failing the walk: a full buffer only withholds the next
/// hour's quiet path.
fn push<T>(values: &mut Vec<T>, value: T) -> bool {
    if allocation::advisory(|| allocation::reserve(values, 1)).is_err() {
        return false;
    }
    values.push(value);
    true
}

/// One Governance walk's politics stores and the observations that certify
/// carrying their rows.
pub struct PoliticsWalk<'a> {
    pub leaders: Option<&'a mut LeaderStore>,
    pub parties: Option<&'a mut PartyStore>,
    pub leader_changes: &'a dyn party_leader_changes::Changes,
    pub party_changes: &'a dyn party_changes::Changes,
    pub status: &'a dyn political_status_changes::Changes,
    /// This hour's checks of carried party leaders, of carried parties and
    /// ideas, and of a quiet hour's countries.
    pub leader_check: Check,
    pub party_check: Check,
    pub check: Check,
    pub force_full: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Plan {
    /// Read every country.
    Walk,
    /// Publish the held tables.
    Quiet,
    /// Read every country and compare each with the held tables, which a
    /// quiet hour would have published.
    Compare,
}

/// One walk hour of the politics tables.
pub(super) struct Hour<'a> {
    walk: PoliticsWalk<'a>,
    key: Option<Key>,
    popularity: Option<Certified>,
    /// Politics, PoliticalParties, ActiveIdeas and PartyLeaders are all
    /// walked this hour.
    usual: bool,
    plan: Plan,
    counts: Counts,
}

impl<'a> Hour<'a> {
    /// Before the walk's first read or registration.
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
            plan: Plan::Walk,
            counts: Counts::default(),
        }
    }

    /// Whether the held tables stand for this hour, before any politics
    /// read; quiet only without a `*_verify` feature.
    fn plan<M: Memory>(&self, countries: &ccountry::Reader<'_, M>) -> Plan {
        let walk = &self.walk;
        let (Some(key), Some(_), Some(parties), Some(leaders)) = (
            self.key,
            self.popularity,
            walk.parties.as_deref(),
            walk.leaders.as_deref(),
        ) else {
            return Plan::Walk;
        };
        if !self.usual || walk.force_full {
            return Plan::Walk;
        }
        let (Some(quiet), Some((held, characters))) = (parties.quiet(), leaders.held()) else {
            return Plan::Walk;
        };
        if quiet.key != Some(key) || held != quiet.held.countries.len() {
            return Plan::Walk;
        }
        if !characters.iter().all(|&(character, certified)| {
            walk.leader_changes.character(character) == Some(certified)
        }) {
            return Plan::Walk;
        }
        if !quiet.same_countries(countries) {
            return Plan::Walk;
        }
        if [walk.check, walk.party_check, walk.leader_check].contains(&Check::Every) {
            Plan::Compare
        } else {
            Plan::Quiet
        }
    }

    /// Decides the hour before any politics read. A quiet hour refreshes the
    /// held popularity when its token moved, reads and compares its slice,
    /// and publishes the held tables into `tables`: whether it did.
    pub(super) fn quiet<M: Memory>(
        &mut self,
        memory: &M,
        base: usize,
        countries: &ccountry::Reader<'_, M>,
        tables: &mut Tables<'_>,
        o: &mut Observation,
    ) -> bool {
        let plan = self.plan(countries);
        let (Some(popularity), Some(store), true) = (
            self.popularity,
            self.walk.parties.as_deref_mut(),
            plan != Plan::Walk,
        ) else {
            return false;
        };
        let quiet = store.quiet_mut();
        let refresh = quiet.popularity != Some(popularity);
        if refresh {
            // A failed gather walks every country.
            if !quiet.refresh_popularity(memory) {
                return false;
            }
            quiet.popularity = Some(popularity);
        }
        if plan == Plan::Compare {
            self.plan = Plan::Compare;
            return false;
        }
        match quiet.sample(memory, base, self.walk.check) {
            Ok(compared) => self.counts.verified = compared,
            Err(compared) => {
                // A missed write: nothing certifies a quiet hour any more,
                // and this hour reads every country.
                self.counts.verified = compared;
                self.counts.mismatches = 1;
                self.walk.status.mismatch();
                self.key = None;
                return false;
            }
        }
        if !quiet.publish(countries.count() as usize, tables, o) {
            return false;
        }
        self.plan = Plan::Quiet;
        self.counts.quiet = !refresh;
        self.counts.popularity = refresh;
        true
    }

    /// The party and party-leader stores' walk views, for a walked hour.
    pub(super) fn reuses(&mut self) -> (Option<LeaderReuse<'_>>, Option<PoliticsReuse<'_>>) {
        let walk = &mut self.walk;
        let (leader_changes, leader_check) = (walk.leader_changes, walk.leader_check);
        let (party_changes, party_check) = (walk.party_changes, walk.party_check);
        (
            walk.leaders
                .as_deref_mut()
                .map(|store| LeaderReuse::begin(store, leader_changes, leader_check)),
            walk.parties
                .as_deref_mut()
                .map(|store| PoliticsReuse::begin(store, party_changes, party_check)),
        )
    }

    /// Ends the hour after the stores' walk views finished: a walked hour
    /// compares (verify) and holds its tables; every hour records its counts.
    pub(super) fn finish(mut self, tables: &Tables<'_>, o: &Observation, census: bool) {
        if self.plan != Plan::Quiet {
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
                if self.plan == Plan::Compare && complete && quiet.recorded {
                    let (verified, mismatches) = quiet.compare(tables);
                    self.counts.verified += verified;
                    self.counts.mismatches += mismatches;
                    if mismatches != 0 {
                        self.walk.status.mismatch();
                        self.key = None;
                    }
                }
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

use super::*;
use std::cell::Cell;

fn take(domain: &Domain, table: Table) -> (Option<u64>, Vec<usize>) {
    let mut ids = Vec::new();
    let epoch = domain.take(table, &mut |id| ids.push(id));
    (epoch, ids)
}

fn site(name: &str) -> usize {
    MUTATIONS
        .iter()
        .position(|(site, _, _)| site.name == name)
        .unwrap()
}

fn put(bytes: &mut [u8], at: usize, value: u64) {
    bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
}

fn put_u32(bytes: &mut [u8], at: usize, value: u32) {
    bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
}

/// A country object with native id `id`, at a stable heap address the
/// receivers read through guarded live reads.
fn country(id: u32) -> Box<[u8; 0x1000]> {
    let mut bytes = Box::new([0_u8; 0x1000]);
    put_u32(&mut bytes[..], engine_native::ccountry::ID, id);
    bytes
}

/// Each site marks the country its receiver names in exactly the tables
/// its writes reach; a writer that changes nothing recorded marks nothing.
#[test]
fn a_site_marks_its_country_in_the_tables_it_reaches() {
    let domain = Domain::enabled(64);
    let country = country(5);
    let first = country.as_ptr() as usize;
    let stability = take(&domain, Table::Stability).0;
    let research = take(&domain, Table::Research).0;
    assert!(stability.is_some() && research.is_some());
    drop(domain.notify(site("add_stability"), first, 0));
    assert_eq!(take(&domain, Table::Stability), (stability, vec![5]));
    assert_eq!(take(&domain, Table::Research), (research, vec![]));
    // The tree rebuild reaches both families, not the technology rows.
    let technologies = take(&domain, Table::Technologies).0;
    drop(domain.notify(site("recalculate_modifiers"), first, 0));
    assert_eq!(take(&domain, Table::Stability), (stability, vec![5]));
    assert_eq!(take(&domain, Table::Research), (research, vec![5]));
    assert_eq!(take(&domain, Table::Technologies), (technologies, vec![]));
    // Only the interception column of the air statistics is recorded.
    drop(domain.notify(site("add_air_statistic"), first, 3));
    assert_eq!(take(&domain, Table::Stability), (stability, vec![]));
    drop(domain.notify(site("add_air_statistic"), first, 1));
    assert_eq!(take(&domain, Table::Stability), (stability, vec![5]));
    // A quiet hour takes nothing and keeps the epoch.
    assert_eq!(take(&domain, Table::Stability), (stability, vec![]));
    assert_eq!(
        domain.calls[site("add_air_statistic")].load(Ordering::Relaxed),
        2
    );
}

#[test]
fn design_modifier_marks_have_an_independent_consumer_and_cover_accumulator_reset() {
    let domain = Domain::enabled(5_001);
    let owner = country(5_000);
    let pointer = owner.as_ptr() as usize;
    let epoch = take(&domain, Table::Designs).0;
    let visibility = take(&domain, Table::Visibility).0;
    drop(domain.notify(site("recalculate_modifiers"), pointer, 0));
    assert_eq!(take(&domain, Table::Visibility), (visibility, vec![5_000]));
    assert_eq!(take(&domain, Table::Designs), (epoch, vec![5_000]));
    assert_eq!(take(&domain, Table::Designs), (epoch, vec![]));
    drop(domain.forward(Forward::ResetCountry, pointer, 0));
    assert_eq!(take(&domain, Table::Designs).1, vec![5_000]);
}

/// A receiver that names no country, or a site that writes countries it
/// does not name, moves the epoch of its own tables only.
#[test]
fn a_writer_naming_no_country_moves_the_epoch_of_its_tables_only() {
    let domain = Domain::enabled(64);
    let stability = take(&domain, Table::Stability).0;
    let research = take(&domain, Table::Research).0;
    drop(domain.notify(site("add_war_support"), 0, 0));
    assert_eq!(domain.unnamed.load(Ordering::Relaxed), 1);
    let moved = take(&domain, Table::Stability).0;
    assert!(moved.is_some() && moved != stability);
    assert_eq!(take(&domain, Table::Research).0, research);
    drop(domain.notify(site("become_exiled"), 0, 0));
    assert_ne!(take(&domain, Table::Stability).0, moved);
    drop(domain.notify(site("register_license"), 0, 0));
    assert_ne!(take(&domain, Table::Research).0, research);
    // Every-country sites are not unnamed receivers.
    assert_eq!(domain.unnamed.load(Ordering::Relaxed), 1);
}

/// A level writer marks its country and every member of its faction (whose
/// levels every member's faction-mode sharing reads), covers its call (a
/// take while it runs certifies nothing and keeps the marks) and marks
/// every member again on return. Only its own country's technology rows
/// change.
#[test]
fn a_shared_level_marks_every_country_sharing_research_over_its_call() {
    let domain = Domain::enabled(64);
    let mut own = country(9);
    let member = country(11);
    let mut diplomacy = Box::new([0_u8; 0x400]);
    let mut faction = Box::new([0_u8; 0x100]);
    let members = Box::new([own.as_ptr() as u64, member.as_ptr() as u64]);
    let mut status = Box::new([0_u8; 0x200]);
    put(
        &mut own[..],
        engine_native::ccountry::DIPLOMACY_STATUS,
        diplomacy.as_ptr() as u64,
    );
    put(
        &mut diplomacy[..],
        engine_native::cdiplomacy_status::FACTION,
        faction.as_ptr() as u64,
    );
    put(
        &mut faction[..],
        engine_native::cfaction::MEMBERS,
        members.as_ptr() as u64,
    );
    put_u32(&mut faction[..], engine_native::cfaction::MEMBERS + 8, 2);
    put_u32(&mut faction[..], engine_native::cfaction::MEMBERS + 12, 2);
    put(
        &mut status[..],
        engine_research::ctechnology_status::COUNTRY,
        own.as_ptr() as u64,
    );
    let epoch = take(&domain, Table::Research).0;
    let technologies = take(&domain, Table::Technologies).0;
    let running = domain.notify(site("research_completed"), status.as_ptr() as usize, 0);
    assert_eq!(take(&domain, Table::Research), (None, vec![]));
    assert_eq!(take(&domain, Table::Technologies), (None, vec![]));
    drop(running);
    assert_eq!(take(&domain, Table::Research), (epoch, vec![9, 11]));
    assert_eq!(take(&domain, Table::Technologies), (technologies, vec![9]));
    assert_eq!(take(&domain, Table::Stability).1, Vec::<usize>::new());
    // Without a faction, the country alone.
    put(
        &mut diplomacy[..],
        engine_native::cdiplomacy_status::FACTION,
        0,
    );
    drop(domain.notify(site("research_completed"), status.as_ptr() as usize, 0));
    assert_eq!(take(&domain, Table::Research), (epoch, vec![9]));
    drop((own, member, diplomacy, faction, members, status));
}

/// The daily `research slot synchronization` marks its country only when the slot
/// roster changed, and covers a call whose roster cannot be read first.
#[test]
fn slot_sync_marks_only_a_changed_roster() {
    let domain = Domain::enabled(64);
    let own = country(3);
    let mut status = Box::new([0_u8; 0x200]);
    put(
        &mut status[..],
        engine_research::ctechnology_status::COUNTRY,
        own.as_ptr() as u64,
    );
    let address = status.as_ptr() as usize;
    let epoch = take(&domain, Table::Research).0;
    let roster = Cell::new(Some([1_u8; 16]));
    assert_eq!(domain.sync(address, &|_: usize| roster.get(), || 7), 7);
    assert_eq!(take(&domain, Table::Research), (epoch, vec![]));
    domain.sync(address, &|_: usize| roster.get(), || {
        roster.set(Some([2; 16]));
        0
    });
    assert_eq!(take(&domain, Table::Research), (epoch, vec![3]));
    roster.set(None);
    domain.sync(address, &|_: usize| roster.get(), || 0);
    assert_eq!(take(&domain, Table::Research), (epoch, vec![3]));
    assert_eq!(domain.resynced.load(Ordering::Relaxed), 1);
    assert_eq!(domain.calls[SYNC_RESEARCH_SLOTS].load(Ordering::Relaxed), 3);
    drop((own, status));
}

/// The deferred hourly queue calls `RemoveUnitLeader` for leaders no roster
/// lists: the owner is marked only when a leader roster count changed, and
/// over a call whose counts cannot be read first. Before, every call marked
/// it, about 200 countries' worth of calls an hour.
#[test]
fn a_leader_removal_marks_its_owner_only_when_a_roster_changed() {
    let domain = Domain::enabled(64);
    let own = country(4);
    let mut characters = Box::new([0_u8; 0x100]);
    put(&mut characters[..], 0x8, own.as_ptr() as u64);
    let address = characters.as_ptr() as usize;
    let epoch = take(&domain, Table::Counters).0;
    let counts = Cell::new(Some([1_u8; 16]));
    assert_eq!(
        domain.remove_leader(address, &|_: usize| counts.get(), || 1),
        1
    );
    assert_eq!(take(&domain, Table::Counters), (epoch, vec![]));
    domain.remove_leader(address, &|_: usize| counts.get(), || {
        counts.set(Some([0; 16]));
        1
    });
    assert_eq!(take(&domain, Table::Counters), (epoch, vec![4]));
    counts.set(None);
    domain.remove_leader(address, &|_: usize| counts.get(), || 0);
    assert_eq!(take(&domain, Table::Counters), (epoch, vec![4]));
    assert_eq!(domain.removed_leaders.load(Ordering::Relaxed), 1);
    assert_eq!(domain.calls[REMOVE_UNIT_LEADER].load(Ordering::Relaxed), 3);
    drop((own, characters));
}

/// The daily faction refresh sets every faction's sharing base: its members
/// are marked only when the base changed, and over a call whose base cannot
/// be read first. Before this site no faction member was marked at all.
#[test]
fn faction_sharing_base_marks_the_faction_only_when_it_changed() {
    let domain = Domain::enabled(64);
    let first = country(3);
    let second = country(5);
    let mut faction = Box::new([0_u8; 0x100]);
    let members = Box::new([first.as_ptr() as u64, second.as_ptr() as u64]);
    put(
        &mut faction[..],
        engine_native::cfaction::MEMBERS,
        members.as_ptr() as u64,
    );
    put_u32(&mut faction[..], engine_native::cfaction::MEMBERS + 8, 2);
    put_u32(&mut faction[..], engine_native::cfaction::MEMBERS + 12, 2);
    let mut group = Box::new([0_u8; 0x100]);
    put(&mut group[..], 0x48, faction.as_ptr() as u64);
    let address = group.as_ptr() as usize;
    let epoch = take(&domain, Table::Research).0;
    let certificate = domain.sharing.sequence(address).unwrap();
    let base = Cell::new(Some([1_u8; 16]));
    assert_eq!(domain.rebase(address, &|_: usize| base.get(), || 7), 7);
    assert_eq!(take(&domain, Table::Research), (epoch, vec![]));
    assert_eq!(domain.sharing.sequence(address), Some(certificate));
    domain.rebase(address, &|_: usize| base.get(), || {
        base.set(Some([2; 16]));
        assert!(domain.sharing.quiet().is_none());
        assert!(domain.sharing.sequence(address).is_none());
        0
    });
    assert_eq!(take(&domain, Table::Research), (epoch, vec![3, 5]));
    let changed = domain.sharing.sequence(address).unwrap();
    assert_ne!(changed, certificate);
    assert_eq!(take(&domain, Table::Research), (epoch, vec![]));
    assert_eq!(domain.sharing.sequence(address), Some(changed));
    base.set(None);
    domain.rebase(address, &|_: usize| base.get(), || 0);
    assert_eq!(take(&domain, Table::Research), (epoch, vec![3, 5]));
    assert_eq!(take(&domain, Table::Stability).1, Vec::<usize>::new());
    assert_eq!(domain.rebased.load(Ordering::Relaxed), 1);
    assert_eq!(
        domain.calls[FACTION_SHARING_BASE].load(Ordering::Relaxed),
        3
    );
    drop((first, second, faction, members, group));
}

/// The domain observes only while its own hooks and every forwarding family
/// are enabled: a forwarder whose patches are off would leave its writers
/// unseen.
#[test]
fn the_domain_observes_only_with_its_hooks_and_every_forwarder() {
    let domain = Domain::new();
    for set in &domain.sets {
        set.offer(64);
        set.boundary(false);
    }
    domain.hooks.store(true, Ordering::SeqCst);
    for forwarder in &domain.forwarders[1..] {
        forwarder.store(true, Ordering::SeqCst);
    }
    domain.apply();
    assert_eq!(take(&domain, Table::Stability).0, None);
    domain.forwarders[0].store(true, Ordering::SeqCst);
    domain.apply();
    assert!(take(&domain, Table::Stability).0.is_some());
    domain.hooks.store(false, Ordering::SeqCst);
    domain.apply();
    assert_eq!(take(&domain, Table::Research).0, None);
}

/// Forwarded writers reach the tables their fields feed, and each other
/// family's site index maps to the writer its name says.
#[test]
fn forwarded_sites_reach_their_tables() {
    use engine_mutations::political_status_mutations::MUTATIONS as STATUS;
    let names: Vec<&str> = STATUS.iter().map(|(site, _, _)| site.name).collect();
    assert_eq!(
        names,
        [
            "set_capital",
            "set_politics",
            "force_ruling_party",
            "set_ruling_party",
            "set_ideas",
            "add_popularity",
            "set_popularity_redistributing",
            "set_political_party_popularity",
            "set_popularities",
        ]
    );
    let forwards: Vec<Option<Forward>> = (0..STATUS.len()).map(status_forward).collect();
    assert_eq!(
        forwards,
        [
            None,
            Some(Forward::RulingParty),
            Some(Forward::RulingParty),
            Some(Forward::RulingParty),
            Some(Forward::Ideas),
            Some(Forward::Popularity),
            Some(Forward::Popularity),
            Some(Forward::Popularity),
            Some(Forward::Popularity),
        ]
    );
    // Ideas and the reset reach research too, the reset the technology rows
    // as well; scientists only research.
    assert_eq!(Forward::Ideas.tables(), Tables::BOTH);
    assert_eq!(Forward::ResetCountry.tables(), Tables::ALL);
    assert_eq!(Forward::ScientistTraits.tables(), Tables::RESEARCH);
    assert_eq!(Forward::Popularity.tables(), Tables::STABILITY);
    // The ruling party's first leader reaches the appearance row; the
    // diplomacy reset the appearance and counter rows too.
    for forward in [Forward::RulingParty, Forward::PartyLeaders, Forward::Roles] {
        assert_eq!(
            forward.tables(),
            Tables::STABILITY.with(Tables::APPEARANCE),
            "{forward:?}"
        );
    }
    assert!(Forward::DiplomacyReset.tables().appearance);
    assert!(Forward::DiplomacyReset.tables().counters);
    assert_eq!(Forward::DiplomacyReset.receiver(), Receiver::Every);
    for forward in [Forward::StateOwner, Forward::StateReset] {
        assert_eq!(
            forward.tables(),
            Tables::COUNTERS.with(DRIFTING),
            "{forward:?}"
        );
    }
    for forward in [
        Forward::WartimeAllies,
        Forward::StateController,
        Forward::Cores,
        Forward::RelationReset,
        Forward::RelationSlot,
        Forward::CountryCharacters,
    ] {
        assert_eq!(forward.tables(), Tables::COUNTERS, "{forward:?}");
    }
    assert_eq!(Forward::StateOwner.receiver(), Receiver::StateOwner);
    assert_eq!(Forward::Cores.receiver(), Receiver::TagPointer);
    assert_eq!(Forward::WartimeAllies.receiver(), Receiver::DiplomacyStatus);
    // The territory, raid and advisor pool site indices name the writers
    // they forward.
    let territory: Vec<(&str, Option<Forward>)> = engine_mutations::territory_mutations::MUTATIONS
        .iter()
        .enumerate()
        .filter_map(|(index, (site, _))| {
            territory_forward(index).map(|forward| (site.name, Some(forward)))
        })
        .collect();
    assert_eq!(
        territory,
        [
            ("state_owner", Some(Forward::StateOwner)),
            ("state_controller", Some(Forward::StateController)),
            ("reset_state", Some(Forward::StateReset)),
        ]
    );
    let raid: Vec<&str> = engine_mutations::raid_mutations::MUTATIONS
        .iter()
        .enumerate()
        .filter(|(index, _)| raid_forward(*index) == Some(Forward::Cores))
        .map(|(_, (site, _))| site.name)
        .collect();
    assert_eq!(raid, ["add_core", "remove_core"]);
    let pool: Vec<&str> = engine_mutations::advisor_pool_mutations::MUTATIONS
        .iter()
        .enumerate()
        .filter(|(index, _)| pool_forward(*index) == Some(Forward::CountryCharacters))
        .map(|(_, (site, _))| site.name)
        .collect();
    assert_eq!(pool, ["construct_pool", "destroy_pool"]);
    use engine_mutations::character_role_mutations as roles;
    for site in [
        roles::COUNTRY_LEADER_MODIFIERS,
        roles::COUNTRY_LEADER_DESTROY,
        roles::COUNTRY_LEADER_MAP_INSERT,
        roles::COUNTRY_LEADER_CREATE,
        roles::ADVISOR_MODIFIERS,
        roles::COUNTRY_LEADER_IDEOLOGY,
    ] {
        assert_eq!(role_forward(site), Forward::Roles, "{site}");
    }
    // A scientist source's name and id are research inputs.
    assert_eq!(
        role_forward(roles::CHARACTER_NAME),
        Forward::CharacterIdentity
    );
    assert_eq!(
        role_forward(roles::CHARACTER_REPAIR),
        Forward::CharacterIdentity
    );
    assert_eq!(Forward::CharacterIdentity.tables(), Tables::RESEARCH);
    assert_eq!(Forward::CharacterIdentity.receiver(), Receiver::Character);
    use engine_mutations::character_roster_mutations as roster;
    assert_eq!(roster_forward(roster::PUSH, 1, 2), None);
    assert_eq!(roster_forward(roster::CLEAR, 1, 2), None);
    assert_eq!(
        roster_forward(roster::SCIENTIST_BUILD, 1, 2),
        Some((Forward::ScientistRoles, 2))
    );
    assert_eq!(
        roster_forward(roster::SCIENTIST_CLEAR, 1, 2),
        Some((Forward::ScientistRoles, 1))
    );
    assert_eq!(
        roster_forward(roster::SCIENTIST_TRAIT_RESET, 1, 2),
        Some((Forward::ScientistTraits, 1))
    );
}

/// A forwarded writer marks like a hooked one: a reset country in both
/// tables, a reset diplomacy every country's stability.
#[test]
fn a_forwarded_writer_marks_its_country() {
    let domain = Domain::enabled(64);
    let country = country(6);
    let stability = take(&domain, Table::Stability).0;
    let research = take(&domain, Table::Research).0;
    drop(domain.forward(Forward::ResetCountry, country.as_ptr() as usize, 0));
    assert_eq!(take(&domain, Table::Stability), (stability, vec![6]));
    assert_eq!(take(&domain, Table::Research), (research, vec![6]));
    drop(domain.forward(Forward::DiplomacyReset, country.as_ptr() as usize, 0));
    assert_ne!(take(&domain, Table::Stability).0, stability);
    assert_eq!(take(&domain, Table::Research).0, research);
    assert_eq!(
        domain.forwarded[Forward::ResetCountry as usize].load(Ordering::Relaxed),
        1
    );
}

/// Every forwarded country reset and the console's grow_intel_network (on
/// entry and on return) move the intel network count, other forwards do
/// not, and nothing is counted while the politics family that forwards the
/// reset is not patched: the networks are read again at the next hour.
#[test]
fn intel_network_writes_count_while_politics_forwards_the_reset() {
    let domain = Domain::enabled(64);
    let country = country(5);
    let first = country.as_ptr() as usize;
    assert_eq!(domain.intel_network_writes(), Some(0));
    assert_eq!(domain.intel_network_resets(), Some(0));
    drop(domain.forward(Forward::ResetCountry, first, 0));
    drop(domain.forward(Forward::Ideas, 0, 0));
    assert_eq!(domain.intel_network_writes(), Some(1));
    assert_eq!(domain.intel_network_resets(), Some(1));
    let running = std::cell::Cell::new(None);
    assert_eq!(
        domain.grow_intel_network(|| {
            running.set(domain.intel_network_writes());
            7
        }),
        7
    );
    assert_eq!(running.get(), Some(2));
    assert_eq!(domain.intel_network_writes(), Some(3));
    // Growth retains national payloads; lifetime invalidation reuses the
    // already-counted resets and adds no callback counter.
    assert_eq!(domain.intel_network_resets(), Some(1));
    domain.forwarders[Forwarder::Politics as usize].store(false, Ordering::SeqCst);
    assert_eq!(domain.intel_network_writes(), None);
    assert_eq!(domain.intel_network_resets(), None);
}

/// A write of the appearance, counter or visibility inputs marks only its own set, and
/// each new site marks only the table it writes: the cosmetic tag the
/// appearance row, a division the counter row, the modifier rebuild the
/// counter row and visibility input beside stability and research.
#[test]
fn appearance_counter_and_visibility_writers_mark_only_their_set() {
    let domain = Domain::enabled(64);
    let country = country(4);
    let first = country.as_ptr() as usize;
    let tables = [
        Table::Stability,
        Table::Research,
        Table::Technologies,
        Table::Focus,
        Table::Projects,
        Table::Appearance,
        Table::Counters,
        Table::Visibility,
        Table::UnitSpeeds,
    ];
    let epochs: Vec<Option<u64>> = tables.iter().map(|&table| take(&domain, table).0).collect();
    assert!(epochs.iter().all(Option::is_some));
    let marked = |expected: &[Table]| {
        for (&table, &epoch) in tables.iter().zip(&epochs) {
            let ids = if expected.contains(&table) {
                vec![4]
            } else {
                vec![]
            };
            assert_eq!(take(&domain, table), (epoch, ids), "{table:?}");
        }
    };
    domain.write(Table::Appearance, 4);
    marked(&[Table::Appearance]);
    domain.write(Table::Counters, 4);
    marked(&[Table::Counters]);
    for name in [
        "set_cosmetic_tag",
        "copy_country_colors",
        "resolve_graphical_culture",
        "resolve_graphical_culture_2d",
        "rebuild_localized_name",
    ] {
        drop(domain.notify(site(name), first, 0));
        marked(&[Table::Appearance]);
    }
    domain.write(Table::UnitSpeeds, 4);
    marked(&[Table::UnitSpeeds]);
    // Every counter writer marks the unit speed set too.
    for name in ["add_army", "remove_army", "refresh_neighbors"] {
        drop(domain.notify(site(name), first, 0));
        marked(&[Table::Counters, Table::UnitSpeeds]);
    }
    drop(domain.notify(site("recalculate_modifiers"), first, 0));
    marked(&[
        Table::Stability,
        Table::Research,
        Table::Counters,
        Table::Visibility,
        Table::UnitSpeeds,
    ]);
    drop(domain.forward(Forward::ResetCountry, first, 0));
    marked(&tables);
    // The MIO policy reload names no country: the counter and unit speed
    // sets' epochs move alone.
    drop(domain.notify(site("reload_mio_policies"), 0, 0));
    for (&table, &epoch) in tables.iter().zip(&epochs) {
        let moved = take(&domain, table).0 != epoch;
        assert_eq!(
            moved,
            matches!(table, Table::Counters | Table::UnitSpeeds),
            "{table:?}"
        );
    }
    domain.write_every(Table::Appearance);
    assert_ne!(take(&domain, Table::Appearance).0, epochs[5]);
    assert_eq!(take(&domain, Table::Counters).1, Vec::<usize>::new());
    drop(country);
}

/// The subject rebuild names its overlord again on return: one it set
/// there that cannot be named moves the appearance epoch, while a rebuild
/// that leaves no overlord marks its country alone.
#[test]
fn the_subject_rebuild_names_its_new_overlord_on_return() {
    let domain = Domain::enabled(64);
    let country = country(8);
    let mut diplomacy = Box::new([0_u8; 0x400]);
    put(
        &mut diplomacy[..],
        engine_native::cdiplomacy_status::COUNTRY,
        country.as_ptr() as u64,
    );
    let receiver = diplomacy.as_ptr() as usize;
    let epoch = take(&domain, Table::Appearance).0;
    drop(domain.notify(site("rebuild_subjects"), receiver, 0));
    assert_eq!(take(&domain, Table::Appearance), (epoch, vec![8]));
    let running = domain.notify(site("rebuild_subjects"), receiver, 0);
    assert_eq!(take(&domain, Table::Appearance), (None, vec![]));
    // The rebuild links an overlord; no game state resolves it here.
    put_u32(
        &mut diplomacy[..],
        engine_native::cdiplomacy_status::OVERLORD,
        7,
    );
    drop(running);
    let (moved, ids) = take(&domain, Table::Appearance);
    assert!(moved.is_some() && moved != epoch);
    assert_eq!(ids, vec![8]);
    assert_eq!(take(&domain, Table::Counters).1, Vec::<usize>::new());
    drop((country, diplomacy));
}

/// The territory, raid, opinion, relation and advisor pool forwards reach
/// the counter set alone: a state given to nobody names no country, a
/// positive tag that does not resolve and the pool constructor move the
/// counter epoch, and a relation status names its owner.
#[test]
fn counter_forwards_reach_the_counter_set() {
    let domain = Domain::enabled(64);
    let owner = country(3);
    let mut diplomacy = Box::new([0_u8; 0x400]);
    put(
        &mut diplomacy[..],
        engine_native::cdiplomacy_status::COUNTRY,
        owner.as_ptr() as u64,
    );
    let mut status = Box::new([0_u8; 0x400]);
    put(
        &mut status[..],
        engine_native::crelation_status::DIPLOMACY_STATUS,
        diplomacy.as_ptr() as u64,
    );
    let state = Box::new([0_u8; 0x200]);
    let mut tag = Box::new(0_u32);
    let stability = take(&domain, Table::Stability).0;
    let epoch = take(&domain, Table::Counters).0;
    drop(domain.forward(Forward::RelationSlot, status.as_ptr() as usize, 0));
    assert_eq!(take(&domain, Table::Counters), (epoch, vec![3]));
    drop(domain.forward(Forward::RelationReset, status.as_ptr() as usize, 0));
    drop(domain.forward(Forward::WartimeAllies, diplomacy.as_ptr() as usize, 0));
    assert_eq!(take(&domain, Table::Counters), (epoch, vec![3]));
    // An unowned state given to nobody, a nonpositive core tag.
    drop(domain.forward(Forward::StateOwner, state.as_ptr() as usize, 0));
    drop(domain.forward(
        Forward::Cores,
        state.as_ptr() as usize,
        &*tag as *const u32 as usize,
    ));
    assert_eq!(take(&domain, Table::Counters), (epoch, vec![]));
    // A positive link that names no country.
    *tag = 5;
    drop(domain.forward(
        Forward::Cores,
        state.as_ptr() as usize,
        &*tag as *const u32 as usize,
    ));
    let moved = take(&domain, Table::Counters).0;
    assert!(moved.is_some() && moved != epoch);
    drop(domain.forward(Forward::CountryCharacters, 0, 0));
    assert_ne!(take(&domain, Table::Counters).0, moved);
    assert_eq!(take(&domain, Table::Stability), (stability, vec![]));
    assert_eq!(
        domain.forwarded[Forward::Cores as usize].load(Ordering::Relaxed),
        2
    );
    drop((owner, diplomacy, status, state, tag));
}

/// A set waits only for the families forwarding its writers: without the
/// raid family's patches the counter set observes nothing, while the
/// stability and appearance sets still do; without the political status
/// family's, the appearance set does not.
#[test]
fn a_set_observes_with_the_forwarders_of_its_writers() {
    let domain = Domain::enabled(64);
    domain.forwarders[Forwarder::Raid as usize].store(false, Ordering::SeqCst);
    domain.apply();
    assert_eq!(take(&domain, Table::Counters).0, None);
    assert!(take(&domain, Table::Stability).0.is_some());
    assert!(take(&domain, Table::Appearance).0.is_some());
    domain.forwarders[Forwarder::Raid as usize].store(true, Ordering::SeqCst);
    domain.forwarders[Forwarder::PoliticalStatus as usize].store(false, Ordering::SeqCst);
    domain.apply();
    assert_eq!(take(&domain, Table::Appearance).0, None);
    assert_eq!(take(&domain, Table::Stability).0, None);
    assert!(take(&domain, Table::Counters).0.is_some());
    // The territory family's state owner and reset also reach the drifting
    // families, which observe only while it is enabled.
    let territory: Vec<usize> = (0..SETS)
        .filter(|&set| Forwarder::Territory.reaches(set))
        .collect();
    assert_eq!(
        territory,
        [
            Table::Research as usize,
            Table::Technologies as usize,
            Table::Focus as usize,
            Table::Projects as usize,
            Table::Counters as usize,
            Table::UnitSpeeds as usize,
        ]
    );
    for forwarder in [Forwarder::Raid, Forwarder::Opinions, Forwarder::AdvisorPool] {
        let sets: Vec<usize> = (0..SETS).filter(|&set| forwarder.reaches(set)).collect();
        assert_eq!(
            sets,
            [Table::Counters as usize, Table::UnitSpeeds as usize],
            "{forwarder:?}"
        );
    }
}

/// Every province weather writer outside the native window moves the
/// weather count on entry and again on return, so a read overlapping one is
/// not taken as an ordinary hour; no country set is marked.
#[test]
fn weather_writers_move_the_weather_count_on_entry_and_return() {
    let domain = Domain::enabled(64);
    let research = take(&domain, Table::Research).0;
    for site in sites::WEATHER_WRITERS {
        let before = domain.weather_writes.load(Ordering::SeqCst);
        let during = domain.weather_write(site, || {
            domain.weather_writes.load(Ordering::SeqCst) as usize
        });
        assert_eq!(during as u64, before + 1);
        assert_eq!(domain.weather_writes.load(Ordering::SeqCst), before + 2);
        assert_eq!(domain.calls[site].load(Ordering::Relaxed), 1);
    }
    assert_eq!(take(&domain, Table::Research), (research, vec![]));
    assert_eq!(
        domain.weather_writes().is_some(),
        cfg!(feature = "weather_changes")
    );
}

/// A state owner change reaches the drifting families of both owners: a
/// country that owned a state only across a daily tick, between two of
/// their drift checks, had its research, focus and project steps run
/// unseen. Here the old owner's positive tag names no country, so every
/// country of each family is read. Before, only the counter rows were.
#[test]
fn a_state_owner_change_reaches_the_drifting_families() {
    let domain = Domain::enabled(64);
    let mut state = Box::new([0_u8; 0x200]);
    put_u32(&mut state[..], engine_native::cstate::OWNER, 5);
    let tables = [
        Table::Technologies,
        Table::Focus,
        Table::Projects,
        Table::Counters,
    ];
    let epochs = tables.map(|table| take(&domain, table).0);
    let stability = take(&domain, Table::Stability).0;
    drop(domain.forward(Forward::StateOwner, state.as_ptr() as usize, 0));
    for (table, epoch) in tables.into_iter().zip(epochs) {
        let moved = take(&domain, table).0;
        assert!(moved.is_some() && moved != epoch, "{table:?}");
    }
    assert_eq!(take(&domain, Table::Stability), (stability, vec![]));
    drop(state);
}

/// An effect's progress (source 2) marks its technology's country in the
/// technology rows, which a carried slice reads only the assigned
/// technologies' progress of; the daily slot step's (source 0) and the
/// assignment's (source 1) mark nothing. Before, no progress adder was
/// observed.
#[test]
fn only_an_effects_progress_marks_its_country() {
    let domain = Domain::enabled(64);
    let owner = country(7);
    let mut status = Box::new([0_u8; 0x200]);
    put(
        &mut status[..],
        engine_research::ctechnology_status::COUNTRY,
        owner.as_ptr() as u64,
    );
    let mut technology = Box::new([0_u8; 0x200]);
    put(
        &mut technology[..],
        engine_research::ctechnology::STATUS,
        status.as_ptr() as u64,
    );
    let first = technology.as_ptr() as usize;
    let epoch = take(&domain, Table::Technologies).0;
    for source in [0, 1] {
        assert_eq!(domain.research_progress(first, source, || 17), 17);
    }
    assert_eq!(take(&domain, Table::Technologies), (epoch, vec![]));
    domain.research_progress(first, 2, || 0);
    assert_eq!(take(&domain, Table::Technologies), (epoch, vec![7]));
    assert_eq!(
        domain.calls[sites::RESEARCH_PROGRESS].load(Ordering::Relaxed),
        1
    );
    drop((owner, status, technology));
}

#[test]
fn visibility_waits_only_for_the_country_reset_forwarder_and_poisoning_is_local() {
    let domain = Domain::enabled(64);
    let forwarders: Vec<_> = Forwarder::ALL
        .into_iter()
        .filter(|forwarder| forwarder.reaches(Table::Visibility as usize))
        .collect();
    assert_eq!(forwarders, [Forwarder::Politics]);
    domain.forwarders[Forwarder::PoliticalStatus as usize].store(false, Ordering::SeqCst);
    domain.apply();
    assert!(take(&domain, Table::Visibility).0.is_some());
    domain.forwarders[Forwarder::Politics as usize].store(false, Ordering::SeqCst);
    domain.apply();
    assert_eq!(take(&domain, Table::Visibility).0, None);
    domain.forwarders[Forwarder::Politics as usize].store(true, Ordering::SeqCst);
    domain.apply();
    assert!(take(&domain, Table::Visibility).0.is_some());
    domain.mismatch(
        Table::Visibility,
        Mismatch {
            country: 4,
            field: "unit modifier raw presence/value",
        },
    );
    assert!(domain.poisoned(Table::Visibility));
    assert!(!domain.poisoned(Table::Counters));
    assert_eq!(take(&domain, Table::Visibility).0, None);
}

#[test]
fn occupation_exile_writers_mark_only_diplomacy_and_preserve_source_certificates() {
    let watches = crate::watch_registry::WatchRegistry::enabled(64);
    let diplomacy = 101;
    let unrelated_diplomacy = 202;
    let country_source = 303;
    let relation_source = 404;
    let state_source = 505;
    let unaffected = [
        unrelated_diplomacy,
        country_source,
        relation_source,
        state_source,
    ];
    let held = unaffected.map(|receiver| watches.sequence(receiver).unwrap());
    let epoch = watches.quiet().unwrap().0;
    for receiver in [
        occupation_exile_receiver(10, diplomacy),
        occupation_exile_receiver(11, diplomacy),
        occupation_exile_receiver(12, diplomacy),
        occupation_exile_receiver(sites::ADD_EXILE_LEGITIMACY, diplomacy),
        occupation_exile_receiver(sites::SET_EXILE_LEGITIMACY, diplomacy),
        occupation_exile_receiver(sites::EXILE_EQUIPMENT_LEGITIMACY, diplomacy),
        occupation_forward_receiver(Forward::DiplomacyReset, diplomacy),
    ] {
        let before = watches.sequence(diplomacy).unwrap();
        let receiver = receiver.expect("exile writer must identify its diplomacy");
        assert_eq!(receiver, diplomacy);
        drop(watches.notify(receiver));
        assert_ne!(watches.sequence(diplomacy).unwrap(), before);
        assert_eq!(watches.quiet().unwrap().0, epoch);
        assert_eq!(
            unaffected.map(|receiver| watches.sequence(receiver).unwrap()),
            held
        );
    }
    for index in [
        sites::ADD_EXILE_LEGITIMACY,
        sites::SET_EXILE_LEGITIMACY,
        sites::EXILE_EQUIPMENT_LEGITIMACY,
    ] {
        let (site, receiver, _) = &sites::MUTATIONS[index];
        assert!(matches!(
            receiver,
            engine_mutations::country_input_mutations::Receiver::DiplomacyStatus
        ));
        assert!(
            engine_occupation::cresistance::changes::SITES
                .iter()
                .all(|owned| owned.rva != site.rva)
        );
    }
    assert_eq!(
        occupation_exile_receiver(sites::ADD_EXILE_MANPOWER, diplomacy),
        None
    );
    assert_eq!(occupation_exile_receiver(9, diplomacy), None);
    assert_eq!(occupation_exile_receiver(13, diplomacy), None);
    assert_eq!(
        occupation_forward_receiver(Forward::RulingParty, diplomacy),
        None
    );
}

#[test]
fn faction_accumulator_certificates_cover_recalculation_and_reset_without_consuming_visibility() {
    let domain = Domain::enabled(64);
    let country = country(5);
    let pointer = country.as_ptr() as usize;
    let before = domain.accumulator.sequence(pointer).unwrap();
    let marks = domain.notify(site("recalculate_modifiers"), pointer, 0);
    assert!(domain.accumulator.sequence(pointer).is_none());
    drop(marks);
    let changed = domain.accumulator.sequence(pointer).unwrap();
    assert_ne!(before, changed);
    let marks = domain.forward(Forward::ResetCountry, pointer, 0);
    assert!(domain.accumulator.quiet().is_none());
    drop(marks);
    assert_ne!(domain.accumulator.sequence(pointer), Some(changed));
    let _ = domain.accumulator.sequence(pointer);
    assert_eq!(take(&domain, Table::Visibility).1, vec![5]);
    domain.forwarders[Forwarder::Politics as usize].store(false, Ordering::SeqCst);
    domain.apply();
    assert!(domain.accumulator.quiet().is_none());
}

#[test]
fn research_mismatch_keeps_numeric_diagnostics_in_first_last_and_status() {
    let domain = Domain::enabled(64);
    let detail = ResearchMismatch {
        previous_counts: Some([4, 2, 10]),
        current_counts: [4, 2, 11],
        previous_modifier_counts: Some([0; RESEARCH_MODIFIER_KINDS]),
        current_modifier_counts: std::array::from_fn(|kind| u64::from(kind == 31)),
        modifier_identity_differences: (1 << 25) | (1 << 31),
        modifier_value_differences: (1 << 30) | (1 << 31),
        take_available: true,
        certified: true,
        force_full: false,
        selection_force_full: true,
    };
    collector_test_support::with_allocations_denied(|| {
        domain.research_mismatch(
            Mismatch {
                country: 40,
                field: "ResearchModifiers row count",
            },
            detail,
        );
    });
    let held = domain.mismatches(Table::Research);
    assert_eq!(held.first.unwrap().research, Some(detail));
    assert_eq!(held.last, held.first);
    let json = serde_json::to_value(held.first.unwrap()).unwrap();
    assert_eq!(json["research"]["current_modifier_counts"][31], 1);
    assert_eq!(
        json["research"]["previous_counts"],
        serde_json::json!([4, 2, 10])
    );
    assert_eq!(
        json["research"]["modifier_value_differences"],
        (1_u32 << 30) | (1_u32 << 31)
    );
    assert!(domain.poisoned(Table::Research));
}

#[test]
fn startup_country_offer_keeps_the_seed_epoch_and_observes_writes_at_scale() {
    let domain = Domain {
        every: [false; SETS],
        ..Domain::new()
    };
    domain.hooks.store(true, Ordering::SeqCst);
    for forwarder in &domain.forwarders {
        forwarder.store(true, Ordering::SeqCst);
    }
    domain.apply();
    assert_eq!(take(&domain, Table::Technologies).0, None);
    domain.offer_countries(1_501);
    domain.boundary(true);
    for table in [
        Table::Technologies,
        Table::Projects,
        Table::Research,
        Table::Focus,
    ] {
        use collector_certificates::slicing::{
            Changed, SliceFingerprints, SliceSelection, select_changed,
        };
        let first = take(&domain, table).0.expect("seed epoch");
        let mut store = SliceFingerprints::new();
        let mut selection = SliceSelection::new(false);
        let mut changed = Changed::default();
        selection.reset(10, true);
        select_changed(
            &mut selection,
            &mut store,
            &mut changed,
            |visit| domain.take(table, visit),
            0,
            false,
        );
        assert!(!changed.certified && selection.force_full);
        domain.boundary(false);
        domain.write(table, 1_500);
        selection.reset(11, false);
        select_changed(
            &mut selection,
            &mut store,
            &mut changed,
            |visit| domain.take(table, visit),
            0,
            false,
        );
        assert!(changed.certified && !selection.force_full);
        assert_eq!(changed.marked(), 1);
        assert!(selection.contains(1_500));
        assert_eq!(take(&domain, table), (Some(first), vec![]));
        domain.offer_countries(3_001);
        domain.boundary(false);
        assert_eq!(take(&domain, table), (Some(first), vec![]));
        domain.write(table, 9_000);
        assert_ne!(take(&domain, table).0, Some(first));
    }
}

#[test]
fn denied_startup_country_offer_stays_uncertified_and_retries_without_stopping() {
    const CHILD: &str = "BATTLE_PLAN_COUNTRY_BITMAP_OFFER_TEST";
    if std::env::var_os(CHILD).is_none() {
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "country_input_changes::tests::denied_startup_country_offer_stays_uncertified_and_retries_without_stopping", "--nocapture"])
            .env(CHILD, "1")
            .status()
            .unwrap();
        assert!(status.success());
        return;
    }
    static TERMINAL: AtomicBool = AtomicBool::new(false);
    fn failed() {
        TERMINAL.store(true, Ordering::SeqCst);
    }
    fn stopped() -> bool {
        TERMINAL.load(Ordering::SeqCst)
    }
    let domain = Domain {
        every: [false; SETS],
        ..Domain::new()
    };
    domain.hooks.store(true, Ordering::SeqCst);
    for forwarder in &domain.forwarders {
        forwarder.store(true, Ordering::SeqCst);
    }
    domain.apply();
    engine_allocation::set_hooks(None, Some(failed));
    engine_allocation::set_stop_hook(Some(stopped));
    let denied = crate::test_support::NullGuard::new(1);
    domain.offer_countries(1_501);
    drop(denied);
    assert!(crate::test_support::DENIALS.load(Ordering::Relaxed) > 0);
    assert!(!stopped() && !engine_allocation::capture_stopped());
    domain.boundary(true);
    assert_eq!(take(&domain, Table::Technologies).0, None);
    domain.offer_countries(1_501);
    domain.boundary(false);
    assert!(take(&domain, Table::Technologies).0.is_some());
    engine_allocation::set_hooks(None, None);
    engine_allocation::set_stop_hook(None);
}

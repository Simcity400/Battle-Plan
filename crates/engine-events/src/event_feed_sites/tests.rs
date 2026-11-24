use super::*;
use std::collections::BTreeMap;

const BASE: usize = 0x140000000;
const STATE: usize = 0x1000;
const ACTOR: usize = 0x20000;
const TARGET: usize = 0x30000;
const RESERVED: usize = 0x10000;
const SELECTED: usize = 0x23000;
const RECIPROCAL: usize = 0x33000;
const CANDIDATE: usize = 0x50000;
const STACK: usize = 0x70000;
const SCOPE: usize = 0x80000;
const FROM: usize = 0x81000;

#[derive(Default)]
struct Image(BTreeMap<usize, u8>);

impl Image {
    fn bytes(&mut self, address: usize, bytes: &[u8]) {
        for (offset, byte) in bytes.iter().enumerate() {
            self.0.insert(address + offset, *byte);
        }
    }

    fn q(&mut self, address: usize, value: usize) {
        self.bytes(address, &(value as u64).to_le_bytes());
    }

    fn d(&mut self, address: usize, value: u32) {
        self.bytes(address, &value.to_le_bytes());
    }

    fn array(&mut self, address: usize, data: usize, count: u32) {
        self.q(address, data);
        self.d(address + 8, count);
        self.d(address + 12, count);
    }

    fn country(&mut self, country: usize, id: u32, instance: u32, diplomacy: usize, matrix: usize) {
        self.q(country, BASE + ccountry::VTABLE_RVA.0);
        self.d(country + ccountry::ID, id);
        self.d(country + ccountry::SURRENDER_TARGET, instance);
        self.q(country + ccountry::DIPLOMACY_STATUS, diplomacy);
        self.q(diplomacy, BASE + engine_native_parts::cdiplomacy_status_layout::VTABLE_RVA.0);
        self.q(diplomacy + engine_native_parts::cdiplomacy_status_layout::COUNTRY, country);
        self.array(diplomacy + engine_native_parts::cdiplomacy_status_layout::RELATIONS, matrix, 4);
    }

    fn status(&mut self, status: usize, diplomacy: usize, target: u32) {
        self.q(status, BASE + crelation_status::VTABLE_RVA.0);
        self.q(status + crelation_status::DIPLOMACY_STATUS, diplomacy);
        self.d(status + crelation_status::TARGET_COUNTRY, target);
        self.q(status + crelation_status::WAR_RELATION, 0);
    }

    fn new() -> Self {
        let mut image = Self::default();
        image.q(BASE + cgame_state::SINGLETON_RVA.0, STATE);
        image.d(STATE + cgame_state::CURRENT_HOUR, 60828400);
        image.array(STATE + cgame_state::COUNTRIES, 0x4000, 4);
        image.array(STATE + cgame_state::COUNTRY_LINKS, 0x5000, 4);
        for (slot, country) in [RESERVED, ACTOR, TARGET, TARGET].into_iter().enumerate() {
            image.q(0x4000 + slot * 8, country);
        }
        for (raw, canonical) in [0, 1, 2, 2].into_iter().enumerate() {
            image.d(0x5000 + raw * 4, canonical);
        }
        image.country(RESERVED, 0, 0xaaaa, 0x11000, 0x12000);
        image.country(ACTOR, 1, 0xbbbb, 0x21000, 0x22000);
        image.country(TARGET, 2, 0xcccc, 0x31000, 0x32000);
        image.q(0x22000 + 2 * 8, SELECTED);
        image.q(0x32000 + 8, RECIPROCAL);
        image.status(SELECTED, 0x21000, 2);
        image.status(RECIPROCAL, 0x31000, 1);
        image.q(CANDIDATE, BASE + cwar_relation::VTABLE_RVA.0);
        image.d(CANDIDATE + crelation::TYPE_ID, cwar_relation::TYPE);
        image.d(CANDIDATE + crelation::FIRST_COUNTRY, 1);
        image.d(CANDIDATE + crelation::SECOND_COUNTRY, 2);
        image.bytes(CANDIDATE + crelation::CANCEL, &[0, 0]);
        image.q(STACK + 0x1d8, BASE + 0x00d8_1bff);
        image.q(STACK + 0x448, BASE + 0x00d8_167d);
        image.q(STACK + 0x438, SELECTED);
        image.q(STACK + 0x4a8, BASE + 0x00d9_8a1e);
        image.q(SCOPE, BASE + cevent_scope::VTABLE_RVA.0);
        image.q(FROM, BASE + cevent_scope::VTABLE_RVA.0);
        image.q(SCOPE + cevent_scope::FROM, FROM);
        image.d(SCOPE + cevent_scope::COUNTRY_TAG, 1);
        image.d(FROM + cevent_scope::COUNTRY_TAG, 2);
        image
    }

    fn attach(&mut self) {
        self.q(SELECTED + crelation_status::WAR_RELATION, CANDIDATE);
        self.q(RECIPROCAL + crelation_status::WAR_RELATION, CANDIDATE);
    }
}

impl Memory for Image {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        for (offset, destination) in output.iter_mut().enumerate() {
            let Some(byte) = address
                .checked_add(offset)
                .and_then(|address| self.0.get(&address))
            else {
                return false;
            };
            *destination = *byte;
        }
        true
    }
}

fn frame() -> WarFrame {
    WarFrame {
        rsp_before_call: STACK,
        rcx: SELECTED,
        rdx: CANDIDATE,
        rdi: CANDIDATE,
    }
}

fn annex_frame() -> AnnexFrame {
    AnnexFrame {
        rbp: SCOPE - 0xa0,
        r8: SCOPE,
        r9: 0,
    }
}

#[test]
fn three_authored_sources_use_actual_creator_actor_without_role_inference() {
    for (return_rva, expected) in [
        (0x00d9_8a1e, Source::Declaration),
        (0x00d9_ed0a, Source::Join),
        (0x0116_8728, Source::CivilWarWarTransfer),
    ] {
        let mut image = Image::new();
        image.q(STACK + 0x4a8, BASE + return_rva);
        let pending = war_before(&image, BASE, frame()).unwrap();
        image.attach();
        image.d(STATE + cgame_state::CURRENT_HOUR, 60828520);
        let event = war_returned(&image, BASE, pending).unwrap();
        assert_eq!(event.source, expected);
        assert_eq!(event.hour, 60828400);
        assert_eq!(
            event.actor,
            CountryIdentity {
                reference: 1,
                id: 1,
                instance: 0xbbbb
            }
        );
        assert_eq!(
            event.target,
            CountryIdentity {
                reference: 2,
                id: 2,
                instance: 0xcccc
            }
        );
    }
}

#[test]
fn no_pre_null_rule_but_a_preinstalled_same_candidate_refuses_source() {
    let mut image = Image::new();
    image.q(SELECTED + crelation_status::WAR_RELATION, 0xdead);
    let pending = war_before(&image, BASE, frame()).unwrap();
    image.attach();
    assert!(war_returned(&image, BASE, pending).is_ok());
    assert_eq!(
        war_before(&image, BASE, frame()).err(),
        Some("event feed candidate already installed before second link")
    );
}

#[test]
fn same_hour_pair_repeated_operations_have_no_pointer_or_pair_deduplication() {
    let mut image = Image::new();
    let mut events = Vec::new();
    for _ in 0..2 {
        image.q(SELECTED + crelation_status::WAR_RELATION, 0);
        let pending = war_before(&image, BASE, frame()).unwrap();
        image.attach();
        events.push(war_returned(&image, BASE, pending).unwrap());
    }
    assert_eq!(events.len(), 2);
    assert_eq!(events[0], events[1]);
}

#[test]
fn malformed_source_frame_or_instruction_never_guesses_a_cause() {
    for offset in [0x1d8, 0x448, 0x438] {
        let mut image = Image::new();
        image.q(STACK + offset, 0);
        assert_eq!(
            war_before(&image, BASE, frame()).err(),
            Some("event feed war creator frame mismatch")
        );
    }
    let mut image = Image::new();
    image.q(STACK + 0x4a8, BASE + 0x1234);
    assert_eq!(
        war_before(&image, BASE, frame()).err(),
        Some("event feed war producer is unclassified")
    );
    assert!(
        war_before(
            &Image::new(),
            BASE,
            WarFrame {
                rsp_before_call: STACK - 8,
                ..frame()
            }
        )
        .is_err()
    );
}

#[test]
fn aliases_preserve_source_words_but_postproof_requires_the_exact_candidate_pair() {
    let mut image = Image::new();
    image.d(SELECTED + crelation_status::TARGET_COUNTRY, 3);
    image.d(CANDIDATE + crelation::SECOND_COUNTRY, 3);
    let pending = war_before(&image, BASE, frame()).unwrap();
    image.attach();
    let event = war_returned(&image, BASE, pending).unwrap();
    assert_eq!(
        event.target,
        CountryIdentity {
            reference: 3,
            id: 2,
            instance: 0xcccc
        }
    );
    image.q(SELECTED + crelation_status::WAR_RELATION, 0);
    let pending = war_before(&image, BASE, frame()).unwrap();
    image.attach();
    image.d(CANDIDATE + crelation::SECOND_COUNTRY, 2);
    assert_eq!(
        war_returned(&image, BASE, pending),
        Err("event feed candidate raw pair changed during native operation")
    );
}

#[test]
fn high_country_references_and_matrix_indices_use_live_header_bounds() {
    for slots in [1500_u32, 5000] {
        let target = slots - 1;
        let mut image = Image::new();
        image.array(STATE + cgame_state::COUNTRIES, 0x90000, slots);
        image.array(STATE + cgame_state::COUNTRY_LINKS, 0xa0000, slots);
        image.q(0x90000, RESERVED);
        image.q(0x90000 + 8, ACTOR);
        image.q(0x90000 + target as usize * 8, TARGET);
        image.d(0xa0000 + 4, 1);
        image.d(0xa0000 + target as usize * 4, target);
        image.d(TARGET + ccountry::ID, target);
        image.d(SELECTED + crelation_status::TARGET_COUNTRY, target);
        image.d(CANDIDATE + crelation::SECOND_COUNTRY, target);
        image.array(0x21000 + engine_native_parts::cdiplomacy_status_layout::RELATIONS, 0xb0000, slots);
        image.array(0x31000 + engine_native_parts::cdiplomacy_status_layout::RELATIONS, 0xc0000, slots);
        image.q(0xb0000 + target as usize * 8, SELECTED);
        image.q(0xc0000 + 8, RECIPROCAL);
        let pending = war_before(&image, BASE, frame()).unwrap();
        image.attach();
        assert_eq!(
            war_returned(&image, BASE, pending).unwrap().target.id,
            target
        );
    }
}

#[test]
fn country_zero_uses_real_reserved_object_and_matrix_slot_zero() {
    let mut image = Image::new();
    image.d(SELECTED + crelation_status::TARGET_COUNTRY, 0);
    image.d(CANDIDATE + crelation::SECOND_COUNTRY, 0);
    image.q(0x22000, SELECTED);
    image.q(0x12000 + 8, 0x13000);
    image.status(0x13000, 0x11000, 1);
    let pending = war_before(&image, BASE, frame()).unwrap();
    image.q(SELECTED + crelation_status::WAR_RELATION, CANDIDATE);
    image.q(0x13000 + crelation_status::WAR_RELATION, CANDIDATE);
    let event = war_returned(&image, BASE, pending).unwrap();
    assert_eq!(
        event.target,
        CountryIdentity {
            reference: 0,
            id: 0,
            instance: 0xaaaa
        }
    );
    image.d(RESERVED + ccountry::ID, 2);
    assert_eq!(
        war_before(&image, BASE, frame()).err(),
        Some("event feed reserved country ID differs")
    );
}

#[test]
fn register_candidate_and_status_crosslinks_are_independently_authenticated() {
    assert_eq!(
        war_before(&Image::new(), BASE, WarFrame { rdi: 0, ..frame() }).err(),
        Some("event feed candidate register mismatch")
    );
    let mut image = Image::new();
    image.q(SELECTED + crelation_status::DIPLOMACY_STATUS, 0x31000);
    assert!(war_before(&image, BASE, frame()).is_err());
    let mut image = Image::new();
    image.q(0x22000 + 2 * 8, RECIPROCAL);
    assert!(war_before(&image, BASE, frame()).is_err());
    let mut image = Image::new();
    image.d(CANDIDATE + crelation::TYPE_ID, 0);
    assert_eq!(
        war_before(&image, BASE, frame()).err(),
        Some("event feed candidate type mismatch")
    );
}

#[test]
fn cancel_retirement_short_read_and_reentrant_replacement_refuse_postproof() {
    for field in [crelation::CANCEL, crelation::RETIRED] {
        let mut image = Image::new();
        image.bytes(CANDIDATE + field, &[1]);
        assert_eq!(
            war_before(&image, BASE, frame()).err(),
            Some("event feed candidate canceled or retired")
        );
        image.bytes(CANDIDATE + field, &[0]);
        let pending = war_before(&image, BASE, frame()).unwrap();
        image.attach();
        image.bytes(CANDIDATE + field, &[1]);
        assert_eq!(
            war_returned(&image, BASE, pending),
            Err("event feed candidate canceled or retired")
        );
    }
    for slot in [SELECTED, RECIPROCAL] {
        let mut image = Image::new();
        let pending = war_before(&image, BASE, frame()).unwrap();
        image.attach();
        image.q(slot + crelation_status::WAR_RELATION, CANDIDATE + 0x1000);
        assert_eq!(
            war_returned(&image, BASE, pending),
            Err("event feed candidate reciprocal attachment not proved")
        );
    }
    let mut image = Image::new();
    let pending = war_before(&image, BASE, frame()).unwrap();
    image.attach();
    image.0.remove(&(CANDIDATE + crelation::SECOND_COUNTRY + 3));
    assert_eq!(
        war_returned(&image, BASE, pending),
        Err("event feed operand unreadable")
    );
}

#[test]
fn fresh_postproof_refuses_replaced_country_and_world_roots() {
    let mut image = Image::new();
    let pending = war_before(&image, BASE, frame()).unwrap();
    image.attach();
    image.d(TARGET + ccountry::ID, 3);
    assert_eq!(
        war_returned(&image, BASE, pending),
        Err("event feed country identity changed during native operation")
    );
    let mut image = Image::new();
    let pending = war_before(&image, BASE, frame()).unwrap();
    image.q(BASE + cgame_state::SINGLETON_RVA.0, STATE + 0x1000);
    assert_eq!(
        war_returned(&image, BASE, pending),
        Err("event feed game state changed during native operation")
    );
}

#[test]
fn annex_copies_pre_dispatch_roles_including_zero_and_aliases() {
    let mut image = Image::new();
    image.d(SCOPE + cevent_scope::COUNTRY_TAG, 0);
    image.d(FROM + cevent_scope::COUNTRY_TAG, 3);
    let event = annex_before(&image, BASE, annex_frame()).unwrap();
    image.d(SCOPE + cevent_scope::COUNTRY_TAG, 2);
    image.d(FROM + cevent_scope::COUNTRY_TAG, 1);
    assert_eq!(event.source, Source::Annex);
    assert_eq!(
        event.actor,
        CountryIdentity {
            reference: 0,
            id: 0,
            instance: 0xaaaa
        }
    );
    assert_eq!(
        event.target,
        CountryIdentity {
            reference: 3,
            id: 2,
            instance: 0xcccc
        }
    );
}

#[test]
fn annex_missing_scope_role_or_wrong_native_frame_is_not_complete_empty() {
    let mut image = Image::new();
    image.q(SCOPE + cevent_scope::FROM, 0);
    assert_eq!(
        annex_before(&image, BASE, annex_frame()),
        Err("event feed object identity mismatch")
    );
    assert_eq!(
        annex_before(
            &Image::new(),
            BASE,
            AnnexFrame {
                r9: 1,
                ..annex_frame()
            }
        ),
        Err("event feed annex register frame mismatch")
    );
    assert_eq!(
        annex_before(
            &Image::new(),
            BASE,
            AnnexFrame {
                rbp: SCOPE,
                ..annex_frame()
            }
        ),
        Err("event feed annex register frame mismatch")
    );
    let mut image = Image::new();
    image.0.remove(&(FROM + cevent_scope::COUNTRY_TAG + 3));
    assert_eq!(
        annex_before(&image, BASE, annex_frame()),
        Err("event feed operand unreadable")
    );
}

mod provenance_sites {
    use super::*;
    use engine_native::cstate;
    use crate::peace_conference;
    use provenance::{ENTRIES, STATE_OWNER_CALLERS, World, route, state_owner};

    const CONFERENCE: usize = 0x90000;
    const PROVINCE_STATE: usize = 0x91000;

    #[test]
    fn world_readers_authenticate_objects_and_keep_references_and_instances() {
        let mut image = Image::new();
        let world = World::new(&image, BASE).unwrap();
        assert_eq!(world.hour(), Ok(60828400));
        assert_eq!(world.country_object(ACTOR).unwrap().instance, 0xbbbb);
        assert_eq!(world.country(3).unwrap().id, 2);
        assert_eq!(world.diplomacy_owner(0x31000).unwrap().reference, 2);
        assert_eq!(
            world
                .war_endpoints(CANDIDATE)
                .unwrap()
                .map(|country| country.id),
            [1, 2]
        );
        image.q(CONFERENCE, BASE + peace_conference::VTABLE_RVA.0);
        image.q(CONFERENCE + peace_conference::REFERENCE, 0x7_0000_0031);
        image.d(CONFERENCE + provenance::MAIN_LOSER, 2);
        image.d(CONFERENCE + provenance::MAIN_WINNER, 1);
        let world = World::new(&image, BASE).unwrap();
        let (reference, [winner, loser]) = world.conference(CONFERENCE).unwrap();
        assert_eq!((reference, winner.id, loser.id), (0x7_0000_0031, 1, 2));
        image.q(CONFERENCE + peace_conference::REFERENCE, 0);
        let world = World::new(&image, BASE).unwrap();
        assert_eq!(
            world.conference(CONFERENCE).err(),
            Some("event feed conference reference is null")
        );
        assert!(world.war_endpoints(CONFERENCE).is_err());
        assert!(world.country_object(CANDIDATE).is_err());
        image.q(PROVINCE_STATE, BASE + cstate::VTABLE_RVA.0);
        image.d(PROVINCE_STATE + cstate::ID, 64);
        image.d(PROVINCE_STATE + cstate::OWNER, 2);
        assert_eq!(state_owner(&image, BASE, PROVINCE_STATE), Ok((64, 2)));
        assert!(state_owner(&image, BASE, CONFERENCE).is_err());
    }
}

#[test]
fn surrender_target_changes_do_not_invalidate_war_postproof_or_rewrite_pre_call_data() {
    let mut image = Image::new();
    let pending = war_before(&image, BASE, frame()).unwrap();
    let before = pending.operands();
    image.attach();
    image.d(TARGET + ccountry::SURRENDER_TARGET, 0xdddd);
    assert_eq!(war_returned(&image, BASE, pending).unwrap(), before);
}

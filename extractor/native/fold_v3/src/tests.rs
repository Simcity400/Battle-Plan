//! What a prototype has to prove before its numbers mean anything.
//!
//! Two families. The EQUIVALENCE tests say the state projects back to the exact
//! bytes it was assigned, across the shapes the real fold actually emits: keyed
//! objects, an array whose order is a string order and not a numeric one, and a
//! section that turns into `null` on an hour its domain was unknown. The
//! MUTATION tests say the committed delta is the only thing a replica needs and
//! that tampering with it FAILS -- a journal you cannot break is a journal you
//! have not tested.

use super::*;

/// A lone backslash and a lone double quote, assembled from their code points.
///
/// Escaped member names are what these tests have to exercise and what a source
/// file is least able to carry safely: one round trip through a tool that decodes
/// escapes and the fixture silently becomes the UNescaped name, which every
/// implementation passes. That already happened once to this file. Bytes built
/// from code points cannot be decoded by anything.
const ESCAPE: &str = "\u{5c}";
const QUOTE: &str = "\u{22}";

fn quote_backslash() -> String {
    format!("{QUOTE}{ESCAPE}")
}

fn parse(json: &str) -> CanonValue {
    CanonValue::parse(json.as_bytes()).expect("test fixture is canonical JSON")
}

fn object_state(name: &str, depth: usize) -> SectionState {
    SectionState::new(SectionSpec {
        name: name.to_string(),
        shape: Shape::Object,
        grain_depth: depth,
    })
}

fn divisions_state(depth: usize) -> SectionState {
    SectionState::new(SectionSpec {
        name: "divisions".to_string(),
        shape: Shape::KeyedArray {
            key_member: "div_uid".to_string(),
        },
        grain_depth: depth,
    })
}

/// Assign `json`, commit, and return the delta.
fn assign(state: &mut SectionState, json: &str) -> SectionDelta {
    let value = parse(json);
    let mut hour = state.begin_hour().expect("hour opens");
    hour.assign_section(&value).expect("assignment succeeds");
    hour.commit()
}

// ---------------------------------------------------------------------------
// Equivalence
// ---------------------------------------------------------------------------

#[test]
fn projection_reproduces_the_assigned_bytes_at_every_grain_depth() {
    let json = r#"{"AAA":{"army":{"divisions":3,"names":["a","b"]},"politics":{"party":"x"}},"BBB":{"army":{"divisions":0,"names":[]},"politics":{"party":"y"}},"CCC":{}}"#;
    for depth in 1..=5 {
        let mut state = object_state("countries", depth);
        assign(&mut state, json);
        assert_eq!(
            String::from_utf8(state.project()).unwrap(),
            json,
            "grain depth {depth} did not project back byte-identically"
        );
    }
}

#[test]
fn projection_reproduces_nested_arrays_and_empty_containers() {
    let json = r#"{"a":[[1,2],[],{"k":[]}],"b":{},"c":[],"d":null,"e":[{"z":1}]}"#;
    for depth in 1..=6 {
        let mut state = object_state("map", depth);
        assign(&mut state, json);
        assert_eq!(
            String::from_utf8(state.project()).unwrap(),
            json,
            "grain depth {depth}"
        );
    }
}

#[test]
fn scalar_spellings_survive_the_round_trip() {
    // The whole reason the cache keeps a value model of its own: a number's
    // source token is not recoverable from its `f64`.
    let json = r#"{"a":{"v":189352.52932739258},"b":{"v":-0.0},"c":{"v":1.50},"d":{"v":1e3}}"#;
    let mut state = object_state("supply", 2);
    assign(&mut state, json);
    assert_eq!(String::from_utf8(state.project()).unwrap(), json);
}

#[test]
fn keyed_array_projects_in_string_order_not_numeric_order() {
    // The trap the design review named: `10:1` precedes `2:1` lexically and
    // follows it numerically, and the fold's own `BTreeMap<String, _>` uses the
    // lexical order. A keyed grain that sorted numerically would silently
    // reorder every division array with more than nine of anything.
    let json = r#"[{"div_uid":"10:1","hp":5},{"div_uid":"2:1","hp":6},{"div_uid":"3:7","hp":7}]"#;
    let mut state = divisions_state(1);
    assign(&mut state, json);
    assert_eq!(String::from_utf8(state.project()).unwrap(), json);
}

#[test]
fn keyed_array_survives_insert_delete_and_reinsert() {
    let mut state = divisions_state(1);
    let first = r#"[{"div_uid":"10:1","hp":5},{"div_uid":"2:1","hp":6}]"#;
    assign(&mut state, first);
    // Insert in the middle -- the case a positional array grain would spell as a
    // whole-array replacement.
    let second = r#"[{"div_uid":"10:1","hp":5},{"div_uid":"11:9","hp":1},{"div_uid":"2:1","hp":6}]"#;
    let delta = assign(&mut state, second);
    assert_eq!(String::from_utf8(state.project()).unwrap(), second);
    assert_eq!(
        delta.mutations, 1,
        "an insertion must move exactly one grain, not the whole array"
    );

    let third = r#"[{"div_uid":"10:1","hp":5},{"div_uid":"2:1","hp":6}]"#;
    assign(&mut state, third);
    assert_eq!(String::from_utf8(state.project()).unwrap(), third);

    let revision_before = state.revision();
    let fourth = r#"[{"div_uid":"10:1","hp":5},{"div_uid":"11:9","hp":2},{"div_uid":"2:1","hp":6}]"#;
    assign(&mut state, fourth);
    assert_eq!(String::from_utf8(state.project()).unwrap(), fourth);
    // The reinserted key must not restart at a revision it already used. A
    // per-child counter dies with the child; this one does not.
    assert!(
        state
            .grain(&[Seg::Key("11:9".into())])
            .expect("reinserted grain")
            .rev()
            > revision_before,
        "a reinserted grain must carry a revision beyond every revision the key already used"
    );
}

#[test]
fn keyed_array_refuses_an_order_or_an_identity_it_cannot_reproduce() {
    let mut state = divisions_state(1);
    // Descending -- the projection would come back ascending.
    let value = parse(r#"[{"div_uid":"2:1"},{"div_uid":"10:1"}]"#);
    let mut hour = state.begin_hour().unwrap();
    assert!(hour.assign_section(&value).is_err());
    drop(hour);

    let mut state = divisions_state(1);
    // A duplicate identity would come back once.
    let value = parse(r#"[{"div_uid":"2:1"},{"div_uid":"2:1"}]"#);
    let mut hour = state.begin_hour().unwrap();
    assert!(hour.assign_section(&value).is_err());
    drop(hour);

    let mut state = divisions_state(1);
    let value = parse(r#"[{"no_uid":1}]"#);
    let mut hour = state.begin_hour().unwrap();
    assert!(hour.assign_section(&value).is_err());
}

#[test]
fn a_section_that_becomes_null_and_comes_back_projects_both_ways() {
    // `divisions` and `special_projects` are spelled `null` on an hour their
    // domain was unknown. A state that could only hold containers would have to
    // spell that as an empty array, which is a different document.
    let mut state = divisions_state(1);
    let populated = r#"[{"div_uid":"1:1","hp":9}]"#;
    assign(&mut state, populated);
    assert_eq!(String::from_utf8(state.project()).unwrap(), populated);

    let delta = assign(&mut state, "null");
    assert_eq!(String::from_utf8(state.project()).unwrap(), "null");
    assert_eq!(state.root_kind(), RootKind::Scalar);
    // The root record sits BETWEEN the removals and the upserts: the kind can
    // only move once the grains that contradict it are gone and before the ones
    // that need it arrive. Here that means one removal, then the root, then the
    // scalar's own upsert.
    let parts = records(&delta.bytes);
    let ops: Vec<u8> = parts.iter().map(|record| record[0]).collect();
    assert_eq!(ops, vec![OP_REMOVE, OP_ROOT, OP_UPSERT]);

    let delta = assign(&mut state, populated);
    assert_eq!(String::from_utf8(state.project()).unwrap(), populated);
    assert_eq!(state.root_kind(), RootKind::Array);
    let ops: Vec<u8> = records(&delta.bytes)
        .iter()
        .map(|record| record[0])
        .collect();
    assert_eq!(ops, vec![OP_REMOVE, OP_ROOT, OP_UPSERT]);

    // And a replica driven only by those deltas follows both transitions.
    let mut producer = divisions_state(1);
    let mut replica = divisions_state(1);
    for json in [populated, "null", populated, "null"] {
        let delta = assign(&mut producer, json);
        replica.apply_delta(&delta.bytes).expect("replica applies");
        assert_eq!(replica.project(), producer.project(), "at {json}");
        assert_eq!(replica.state_root(), delta.state_root, "at {json}");
    }
}

#[test]
fn an_escaped_key_respelling_replaces_the_stored_spelling() {
    // `CanonValue` orders members by their DECODED text but writes back their
    // SOURCE spelling, so a plain `a` and the same name spelled as a `u0061`
    // escape are ONE address with two spellings. A state that kept the first
    // spelling would project a document that no longer matches its source.
    let escaped = format!("{{{}u0061{QUOTE}:1}}", quote_backslash());
    assert!(escaped.contains(ESCAPE), "the fixture lost its escape");

    let mut state = object_state("countries", 1);
    assign(&mut state, r#"{"a":1}"#);
    assign(&mut state, &escaped);
    assert_eq!(
        String::from_utf8(state.project()).unwrap(),
        escaped,
        "the stored key spelling must follow the assignment"
    );
    assert_eq!(state.grain_count(), 1, "one address, not two");
    // The spelling is inside the state root's hash, so a respelling that
    // retracted the OLD contribution under the NEW spelling would leave a root
    // that is stable, self-consistent and wrong.
    assert_eq!(state.state_root(), state.recomputed_state_root());

    // And a removal has to retract under the spelling it was added with.
    assign(&mut state, r#"{"b":1}"#);
    assert_eq!(state.state_root(), state.recomputed_state_root());
    assert_eq!(String::from_utf8(state.project()).unwrap(), r#"{"b":1}"#);

    // A replica driven only by the deltas must land on the same bytes and root.
    let mut producer = object_state("countries", 1);
    let mut replica = object_state("countries", 1);
    for json in [r#"{"a":1}"#.to_string(), escaped, r#"{"b":1}"#.to_string()] {
        let delta = assign(&mut producer, &json);
        replica.apply_delta(&delta.bytes).expect("replica applies");
        assert_eq!(replica.state_root(), delta.state_root, "at {json}");
        assert_eq!(replica.project(), producer.project(), "at {json}");
    }
}

#[test]
fn a_respelling_within_one_hour_carries_the_new_spelling_into_the_delta() {
    // Two writes to one address inside one hour, the second under a different
    // escaped spelling. `BTreeMap::insert` keeps the ORIGINAL key when the new
    // one compares equal, so the journal would carry the old spelling while the
    // state carried the new one -- a replica that agreed on every digest and
    // disagreed on the bytes.
    let escaped_key = format!("{ESCAPE}u0061");
    let mut producer = object_state("countries", 1);
    let mut replica = object_state("countries", 1);
    let delta = {
        let plain = parse(r#"{"x":1}"#);
        let other = parse(r#"{"x":2}"#);
        let mut hour = producer.begin_hour().unwrap();
        hour.upsert(vec![Seg::Key("a".into())], &plain).unwrap();
        hour.upsert(vec![Seg::Key(escaped_key.clone())], &other)
            .unwrap();
        hour.commit()
    };
    replica.apply_delta(&delta.bytes).expect("replica applies");
    // The LAST write's spelling is the one that stands, in the state and in the
    // delta alike.
    assert_eq!(
        String::from_utf8(producer.project()).unwrap(),
        format!("{{{QUOTE}{escaped_key}{QUOTE}:{{{QUOTE}x{QUOTE}:2}}}}")
    );
    assert_eq!(replica.project(), producer.project());
    assert_eq!(replica.state_root(), delta.state_root);
    assert_eq!(producer.state_root(), producer.recomputed_state_root());
}

#[test]
fn an_assignment_that_cannot_be_represented_leaves_the_state_untouched() {
    let mut state = divisions_state(1);
    let good = r#"[{"div_uid":"1:1","hp":9}]"#;
    assign(&mut state, good);
    let root_before = state.state_root();
    let revision_before = state.revision();

    let bad = parse(r#"[{"div_uid":"9:9"},{"div_uid":"1:1"}]"#);
    {
        let mut hour = state.begin_hour().unwrap();
        assert!(hour.assign_section(&bad).is_err());
        // Nothing was written, so this is a clean commit, not a dropped one.
        hour.commit();
    }
    assert_eq!(state.state_root(), root_before);
    assert_eq!(state.revision(), revision_before);
    assert_eq!(String::from_utf8(state.project()).unwrap(), good);
}

// ---------------------------------------------------------------------------
// Path sets the projection could not write back
// ---------------------------------------------------------------------------

#[test]
fn an_empty_container_section_is_its_root_kind_and_nothing_else() {
    // An empty object decomposes to NO grains. A grain at the empty path would
    // be a whole-section value under a container root -- the one path set
    // `project` cannot spell.
    let mut state = object_state("countries", 2);
    assign(&mut state, "{}");
    assert_eq!(String::from_utf8(state.project()).unwrap(), "{}");
    assert_eq!(state.grain_count(), 0);
    assert_eq!(state.root_kind(), RootKind::Object);

    let mut state = divisions_state(1);
    assign(&mut state, "[]");
    assert_eq!(String::from_utf8(state.project()).unwrap(), "[]");
    assert_eq!(state.grain_count(), 0);

    // And it still round-trips through a replica.
    let mut producer = object_state("countries", 2);
    let mut replica = object_state("countries", 2);
    for json in [r#"{"AAA":{"x":1}}"#, "{}", r#"{"BBB":{"y":2}}"#] {
        let delta = assign(&mut producer, json);
        replica.apply_delta(&delta.bytes).expect("replica applies");
        assert_eq!(replica.project(), producer.project(), "at {json}");
        assert_eq!(replica.state_root(), delta.state_root, "at {json}");
    }
}

#[test]
fn a_member_that_turns_from_a_value_into_a_container_replays() {
    // Capture t30 does this: `supply.hubs` is a scalar on one hour and an object
    // on the next. One grain at `[hubs]` becomes grains at `[hubs, ...]`, and
    // `[hubs]` sorts BEFORE `[hubs, x]` -- so a delta written in one ascending
    // pass asks the reader to install a value at an address that still has a
    // container beneath it. Removals have to go first.
    let mut producer = object_state("supply", 2);
    let mut replica = object_state("supply", 2);
    for json in [
        r#"{"hubs":1,"z":{"k":0}}"#,
        r#"{"hubs":{"x":2,"y":3},"z":{"k":0}}"#,
        r#"{"hubs":4,"z":{"k":0}}"#,
        r#"{"hubs":{"x":9},"z":{"k":1}}"#,
    ] {
        let delta = assign(&mut producer, json);
        replica
            .apply_delta(&delta.bytes)
            .unwrap_or_else(|e| panic!("replica rejected the delta at {json}: {e}"));
        assert_eq!(String::from_utf8(producer.project()).unwrap(), json);
        assert_eq!(replica.project(), producer.project(), "at {json}");
        assert_eq!(replica.state_root(), delta.state_root, "at {json}");
    }
}

#[test]
fn a_delta_whose_removals_follow_its_upserts_is_refused() {
    let mut producer = object_state("supply", 2);
    assign(&mut producer, r#"{"hubs":1,"z":{"k":0}}"#);
    let delta = assign(&mut producer, r#"{"hubs":{"x":2},"z":{"k":0}}"#);
    let parts = records(&delta.bytes);
    assert!(parts.len() >= 2);
    // The producer wrote the removal first; hand the replica the other order.
    let mut reversed = parts.clone();
    reversed.reverse();
    let mut replica = object_state("supply", 2);
    let first = assign(
        &mut { object_state("supply", 2) },
        r#"{"hubs":1,"z":{"k":0}}"#,
    );
    replica.apply_delta(&first.bytes).unwrap();
    assert!(replica.apply_delta(&reversed.concat()).is_err());
}

#[test]
fn an_hour_that_doubles_back_emits_nothing() {
    // The delta is a record of what CHANGED, not of what the producer DID. An
    // hour that inserts a grain and then removes it, or removes one and then
    // puts it back, or flips the root kind and flips it back, changed nothing --
    // and a delta that said otherwise would ask the replica to remove a grain it
    // never held or to install bytes it already has, both of which the strict
    // decoder rejects.
    let mut producer = object_state("countries", 2);
    let mut replica = object_state("countries", 2);
    let first = assign(&mut producer, r#"{"AAA":{"x":1}}"#);
    replica.apply_delta(&first.bytes).unwrap();

    let value = parse("1");
    let delta = {
        let mut hour = producer.begin_hour().unwrap();
        // Inserted and withdrawn.
        hour.upsert(vec![Seg::Key("BBB".into()), Seg::Key("x".into())], &value)
            .unwrap();
        hour.remove(&[Seg::Key("BBB".into()), Seg::Key("x".into())])
            .unwrap();
        // Removed and restored.
        hour.remove(&[Seg::Key("AAA".into()), Seg::Key("x".into())])
            .unwrap();
        hour.upsert(vec![Seg::Key("AAA".into()), Seg::Key("x".into())], &value)
            .unwrap();
        hour.commit()
    };
    assert_eq!(delta.mutations, 0, "a round trip is not a change");
    assert!(delta.bytes.is_empty());
    replica
        .apply_delta(&delta.bytes)
        .expect("an empty delta applies");
    assert_eq!(replica.state_root(), delta.state_root);
    assert_eq!(replica.project(), producer.project());
    assert_eq!(producer.state_root(), producer.recomputed_state_root());
}

#[test]
fn a_root_kind_flipped_and_flipped_back_emits_nothing() {
    let mut producer = divisions_state(1);
    let mut replica = divisions_state(1);
    let populated = r#"[{"div_uid":"1:1","hp":9}]"#;
    let first = assign(&mut producer, populated);
    replica.apply_delta(&first.bytes).unwrap();

    let delta = {
        let value = parse(populated);
        let null = parse("null");
        let mut hour = producer.begin_hour().unwrap();
        hour.assign_section(&null).unwrap();
        hour.assign_section(&value).unwrap();
        hour.commit()
    };
    assert_eq!(delta.mutations, 0);
    replica.apply_delta(&delta.bytes).unwrap();
    assert_eq!(replica.state_root(), delta.state_root);
    assert_eq!(String::from_utf8(producer.project()).unwrap(), populated);
}

#[test]
fn an_erase_then_a_respelled_reinstall_carries_the_new_spelling() {
    // The rekey path consults `grains`, and after an erase there is no grain to
    // consult. The delta is derived from the state at commit, so it carries the
    // spelling that actually stands.
    let escaped_key = format!("{ESCAPE}u0061");
    let mut producer = object_state("countries", 1);
    let mut replica = object_state("countries", 1);
    let first = assign(&mut producer, r#"{"a":1}"#);
    replica.apply_delta(&first.bytes).unwrap();

    let other = parse("2");
    let delta = {
        let mut hour = producer.begin_hour().unwrap();
        hour.remove(&[Seg::Key("a".into())]).unwrap();
        hour.upsert(vec![Seg::Key(escaped_key.clone())], &other)
            .unwrap();
        hour.commit()
    };
    replica.apply_delta(&delta.bytes).expect("replica applies");
    assert_eq!(
        String::from_utf8(producer.project()).unwrap(),
        format!("{{{QUOTE}{escaped_key}{QUOTE}:2}}")
    );
    assert_eq!(replica.project(), producer.project());
    assert_eq!(replica.state_root(), delta.state_root);
}

#[test]
fn a_root_addressed_the_wrong_way_is_refused() {
    // Below the root a container's kind is read off the address that reaches into
    // it, so it cannot disagree. The root has no such address.
    let mut state = object_state("countries", 1);
    let value = parse("1");
    let mut hour = state.begin_hour().unwrap();
    assert!(hour.upsert(vec![Seg::Index(0)], &value).is_err());
    hour.commit();

    let mut state = divisions_state(1);
    let mut hour = state.begin_hour().unwrap();
    assert!(hour.upsert(vec![Seg::Index(0)], &value).is_err());
    hour.commit();
}

#[test]
fn a_root_kind_cannot_change_out_from_under_its_grains() {
    let mut state = object_state("countries", 1);
    assign(&mut state, r#"{"a":1}"#);
    let mut hour = state.begin_hour().unwrap();
    // `project` would return `null` while the state root still committed `a`.
    assert!(hour.set_root_kind(RootKind::Scalar).is_err());
    hour.commit();
}

#[test]
fn a_poisoned_section_holds_nothing() {
    // A flag that only blocks writes still lets a half-applied state be read.
    let mut producer = object_state("countries", 2);
    let delta = assign(&mut producer, HOURS[0]);
    let mut replica = object_state("countries", 2);
    let parts = records(&delta.bytes);
    let mut bytes = parts[0].clone();
    bytes.push(99);
    assert!(replica.apply_delta(&bytes).is_err());
    assert!(replica.poisoned().is_some());
    assert_eq!(replica.grain_count(), 0, "a poisoned section serves nothing");
    assert_eq!(
        replica.state_root(),
        replica.recomputed_state_root(),
        "and what it does report is still self-consistent"
    );
}

#[test]
fn a_value_and_a_container_cannot_share_one_address() {
    // `{"a":1}` and `{"a":{"b":2}}` at once would project as a duplicate member.
    let mut state = object_state("countries", 3);
    let scalar = parse("1");
    let nested = parse("2");
    let mut hour = state.begin_hour().unwrap();
    hour.upsert(vec![Seg::Key("a".into())], &scalar).unwrap();
    assert!(hour
        .upsert(
            vec![Seg::Key("a".into()), Seg::Key("b".into())],
            &nested
        )
        .is_err());
    hour.commit();

    // And the other way round.
    let mut state = object_state("countries", 3);
    let mut hour = state.begin_hour().unwrap();
    hour.upsert(vec![Seg::Key("a".into()), Seg::Key("b".into())], &nested)
        .unwrap();
    assert!(hour.upsert(vec![Seg::Key("a".into())], &scalar).is_err());
    hour.commit();
}

#[test]
fn two_siblings_cannot_disagree_on_how_their_container_addresses_them() {
    let mut state = object_state("map", 3);
    let value = parse("1");
    let mut hour = state.begin_hour().unwrap();
    hour.upsert(vec![Seg::Key("a".into()), Seg::Index(0)], &value)
        .unwrap();
    assert!(hour
        .upsert(vec![Seg::Key("a".into()), Seg::Key("k".into())], &value)
        .is_err());
    hour.commit();
}

#[test]
fn a_member_name_the_projection_could_not_quote_is_refused() {
    let mut state = object_state("countries", 1);
    let value = parse("1");
    let mut hour = state.begin_hour().unwrap();
    // A bare quote inside a member name body is written between quotes verbatim,
    // so it would emit JSON no parser accepts.
    assert!(hour
        .upsert(vec![Seg::Key(QUOTE.to_string())], &value)
        .is_err());
    // A lone trailing backslash is not a valid escape either.
    assert!(hour
        .upsert(vec![Seg::Key(ESCAPE.to_string())], &value)
        .is_err());
    hour.commit();
}

#[test]
fn the_empty_path_belongs_to_a_scalar_root_only() {
    let mut state = object_state("countries", 1);
    let value = parse("1");
    let mut hour = state.begin_hour().unwrap();
    assert!(hour.upsert(Vec::new(), &value).is_err());
    hour.commit();

    // Under a scalar root it is the only legal address.
    let mut state = divisions_state(1);
    assign(&mut state, "null");
    let mut hour = state.begin_hour().unwrap();
    assert!(hour
        .upsert(vec![Seg::Key("1:1".into())], &value)
        .is_err());
    hour.commit();
}

#[test]
fn a_delta_carrying_an_unrepresentable_path_is_refused_and_poisons() {
    let mut producer = object_state("countries", 2);
    let delta = assign(&mut producer, r#"{"AAA":{"x":1}}"#);
    let mut replica = object_state("countries", 2);
    replica.apply_delta(&delta.bytes).unwrap();

    // A hand-built delta whose upsert lands beneath an existing grain.
    let mut bytes = Vec::new();
    bytes.push(OP_UPSERT);
    encode_path_into(
        &[
            Seg::Key("AAA".into()),
            Seg::Key("x".into()),
            Seg::Key("deeper".into()),
        ],
        &mut bytes,
    );
    write_varint(&mut bytes, 1);
    bytes.push(b'1');
    assert!(replica.apply_delta(&bytes).is_err());
    // A state that took half a delta must not go on serving itself as if it had
    // taken all of it.
    assert!(replica.poisoned().is_some());
}

#[test]
fn a_delta_rejected_part_way_through_poisons_the_replica() {
    let mut producer = object_state("countries", 2);
    assign(&mut producer, HOURS[0]);
    let delta = assign(&mut producer, HOURS[1]);

    let mut replica = object_state("countries", 2);
    let first = assign(&mut { object_state("countries", 2) }, HOURS[0]);
    replica.apply_delta(&first.bytes).unwrap();

    // One good record, then an unknown opcode.
    let parts = records(&delta.bytes);
    let mut bytes = parts[0].clone();
    bytes.push(99);
    assert!(replica.apply_delta(&bytes).is_err());
    assert!(
        replica.poisoned().is_some(),
        "a half-applied delta leaves a state that is neither the old one nor the new one"
    );
    assert!(replica.apply_delta(&[]).is_err());
}

#[test]
fn an_overflowing_varint_alias_is_refused() {
    // Nine continuation bytes then `0x02` would truncate to zero -- a second
    // spelling of a value the writer spells in one byte.
    let mut bytes = vec![OP_UPSERT];
    for _ in 0..9 {
        bytes.push(0x80);
    }
    bytes.push(0x02);
    let mut replica = object_state("countries", 2);
    assert!(replica.apply_delta(&bytes).is_err());
}

// ---------------------------------------------------------------------------
// The state root
// ---------------------------------------------------------------------------

#[test]
fn the_incremental_root_equals_a_full_recomputation() {
    // The incremental root is the design's load-bearing claim and therefore the
    // thing most worth disbelieving: an add not matched by its subtraction leaves
    // a root that is stable, self-consistent and wrong.
    let mut state = object_state("countries", 2);
    for json in [
        r#"{"AAA":{"x":1},"BBB":{"x":2}}"#,
        r#"{"AAA":{"x":9},"BBB":{"x":2},"CCC":{"x":3}}"#,
        r#"{"BBB":{"x":2}}"#,
        r#"{"AAA":{"x":1},"BBB":{"x":2}}"#,
    ] {
        assign(&mut state, json);
        assert_eq!(
            state.state_root(),
            state.recomputed_state_root(),
            "after assigning {json}"
        );
    }
}

#[test]
fn the_root_distinguishes_a_null_section_from_an_empty_one() {
    let mut null_state = divisions_state(1);
    assign(&mut null_state, "null");
    let mut empty_state = divisions_state(1);
    assign(&mut empty_state, "[]");
    assert_ne!(null_state.state_root(), empty_state.state_root());
}

// ---------------------------------------------------------------------------
// Mutation tests: the journal must be breakable
// ---------------------------------------------------------------------------

/// Drive a replica from nothing but the committed deltas, and require it to
/// reach the producer's state root and bytes at every hour.
fn replay(hours: &[&str], spec: impl Fn() -> SectionState) -> Vec<SectionDelta> {
    let mut producer = spec();
    let mut replica = spec();
    let mut deltas = Vec::new();
    for json in hours {
        let delta = assign(&mut producer, json);
        replica
            .apply_delta(&delta.bytes)
            .expect("the replica applies the delta");
        assert_eq!(
            replica.state_root(),
            delta.state_root,
            "replica root diverged at {json}"
        );
        assert_eq!(
            replica.project(),
            producer.project(),
            "replica bytes diverged at {json}"
        );
        deltas.push(delta);
    }
    deltas
}

const HOURS: [&str; 4] = [
    r#"{"AAA":{"x":1,"y":2},"BBB":{"x":2,"y":3}}"#,
    r#"{"AAA":{"x":9,"y":2},"BBB":{"x":2,"y":3},"CCC":{"x":3,"y":4}}"#,
    r#"{"AAA":{"x":9,"y":5},"CCC":{"x":3,"y":4}}"#,
    r#"{"AAA":{"x":9,"y":5},"BBB":{"x":7,"y":7},"CCC":{"x":3,"y":4}}"#,
];

#[test]
fn a_replica_driven_only_by_the_deltas_stays_byte_identical() {
    let deltas = replay(&HOURS, || object_state("countries", 2));
    assert!(deltas.iter().all(|delta| delta.mutations > 0));
}

/// Split one delta into its records so a test can drop, duplicate or reorder
/// them the way a broken writer would.
fn records(bytes: &[u8]) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let mut reader = ByteReader::new(bytes);
    while !reader.is_empty() {
        let start = reader.position;
        let op = reader.u8().unwrap();
        if op == OP_ROOT {
            reader.u8().unwrap();
        } else {
            reader.path().unwrap();
            if op == OP_UPSERT {
                let len = reader.varint().unwrap() as usize;
                reader.take(len).unwrap();
            }
        }
        out.push(bytes[start..reader.position].to_vec());
    }
    out
}

/// Bring a replica up to just before the last hour, then hand it `tampered` in
/// place of that hour's delta. Returns what the replica did and, if it accepted
/// the bytes, whether it still reached the producer's root.
fn tamper(tampered: impl Fn(&[Vec<u8>]) -> Vec<u8>) -> Result<[u8; 32], FoldV3Error> {
    let mut producer = object_state("countries", 2);
    let mut replica = object_state("countries", 2);
    let mut last = Vec::new();
    let mut expected = [0u8; 32];
    for json in HOURS {
        let delta = assign(&mut producer, json);
        last = delta.bytes;
        expected = delta.state_root;
        if json != HOURS[HOURS.len() - 1] {
            replica.apply_delta(&last).unwrap();
        }
    }
    let parts = records(&last);
    assert!(parts.len() >= 2, "the tamper fixture needs several records");
    replica.apply_delta(&tampered(&parts))?;
    assert_ne!(
        replica.state_root(),
        expected,
        "a tampered delta was accepted AND still reached the producer's root"
    );
    Ok(replica.state_root())
}

#[test]
fn a_dropped_mutation_fails() {
    // Dropping the leading record leaves bytes that still decode -- every
    // remaining record is well formed and in order -- so the decoder cannot be
    // what catches this. The state root is.
    tamper(|parts| parts[1..].concat()).expect("the remaining records still decode");
}

#[test]
fn a_dropped_mutation_that_decodes_still_changes_the_root() {
    // Dropping the LAST record leaves a prefix that decodes cleanly, so the only
    // thing that can catch it is the state root.
    let mut producer = object_state("countries", 2);
    let mut replica = object_state("countries", 2);
    let mut last = Vec::new();
    for json in HOURS {
        let delta = assign(&mut producer, json);
        last = delta.bytes.clone();
        if json != HOURS[HOURS.len() - 1] {
            replica.apply_delta(&last).unwrap();
        } else {
            let parts = records(&last);
            let truncated = parts[..parts.len() - 1].concat();
            replica.apply_delta(&truncated).expect("prefix decodes");
            assert_ne!(
                replica.state_root(),
                delta.state_root,
                "a delta short one record must not reach the producer's root"
            );
        }
    }
}

#[test]
fn a_reordered_delta_fails() {
    let outcome = tamper(|parts| {
        let mut swapped = parts.to_vec();
        swapped.swap(0, 1);
        swapped.concat()
    });
    assert!(outcome.is_err(), "records out of canonical order were accepted");
}

#[test]
fn a_duplicated_mutation_fails() {
    let outcome = tamper(|parts| {
        let mut doubled = parts.to_vec();
        doubled.insert(1, parts[0].clone());
        doubled.concat()
    });
    assert!(outcome.is_err(), "a duplicated record was accepted");
}

#[test]
fn a_truncated_delta_fails() {
    let outcome = tamper(|parts| {
        let mut bytes = parts.concat();
        bytes.truncate(bytes.len() - 1);
        bytes
    });
    assert!(outcome.is_err(), "a delta cut mid-value was accepted");
}

#[test]
fn a_corrupted_value_byte_changes_the_root() {
    let mut producer = object_state("countries", 2);
    let mut replica = object_state("countries", 2);
    let mut delta = assign(&mut producer, HOURS[0]);
    // The last byte of the last record is inside a grain's JSON body.
    let position = delta.bytes.len() - 2;
    delta.bytes[position] = if delta.bytes[position] == b'3' { b'4' } else { b'3' };
    let applied = replica.apply_delta(&delta.bytes);
    match applied {
        Err(_) => {}
        Ok(_) => assert_ne!(
            replica.state_root(),
            delta.state_root,
            "a corrupted value byte must not reach the producer's root"
        ),
    }
}

#[test]
fn a_stale_grain_reused_from_an_earlier_hour_changes_the_root() {
    // The failure the refused memoization design could not detect: a child whose
    // cached subtree is one hour old but internally consistent. Here it is spelled
    // as a replica that keeps hour 1's grain instead of taking hour 2's.
    let mut producer = object_state("countries", 2);
    let hour_one = assign(&mut producer, HOURS[0]);
    let root_one = hour_one.state_root;
    let hour_two = assign(&mut producer, HOURS[1]);

    let mut replica = object_state("countries", 2);
    replica.apply_delta(&hour_one.bytes).unwrap();
    // Apply everything of hour 2 except the record that refreshes `AAA/x`.
    let kept: Vec<Vec<u8>> = records(&hour_two.bytes)
        .into_iter()
        .filter(|record| {
            let mut reader = ByteReader::new(record);
            let op = reader.u8().unwrap();
            if op == OP_ROOT {
                return true;
            }
            let path = reader.path().unwrap();
            path != vec![Seg::Key("AAA".into()), Seg::Key("x".into())]
        })
        .collect();
    replica.apply_delta(&kept.concat()).unwrap();
    assert_ne!(replica.state_root(), hour_two.state_root);
    assert_ne!(replica.state_root(), root_one);
    assert_ne!(replica.project(), producer.project());
}

#[test]
fn a_non_minimal_varint_fails() {
    let mut state = object_state("countries", 2);
    let delta = assign(&mut state, HOURS[0]);
    // Re-encode the first record's path length as a two-byte varint.
    let mut bytes = Vec::new();
    bytes.push(delta.bytes[0]);
    bytes.push(0x80);
    bytes.push(0x00);
    bytes.extend_from_slice(&delta.bytes[2..]);
    let mut replica = object_state("countries", 2);
    assert!(replica.apply_delta(&bytes).is_err());
}

#[test]
fn an_unknown_opcode_fails() {
    let mut state = object_state("countries", 2);
    let mut delta = assign(&mut state, HOURS[0]);
    delta.bytes[0] = 99;
    let mut replica = object_state("countries", 2);
    assert!(replica.apply_delta(&delta.bytes).is_err());
}

#[test]
fn a_root_record_that_is_not_first_fails() {
    let mut replica = divisions_state(1);
    let mut bytes = Vec::new();
    bytes.push(OP_UPSERT);
    encode_path_into(&[Seg::Key("1:1".into())], &mut bytes);
    let body = br#"{"div_uid":"1:1"}"#;
    write_varint(&mut bytes, body.len() as u64);
    bytes.extend_from_slice(body);
    bytes.push(OP_ROOT);
    bytes.push(RootKind::Scalar.tag());
    assert!(replica.apply_delta(&bytes).is_err());
}

// ---------------------------------------------------------------------------
// Hour discipline
// ---------------------------------------------------------------------------

#[test]
fn dropping_an_hour_with_pending_mutations_poisons_the_section() {
    let mut state = object_state("countries", 2);
    {
        let value = parse(HOURS[0]);
        let mut hour = state.begin_hour().unwrap();
        hour.assign_section(&value).unwrap();
        // No commit. In production this is an hour whose mutations were written
        // into the state and never into any delta.
    }
    assert!(state.poisoned().is_some());
    assert!(state.begin_hour().is_err());
    assert!(state.apply_delta(&[]).is_err());
}

#[test]
fn an_hour_with_no_changes_commits_an_empty_delta() {
    let mut state = object_state("countries", 2);
    assign(&mut state, HOURS[0]);
    let delta = assign(&mut state, HOURS[0]);
    assert_eq!(delta.mutations, 0);
    assert!(delta.bytes.is_empty());
    // An unchanged hour is still a published hour, so the root must be the same
    // root and not a missing one.
    assert_eq!(delta.state_root, state.state_root());
}

#[test]
fn upsert_bytes_refuses_a_body_that_is_not_canonical_json() {
    let mut state = object_state("countries", 1);
    let mut hour = state.begin_hour().unwrap();
    assert!(hour
        .upsert_bytes(vec![Seg::Key("AAA".into())], b"{\"b\":1,\"a\":2}".to_vec())
        .is_err());
    assert!(hour
        .upsert_bytes(vec![Seg::Key("AAA".into())], b"{oops".to_vec())
        .is_err());
    hour.commit();
}

#[test]
fn a_no_op_upsert_is_counted_and_journals_nothing() {
    let mut state = object_state("countries", 1);
    assign(&mut state, r#"{"AAA":{"x":1}}"#);
    let value = parse(r#"{"x":1}"#);
    let mut hour = state.begin_hour().unwrap();
    assert!(!hour.upsert(vec![Seg::Key("AAA".into())], &value).unwrap());
    let delta = hour.commit();
    assert!(delta.bytes.is_empty());
    assert_eq!(state.stats().no_ops, 1);
}

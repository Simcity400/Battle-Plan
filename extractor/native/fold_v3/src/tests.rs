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

// ---------------------------------------------------------------------------
// The ORDINAL-addressed array root
// ---------------------------------------------------------------------------
//
// `Shape::Array` was written for completeness and, until lane EI's tranche 4,
// nothing used it: `divisions` is a `KeyedArray` and every other converted
// section is an `Object`. The census then found three array-spelled members --
// `combats`, `ended_combats`, `events` -- with no member that is a unique,
// already-ascending string identity on every hour, so they can only be
// addressed by ordinal. These tests are what makes that shape's root behaviour
// -- element prefixes written without a name, ordinals accepted at the root and
// names refused, density across growth and shrinkage -- something the code says
// rather than something the comments do.

fn array_state(name: &str, depth: usize) -> SectionState {
    SectionState::new(SectionSpec {
        name: name.to_string(),
        shape: Shape::Array,
        grain_depth: depth,
    })
}

#[test]
fn an_array_rooted_section_projects_back_at_every_grain_depth() {
    // Elements of mixed kind, an empty container among them, and a nested
    // array: the root's children are written WITHOUT a member prefix while
    // everything inside an object element keeps one, and `container_is_object`
    // is the only thing that knows the difference at depth 0.
    let json = r#"[{"hp":[1,2],"id":"a"},{"hp":[],"id":"b"},[],7,null,{"k":{"j":1}}]"#;
    for depth in 1..=5 {
        let mut state = array_state("combats", depth);
        assign(&mut state, json);
        assert_eq!(
            String::from_utf8(state.project()).unwrap(),
            json,
            "grain depth {depth} did not project an array root back byte-identically"
        );
        assert_eq!(state.root_kind(), RootKind::Array);
        assert_eq!(state.ordinal_density_error(), None, "grain depth {depth}");
    }
}

#[test]
fn an_empty_array_root_holds_no_grain_at_all() {
    // `events` and `ended_combats` are `[]` on almost every hour. An empty
    // container that stored a whole-section grain would put a value under a
    // container root -- the one path set the projection cannot write back.
    let mut state = array_state("events", 2);
    assign(&mut state, "[]");
    assert_eq!(String::from_utf8(state.project()).unwrap(), "[]");
    assert_eq!(state.grain_count(), 0);
    assert_eq!(state.root_kind(), RootKind::Array);
}

#[test]
fn an_array_root_addressed_by_name_is_refused() {
    // The root has no address reaching into it, so its addressing mode comes
    // from the shape alone. Without that arm the state would accept
    // `["x"]`-as-a-member and project `["x":...]`, which no parser accepts.
    let mut state = array_state("combats", 1);
    let mut hour = state.begin_hour().unwrap();
    assert!(hour
        .upsert(vec![Seg::Key("0".into())], &parse("1"))
        .is_err());
    assert!(hour.upsert(vec![Seg::Index(0)], &parse("1")).is_ok());
    hour.commit();
}

#[test]
fn an_array_root_that_grows_shrinks_and_mutates_in_place_replays() {
    // The three transitions an ordinal root has to survive, and the one the
    // census predicts will be expensive: an insertion at position 1 moves every
    // element after it, so the delta is proportional to the TAIL and not to the
    // change. That is priced, not avoided -- `combats` is 8.8 kB -- and pricing
    // it honestly means the shift has to actually happen here.
    let hours = [
        r#"[{"i":1},{"i":2},{"i":3}]"#,
        // Insert in the middle: ordinals 1 and 2 are rewritten and 3 is added.
        r#"[{"i":1},{"i":9},{"i":2},{"i":3}]"#,
        // Mutate the last element only: exactly one grain moves.
        r#"[{"i":1},{"i":9},{"i":2},{"i":4}]"#,
        // Shrink from the front: everything shifts down and the tail ordinal
        // is removed. A removal that stranded a later ordinal would leave the
        // state projecting elements at positions they are not addressed by,
        // and the replica's density check would refuse the delta.
        r#"[{"i":9},{"i":2},{"i":4}]"#,
        r#"[]"#,
        r#"[{"i":1}]"#,
    ];
    let deltas = replay(&hours, || array_state("combats", 1));

    // Hour 2: three records for a one-element insertion. The number is the
    // point -- it is what a keyed grain would have made one, and it is why the
    // census's ordinal density is reported as an artefact of the addressing.
    assert_eq!(deltas[1].mutations, 3);
    // Hour 3: a change confined to the last element moves one grain and no
    // other, which is what says the ordinal addressing is not re-emitting the
    // whole array on every hour either.
    assert_eq!(deltas[2].mutations, 1);
    // Hour 5: the array empties. Every ordinal is removed and the root stays an
    // array, so no root record is emitted.
    let ops: Vec<u8> = records(&deltas[4].bytes)
        .iter()
        .map(|record| record[0])
        .collect();
    assert_eq!(ops, vec![OP_REMOVE, OP_REMOVE, OP_REMOVE]);
}

#[test]
fn an_array_rooted_section_handed_a_non_array_becomes_a_scalar_root_and_comes_back() {
    // `combats` and `naval_combats` each flip kind once per capture. A section
    // is not its shape: the hour it is not an array it holds the whole member
    // at the empty path, and the root-kind record is what carries the
    // difference to a replica.
    let mut producer = array_state("combats", 1);
    let mut replica = array_state("combats", 1);
    for json in [r#"[{"i":1},{"i":2}]"#, "null", r#"[{"i":3}]"#, r#"{"a":1}"#, "[]"] {
        let delta = assign(&mut producer, json);
        replica.apply_delta(&delta.bytes).expect("replica applies");
        assert_eq!(
            String::from_utf8(producer.project()).unwrap(),
            json,
            "producer at {json}"
        );
        assert_eq!(replica.project(), producer.project(), "replica at {json}");
        assert_eq!(replica.state_root(), delta.state_root, "root at {json}");
    }
}

#[test]
fn a_scalar_null_root_holds_exactly_one_grain_and_not_nothing() {
    // `project_into` renders a scalar root with NO grain as `null`, which is the
    // right answer for a state nothing ever assigned and a trap for everything
    // downstream of it: a section that is `null` on every hour -- nine of the
    // document's members are never containers and five are `null` throughout --
    // projects the same four bytes whether it holds the grain or lost it. Every
    // byte-comparing check therefore passes over the loss, so the grain SET has
    // to be asserted directly and here.
    for shape in [
        Shape::Object,
        Shape::Array,
        Shape::KeyedArray {
            key_member: "div_uid".to_string(),
        },
    ] {
        let mut state = SectionState::new(SectionSpec {
            name: "save_version".to_string(),
            shape: shape.clone(),
            grain_depth: 1,
        });
        assign(&mut state, "null");
        assert_eq!(state.root_kind(), RootKind::Scalar, "{shape:?}");
        assert_eq!(
            state.grain_count(),
            1,
            "a scalar-null root that holds nothing projects `null` all the same ({shape:?})"
        );
        assert!(state.grain(&[]).is_some(), "{shape:?}");
        assert_eq!(String::from_utf8(state.project()).unwrap(), "null");
    }
}

#[test]
fn a_state_that_never_assigned_its_scalar_is_not_the_state_that_assigned_null() {
    // The two states `project` cannot distinguish must at least differ in their
    // ROOT, or a dropped empty-path upsert is invisible to every check a driver
    // can make.
    let mut assigned = object_state("save_version", 1);
    assign(&mut assigned, "null");
    let mut never = object_state("save_version", 1);
    // Same root KIND, no grain: reached by moving the kind and assigning nothing.
    let mut hour = never.begin_hour().unwrap();
    hour.set_root_kind(RootKind::Scalar).unwrap();
    hour.commit();

    assert_eq!(assigned.project(), never.project(), "both project `null`");
    assert_eq!(assigned.root_kind(), never.root_kind());
    assert_ne!(
        assigned.state_root(),
        never.state_root(),
        "a lost scalar grain must move the state root even though it cannot move the bytes"
    );
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
        hour.upsert(vec![Seg::Key(escaped_key.as_str().into())], &other)
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
        hour.upsert(vec![Seg::Key(escaped_key.as_str().into())], &other)
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
        .upsert(vec![Seg::Key(QUOTE.into())], &value)
        .is_err());
    // A lone trailing backslash is not a valid escape either.
    assert!(hour
        .upsert(vec![Seg::Key(ESCAPE.into())], &value)
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

#[test]
fn a_revision_overflow_empties_the_section_it_poisons() {
    // The counter can only fail at `u64::MAX`, which is why nothing had ever
    // exercised it -- and why it was covered for by accident: the primitives
    // happened to record an hour baseline before the bump, so the dropped
    // transaction found pending changes and emptied the section. `erase` takes
    // its baseline AFTER the bump now, and a section that is poisoned but not
    // emptied keeps serving the grains it is refusing to stand behind, with a
    // state root that still agrees with a full recomputation.
    for erase_it in [false, true] {
        let mut state = object_state("countries", 1);
        assign(&mut state, r#"{"AAA":{"x":1}}"#);
        // Reaching `u64::MAX` through the API would take 2^64 mutations. The
        // tests are inside the crate, so the counter is reachable directly.
        state.revision = u64::MAX;
        let value = parse(r#"{"x":2}"#);
        {
            let mut hour = state.begin_hour().unwrap();
            let outcome = if erase_it {
                hour.remove(&[Seg::Key("AAA".into())]).map(|_| ())
            } else {
                hour.upsert(vec![Seg::Key("AAA".into())], &value).map(|_| ())
            };
            assert!(outcome.is_err(), "the counter must refuse to wrap");
            hour.commit();
        }
        assert!(state.poisoned().is_some(), "erase_it={erase_it}");
        assert_eq!(
            state.grain_count(),
            0,
            "a poisoned section serves nothing (erase_it={erase_it})"
        );
        assert_eq!(state.state_root(), state.recomputed_state_root());
        assert!(state.begin_hour().is_err());
    }
}

// ---------------------------------------------------------------------------
// Ordinal density
// ---------------------------------------------------------------------------

/// Hand-encode one upsert record. The delta encoder only ever emits shapes the
/// producer can reach, so a test about shapes it cannot reach has to write the
/// bytes itself.
fn upsert_record(path: &[Seg], body: &[u8]) -> Vec<u8> {
    let mut bytes = vec![OP_UPSERT];
    encode_path_into(path, &mut bytes);
    write_varint(&mut bytes, body.len() as u64);
    bytes.extend_from_slice(body);
    bytes
}

fn remove_record(path: &[Seg]) -> Vec<u8> {
    let mut bytes = vec![OP_REMOVE];
    encode_path_into(path, &mut bytes);
    bytes
}

#[test]
fn a_delta_that_installs_an_ordinal_with_no_predecessor_is_refused() {
    // `project` writes an array's elements in ascending-ordinal order and never
    // writes the ordinals, so a state holding only `[1]` under `a` emits
    // `{"a":[..]}` with that element at position 0 -- a different document from
    // the one the state's own paths describe. The producer cannot reach this
    // shape; a delta from outside can, and it is the decoder's job to stop it.
    let mut replica = object_state("map", 2);
    let path = vec![Seg::Key("a".into()), Seg::Index(1)];
    let error = replica
        .apply_delta(&upsert_record(&path, b"7"))
        .expect_err("a sparse ordinal must be refused");
    assert!(
        error.message().contains("ordinal 1"),
        "unexpected message: {}",
        error.message()
    );
    // Refusing part-way through poisons, which is the existing contract.
    assert!(replica.poisoned().is_some());

    // Ordinal 0 first, then 1, is the same two records in a legal order.
    let mut replica = object_state("map", 2);
    let mut delta = upsert_record(&[Seg::Key("a".into()), Seg::Index(0)], b"6");
    delta.extend_from_slice(&upsert_record(&path, b"7"));
    replica.apply_delta(&delta).expect("dense ordinals apply");
    assert_eq!(String::from_utf8(replica.project()).unwrap(), r#"{"a":[6,7]}"#);
    assert_eq!(replica.ordinal_density_error(), None);
}

#[test]
fn a_delta_that_removes_an_ordinal_from_the_middle_is_refused() {
    let mut producer = object_state("map", 2);
    let delta = assign(&mut producer, r#"{"a":[1,2,3]}"#);
    let mut replica = object_state("map", 2);
    replica.apply_delta(&delta.bytes).expect("replica applies");

    let error = replica
        .apply_delta(&remove_record(&[Seg::Key("a".into()), Seg::Index(1)]))
        .expect_err("a hole in the middle of an array must be refused");
    assert!(
        error.message().contains("ordinal 1"),
        "unexpected message: {}",
        error.message()
    );
}

#[test]
fn a_gap_inside_an_array_of_objects_is_refused_even_though_no_path_ends_in_it() {
    // The ordinals of an array whose elements are OBJECTS sit in the middle of
    // its grains' paths -- `[a, 1, x]`, not `[a, 1]`. A check that looked only
    // at each path's final segment would see a member name there and wave the
    // gap through, and this is the shape real captures are full of.
    let mut replica = object_state("map", 3);
    let path = vec![Seg::Key("a".into()), Seg::Index(1), Seg::Key("x".into())];
    let error = replica
        .apply_delta(&upsert_record(&path, b"7"))
        .expect_err("a sparse ordinal one level up must still be refused");
    assert!(
        error.message().contains("ordinal 1"),
        "unexpected message: {}",
        error.message()
    );

    // The same shape with element 0 present is legal and projects as an array.
    let mut replica = object_state("map", 3);
    let mut delta = upsert_record(
        &[Seg::Key("a".into()), Seg::Index(0), Seg::Key("x".into())],
        b"6",
    );
    delta.extend_from_slice(&upsert_record(&path, b"7"));
    replica.apply_delta(&delta).expect("dense ordinals apply");
    assert_eq!(
        String::from_utf8(replica.project()).unwrap(),
        r#"{"a":[{"x":6},{"x":7}]}"#
    );
    assert_eq!(replica.ordinal_density_error(), None);

    // And removing the FIRST element of a two-element array strands the second.
    let error = replica
        .apply_delta(&remove_record(&[
            Seg::Key("a".into()),
            Seg::Index(0),
            Seg::Key("x".into()),
        ]))
        .expect_err("stranding element 1 must be refused");
    assert!(
        error.message().contains("ordinal 0"),
        "unexpected message: {}",
        error.message()
    );
}

#[test]
fn the_whole_state_density_scan_agrees_with_the_incremental_check() {
    // The incremental check in `apply_delta` only looks at what the delta
    // touched, which is only sound because the state was dense before it. This
    // is the reference it is measured against: build the sparse state through
    // the producer API (which does not check density) and require the whole-state
    // scan to name it.
    let mut state = object_state("map", 2);
    let value = parse("7");
    {
        let mut hour = state.begin_hour().unwrap();
        hour.upsert(vec![Seg::Key("a".into()), Seg::Index(1)], &value)
            .expect("the producer API does not check density");
        hour.commit();
    }
    let complaint = state
        .ordinal_density_error()
        .expect("the scan must name the hole");
    assert!(complaint.contains("to 1"), "unexpected: {complaint}");
    // And the projection is exactly the silent wrongness the check exists for:
    // one element addressed at 1 and written at 0.
    assert_eq!(String::from_utf8(state.project()).unwrap(), r#"{"a":[7]}"#);
}

#[test]
fn an_array_that_shrinks_passes_through_a_sparse_intermediate_and_is_still_accepted() {
    // A delta's removals ascend, so `[1,2,3]` becoming `[9]` removes ordinal 1
    // and leaves ordinal 2 standing until the next record. Checking density per
    // RECORD rather than per DELTA would refuse this hour, which the producer
    // emits on real data every time an array loses its tail.
    let mut producer = object_state("map", 2);
    let mut replica = object_state("map", 2);
    for json in [
        r#"{"a":[1,2,3]}"#,
        r#"{"a":[9]}"#,
        r#"{"a":[9,8,7,6]}"#,
        r#"{"a":[]}"#,
        r#"{"a":[5,4]}"#,
    ] {
        let delta = assign(&mut producer, json);
        replica
            .apply_delta(&delta.bytes)
            .unwrap_or_else(|error| panic!("replica refused a legal hour at {json}: {error}"));
        assert_eq!(String::from_utf8(replica.project()).unwrap(), json);
        assert_eq!(replica.state_root(), delta.state_root);
        assert_eq!(producer.ordinal_density_error(), None, "at {json}");
    }
}

// ---------------------------------------------------------------------------
// What the hot path is allowed to skip
// ---------------------------------------------------------------------------

#[test]
fn the_cached_contribution_tracks_the_grain_it_is_stored_with() {
    // The state root retracts a grain using the contribution the grain carries
    // rather than re-hashing its path and digest. A cache computed from the
    // wrong path, from a superseded digest, or not replaced when the grain is
    // replaced, leaves a root that is stable, self-consistent and wrong -- and
    // the only thing that says so is a full recomputation.
    let escaped = format!("{ESCAPE}u0061");
    let value = parse("1");
    let other = parse("2");
    let mut state = object_state("countries", 1);
    let steps: Vec<Box<dyn Fn(&mut HourTxn<'_>)>> = vec![
        Box::new(move |hour| {
            hour.upsert(vec![Seg::Key("a".into())], &value).unwrap();
        }),
        // Same address, new bytes: the retraction must use the OLD digest.
        Box::new(move |hour| {
            hour.upsert(vec![Seg::Key("a".into())], &other).unwrap();
        }),
        // Same address, new spelling: the retraction must use the OLD spelling,
        // which is inside the hash.
        Box::new({
            let escaped = escaped.clone();
            let value = parse("2");
            move |hour| {
                hour.upsert(vec![Seg::Key(escaped.as_str().into())], &value).unwrap();
            }
        }),
        // Gone: the retraction has to cancel the contribution exactly.
        Box::new(|hour| {
            hour.remove(&[Seg::Key("a".into())]).unwrap();
        }),
    ];
    for (index, step) in steps.iter().enumerate() {
        let mut hour = state.begin_hour().unwrap();
        step(&mut hour);
        hour.commit();
        assert_eq!(
            state.state_root(),
            state.recomputed_state_root(),
            "the cached contribution disagreed with a fresh one after step {index}"
        );
    }
    assert_eq!(state.grain_count(), 0);
    assert_eq!(state.state_root(), object_state("countries", 1).state_root());
}

#[test]
fn a_path_check_skipped_for_a_standing_address_is_not_skipped_for_a_new_one() {
    // `install` does not re-run `check_path` for an address that already stands
    // under the same spelling, because the path SET it validates has not moved.
    // The skip is per ADDRESS: a second write to `[a,b]` is waved through, and a
    // first write to `[a]` -- which would put a value where a container stands --
    // still has to be refused.
    let mut state = object_state("countries", 3);
    let value = parse("1");
    let other = parse("2");
    let mut hour = state.begin_hour().unwrap();
    hour.upsert(vec![Seg::Key("a".into()), Seg::Key("b".into())], &value)
        .unwrap();
    hour.commit();

    let mut hour = state.begin_hour().unwrap();
    assert!(hour
        .upsert(vec![Seg::Key("a".into()), Seg::Key("b".into())], &other)
        .expect("a standing address takes new bytes"));
    assert!(
        hour.upsert(vec![Seg::Key("a".into())], &value).is_err(),
        "a new address that collides with a standing grain must still be refused"
    );
    assert!(
        hour.upsert(vec![Seg::Key("a".into()), Seg::Index(0)], &value)
            .is_err(),
        "a new sibling that disagrees on addressing must still be refused"
    );
    hour.commit();
    assert_eq!(
        String::from_utf8(state.project()).unwrap(),
        r#"{"a":{"b":2}}"#
    );
}

// ---------------------------------------------------------------------------
// Interning
// ---------------------------------------------------------------------------

/// Every member-name body these tests and the corpus can present, chosen so the
/// comparison head is exercised where it decides and where it must not.
///
/// The head is the first EIGHT bytes of the ordering text, so the interesting
/// pairs are the ones that agree on all eight and differ later, the ones where
/// one name is a prefix of the other on either side of that boundary, and the
/// escaped names -- for which the head is not consulted at all, because their
/// ordering text is not their body.
fn ordering_fixture() -> Vec<String> {
    let escape = ESCAPE;
    vec![
        String::new(),
        "a".to_string(),
        "ab".to_string(),
        "b".to_string(),
        // Eight bytes exactly, and its own extensions -- the boundary the head
        // is cut at.
        "12345678".to_string(),
        "123456789".to_string(),
        "12345678a".to_string(),
        "1234567".to_string(),
        // A shared nine-byte prefix: the head cannot decide either of these and
        // has to fall through to the text.
        "prefix_aa_1".to_string(),
        "prefix_aa_2".to_string(),
        "prefix_aa".to_string(),
        // The corpus's real shapes: division uids and country tags, whose order
        // is a STRING order and not a numeric one.
        "10:1".to_string(),
        "2:1".to_string(),
        "100:12".to_string(),
        "GER".to_string(),
        "SOV".to_string(),
        // Escaped, and decoding to something that sorts nowhere near its
        // spelling: `a` is `a`, whose head would start with a backslash if
        // the head were taken from the body.
        format!("{escape}u0061"),
        format!("{escape}u0062"),
        format!("{escape}n"),
        format!("{escape}{escape}"),
        format!("a{escape}{QUOTE}b"),
        // Escaped and UNDECODABLE: a lone high surrogate. `key_cmp` falls back
        // to comparing raw bodies whenever either side is one of these, and so
        // must the interned comparison.
        format!("{escape}ud800"),
        format!("{escape}ud801"),
        // Non-ASCII, so the head carries UTF-8 continuation bytes.
        "caf\u{e9}".to_string(),
        "caf\u{e9}_long_enough_to_pass_the_head".to_string(),
        "\u{4e2d}\u{6587}".to_string(),
    ]
}

/// The interned comparison IS `key_cmp`, on every pair of the fixture.
///
/// This is the whole safety argument for the comparison head and for the
/// pointer-equality fast path. A head computed from the body instead of the
/// ordering text, a head compared as little-endian, a head trusted between an
/// escaped name and a plain one, or a fast path that returned `Equal` for two
/// different entries would all show up here as a pair that orders differently
/// from the function the projection and the delta encoding are defined against.
#[test]
fn the_interned_order_is_key_cmp_on_every_pair() {
    let names = ordering_fixture();
    for left in &names {
        for right in &names {
            let expected = snapshot::canon_value::key_cmp(left, right);
            let interned = Seg::key(left).cmp(&Seg::key(right));
            assert_eq!(
                interned, expected,
                "{left:?} vs {right:?}: interned said {interned:?}, key_cmp said {expected:?}"
            );
        }
    }
}

/// The fixture minus the bodies that do not decode.
///
/// `key_cmp` is not TRANSITIVE across an undecodable name, and never was: with
/// `U` a lone surrogate escape, `D` an escape decoding to `a` and `P` the plain
/// `a`, it reports `D == P`, `D < U` (both bodies decodable-or-not, compared as
/// text) and `P > U` (compared as raw bodies, where `a` follows a backslash).
/// That is a property of the production function this crate is defined against,
/// it is identical before and after interning -- the pairwise test above holds
/// on exactly those pairs -- and it is unreachable in a state, because
/// `check_path` refuses an undecodable member name outright. Sorting is
/// therefore asserted on the population that can actually reach a path set.
fn sortable_ordering_fixture() -> Vec<String> {
    ordering_fixture()
        .into_iter()
        .filter(|body| snapshot::canon_value::is_valid_object_key(body))
        .collect()
}

/// Sorting a path set through the interned order gives the same sequence
/// `key_cmp` does -- which is the order `project` writes and the order a delta's
/// records must ascend in.
#[test]
fn sorting_paths_agrees_with_the_uninterned_order() {
    let names = sortable_ordering_fixture();
    assert!(names.len() > 20, "the sortable fixture lost its coverage");
    let mut interned: Vec<Seg> = names.iter().map(|name| Seg::key(name)).collect();
    interned.sort();
    let mut expected = names.clone();
    expected.sort_by(|left, right| snapshot::canon_value::key_cmp(left, right));
    let interned: Vec<String> = interned
        .iter()
        .map(|seg| match seg {
            Seg::Key(name) => name.body().to_string(),
            Seg::Index(_) => unreachable!("the fixture holds no ordinals"),
        })
        .collect();
    assert_eq!(interned, expected);
}

/// One body has exactly one entry, which is what makes pointer equality an
/// exact answer to "same spelling" rather than an optimistic one.
#[test]
fn one_body_interns_to_one_entry() {
    for name in ordering_fixture() {
        let first = Seg::key(&name);
        let second = Seg::key(&name.clone());
        assert!(
            same_spelling(&[first], &[second]),
            "{name:?} interned to two entries"
        );
        assert_eq!(first.cmp(&second), std::cmp::Ordering::Equal);
    }
    // And two DIFFERENT bodies never share one, however close their heads.
    assert!(!same_spelling(
        &[Seg::key("prefix_aa_1")],
        &[Seg::key("prefix_aa_2")]
    ));
}

/// A section that has already taken the one-descent install path must still
/// rekey when an escaped respelling finally arrives.
///
/// `install` routes to a path that gives the incoming address to
/// `BTreeMap::entry` and therefore cannot replace a stored key, and it is
/// allowed to do that only while no escaped name has been seen. If that guard
/// were wrong -- read from the stored side only, or reset between hours -- the
/// projection would keep writing the OLD spelling while every digest agreed,
/// which is precisely the failure the guard exists to prevent.
#[test]
fn a_section_that_took_the_fast_path_still_rekeys_later() {
    let escaped_key = format!("{ESCAPE}u0061");
    let mut producer = object_state("countries", 1);
    let mut replica = object_state("countries", 1);
    // Several hours of plain, unrespellable addresses first, so the fast path
    // is what installed the grain that is about to be respelled.
    for json in [r#"{"a":1}"#, r#"{"a":2}"#, r#"{"a":3,"b":1}"#] {
        let delta = assign(&mut producer, json);
        replica.apply_delta(&delta.bytes).unwrap();
        assert_eq!(replica.project(), producer.project());
    }
    let delta = {
        let value = parse("4");
        let mut hour = producer.begin_hour().unwrap();
        hour.upsert(vec![Seg::Key(escaped_key.as_str().into())], &value)
            .unwrap();
        hour.commit()
    };
    replica.apply_delta(&delta.bytes).expect("replica applies");
    assert_eq!(
        String::from_utf8(producer.project()).unwrap(),
        format!("{{{QUOTE}{escaped_key}{QUOTE}:4,{QUOTE}b{QUOTE}:1}}"),
        "the stored spelling was not replaced"
    );
    assert_eq!(replica.project(), producer.project());
    assert_eq!(replica.state_root(), delta.state_root);
    assert_eq!(producer.state_root(), producer.recomputed_state_root());
    assert_eq!(producer.grain_count(), 2, "one address, not two");
}

/// An hour that removes one address and rewrites another emits its removal
/// before its upsert, and every record of each phase ascends.
///
/// The commit walks its baseline ONCE and builds the two phases into two
/// buffers, where it used to walk it twice. If the buffers were concatenated in
/// the wrong order, or an upsert leaked into the removal buffer, the strict
/// decoder would reject the delta -- so this asserts the decoder accepted it AND
/// that the bytes begin with the removal.
#[test]
fn one_hour_emits_removals_before_upserts_in_one_pass() {
    let mut producer = object_state("countries", 1);
    let mut replica = object_state("countries", 1);
    let first = assign(&mut producer, r#"{"a":1,"b":2,"c":3}"#);
    replica.apply_delta(&first.bytes).unwrap();

    // `b` goes, `a` changes, `d` arrives: all three phases' worth of work in
    // one hour, presented in an order that is not the emitted one.
    let delta = {
        let mut hour = producer.begin_hour().unwrap();
        hour.upsert(vec![Seg::Key("d".into())], &parse("4")).unwrap();
        hour.upsert(vec![Seg::Key("a".into())], &parse("9")).unwrap();
        hour.remove(&[Seg::Key("b".into())]).unwrap();
        hour.commit()
    };
    assert_eq!(delta.bytes.first(), Some(&OP_REMOVE), "removals come first");
    assert_eq!(delta.mutations, 3);
    replica.apply_delta(&delta.bytes).expect("replica applies");
    assert_eq!(replica.state_root(), delta.state_root);
    assert_eq!(replica.project(), producer.project());
    assert_eq!(
        String::from_utf8(producer.project()).unwrap(),
        r#"{"a":9,"c":3,"d":4}"#
    );
}

/// The interner accounts for what it holds, so the memory a path no longer
/// spends on its own copy of a name is reported where it went.
#[test]
fn the_interner_accounts_for_the_names_it_holds() {
    // A body no other test can have interned.
    let name = "lane_fb_accounting_probe_0123456789";
    let before = intern::retained_bytes();
    let names_before = intern::interned_names();
    let first = Seg::key(name);
    let after = intern::retained_bytes();
    // ONE-SIDED, because the interner is process-global and every test running
    // beside this one is interning into it. `>=` is the whole of what this test
    // can honestly say about a shared counter: an equality here reads as an
    // assertion about the name and is in fact an assertion about which other
    // tests the harness happened to schedule. It was one, and the tests added
    // for the dense-slot store -- which intern thousands of names -- are what
    // made it fail.
    assert!(
        intern::interned_names() > names_before,
        "interning {name:?} did not add an entry"
    );
    assert!(
        after >= before + name.len() as u64,
        "interning {name:?} added {} bytes",
        after - before
    );
    // Interning it again is free: one body, one entry. Asserted as the entry
    // IDENTITY rather than as a global count that stands still, which is the
    // fact the counts were standing in for. `one_body_interns_to_one_entry`
    // makes the same point about the table.
    let again = Seg::key(name);
    match (first, again) {
        (Seg::Key(first), Seg::Key(again)) => assert!(
            first.same_spelling(&again),
            "one body interned to two entries"
        ),
        _ => unreachable!("Seg::key builds a Key"),
    }
    // An escaped body whose decoding FAILS, which is the case the charge got
    // wrong: it is escaped, so a test that asked "was it escaped" would charge
    // for an ordering text; but the decode returned nothing, so the ordering
    // text IS the body, one string, and charging for both overstates the
    // interner by a body length. A lone surrogate is the shortest way to write
    // an escape that cannot decode. Interned before the assertion below, so
    // there is such an entry in the table for it to find.
    let _ = Seg::key(&format!("{ESCAPE}ud800_lane_fl_undecodable_probe"));
    // And the counter is what the entries add up to.
    //
    // This is the assertion the one-sided deltas above cannot make: a charge
    // added on the path that returns an ALREADY interned name -- which the two
    // calls to `Seg::key` just took -- would make the total drift above the
    // vocabulary, and a charge removed would make it drift below, and a running
    // total that is only ever compared against itself would report both as
    // progress. Recomputed inside one acquisition of the interner's lock, so
    // the comparison is not racing every other test in this file.
    assert_eq!(
        intern::accounting_disagreement(),
        None,
        "the interner's byte counter is not the sum of what it holds"
    );
}

/// A path costs its own segment array and nothing per name, because the names
/// live in the interner.
#[test]
fn a_paths_retained_bytes_are_its_segments() {
    let mut state = object_state("countries", 1);
    assign(&mut state, r#"{"a_very_long_member_name_indeed":1,"b":2}"#);
    assert_eq!(
        state.retained_path_bytes(),
        2 * std::mem::size_of::<Seg>() as u64,
        "a name's bytes must not be charged to every path that mentions it"
    );
}

// ---------------------------------------------------------------------------
// The dense-slot store: the order it iterates in, and what it does with a slot
// ---------------------------------------------------------------------------

/// A deterministic 64-bit stream (SplitMix64), so a failure is reproducible
/// from the seed in the test rather than from a lucky rerun.
struct Rng(u64);

impl Rng {
    fn next_word(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut word = self.0;
        word = (word ^ (word >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        word = (word ^ (word >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        word ^ (word >> 31)
    }
    fn below(&mut self, bound: u64) -> u64 {
        self.next_word() % bound
    }
}

/// A member name for address `index`. Decimal, so the STRING order these are
/// held in disagrees with the numeric order they are generated in -- which is
/// the order the store has to reproduce and not the one it would fall into.
fn churn_key(index: u64) -> String {
    format!("k{index}")
}

/// The store's iteration order is `<[Seg]>::cmp` ascending -- the `BTreeMap`
/// order -- through every insertion, removal, reinstallation and merge.
///
/// This is the property the dense-slot rewrite rests on and the one nothing
/// else in this file can see: the projection, the delta encoding and
/// `check_path`'s neighbour tests all read the order, and a store that got it
/// wrong would still agree with itself. So the order is compared against an
/// actual `BTreeMap` driven by the same operations, after EVERY operation, over
/// a state big enough to cross both merge thresholds many times.
#[test]
fn the_store_iterates_in_the_btreemap_order_through_churn() {
    let mut rng = Rng(0x5eed_1234_abcd_0001);
    let mut state = object_state("countries", 1);
    let mut shadow: BTreeMap<Path, Vec<u8>> = BTreeMap::new();
    // 600 distinct addresses against a merge floor of 64: the pending array is
    // merged repeatedly, and so are the tombstones.
    const ADDRESSES: u64 = 600;
    {
        let mut hour = state.begin_hour().expect("hour opens");
        for step in 0..12_000u64 {
            let index = rng.below(ADDRESSES);
            let path: Path = vec![Seg::key(&churn_key(index))];
            if rng.below(3) == 0 {
                hour.remove(&path).expect("removal is legal");
                shadow.remove(&path);
            } else {
                let value = parse(&format!("{}", rng.below(1_000_000)));
                let mut bytes = Vec::new();
                value.write_json(&mut bytes);
                hour.upsert(path.clone(), &value).expect("upsert is legal");
                shadow.insert(path, bytes);
            }
            // Every step, not every hundredth: a store that drifts and drifts
            // back between samples was wrong in between.
            let observed: Vec<&Path> = hour.state().grain_paths().collect();
            assert_eq!(
                observed.len(),
                shadow.len(),
                "step {step}: the store holds {} addresses, the map holds {}",
                observed.len(),
                shadow.len()
            );
            assert!(
                observed
                    .iter()
                    .zip(shadow.keys())
                    .all(|(left, right)| *left == right),
                "step {step}: the store's order is not the map's"
            );
            assert_eq!(hour.state().grain_count(), shadow.len(), "step {step}");
        }
        hour.commit();
    }
    // And the bytes that order produces are the bytes the same grains in a
    // `BTreeMap` produce.
    let mut expected = vec![b'{'];
    for (index, (path, bytes)) in shadow.iter().enumerate() {
        if index > 0 {
            expected.push(b',');
        }
        let Seg::Key(name) = path[0] else {
            unreachable!("every address here is a member name")
        };
        expected.push(b'"');
        expected.extend_from_slice(name.body().as_bytes());
        expected.extend_from_slice(b"\":");
        expected.extend_from_slice(bytes);
    }
    expected.push(b'}');
    assert_eq!(state.project(), expected, "the projection is not the map's");
    assert_eq!(state.state_root(), state.recomputed_state_root());
}

/// A replica driven only by the deltas of a churning section stays byte
/// identical to the producer, hour after hour.
///
/// The order test above compares the producer against a map. This compares it
/// against an independent state that has seen nothing but the encoded deltas,
/// so it is the one that can see a removal encoded under the wrong spelling, an
/// upsert emitted for a grain that did not move, or a phase boundary the strict
/// decoder rejects -- none of which the order alone can show.
#[test]
fn a_churning_section_replicates_hour_after_hour() {
    let mut rng = Rng(0x5eed_1234_abcd_0002);
    let mut producer = object_state("countries", 1);
    let mut replica = object_state("countries", 1);
    const ADDRESSES: u64 = 400;
    for hour_index in 0..40u64 {
        let delta = {
            let mut hour = producer.begin_hour().expect("hour opens");
            for _ in 0..500 {
                let path: Path = vec![Seg::key(&churn_key(rng.below(ADDRESSES)))];
                if rng.below(3) == 0 {
                    hour.remove(&path).expect("removal is legal");
                } else {
                    let value = parse(&format!("{}", rng.below(1_000_000)));
                    hour.upsert(path, &value).expect("upsert is legal");
                }
            }
            hour.commit()
        };
        replica.apply_delta(&delta.bytes).unwrap_or_else(|error| {
            panic!("hour {hour_index}: replica rejects the delta: {error}")
        });
        assert_eq!(replica.state_root(), delta.state_root, "hour {hour_index}");
        assert_eq!(replica.project(), producer.project(), "hour {hour_index}");
        assert_eq!(
            producer.state_root(),
            producer.recomputed_state_root(),
            "hour {hour_index}"
        );
    }
}

/// An address removed and reinstalled comes back as itself, not as whatever the
/// slot it landed in used to hold.
///
/// A dense store recycles slots, and a slot brought back into service while the
/// sorted order still named it would sit in that order under its OLD path. The
/// store therefore releases a slot only once no order array names it. This
/// drives the exact sequence that would break the rule -- remove, install a
/// DIFFERENT address, reinstall the removed one -- inside one hour and across
/// thirty of them, and reads the order back each round.
#[test]
fn a_removed_address_reinstalled_does_not_inherit_a_slot() {
    let mut state = object_state("countries", 1);
    let mut shadow: BTreeMap<Path, ()> = BTreeMap::new();
    for round in 0..30u64 {
        {
            let mut hour = state.begin_hour().expect("hour opens");
            for index in 0..40u64 {
                let path: Path = vec![Seg::key(&churn_key(index))];
                hour.upsert(path.clone(), &parse(&format!("{}", round * 100 + index)))
                    .expect("upsert is legal");
                shadow.insert(path, ());
            }
            // Retire every third address, install a fresh one in its place, and
            // bring the retired one back -- all inside this hour, so the freed
            // slot and the new address exist at once.
            for index in (0..40u64).step_by(3) {
                let gone: Path = vec![Seg::key(&churn_key(index))];
                assert!(hour.remove(&gone).expect("removal is legal"));
                shadow.remove(&gone);
                let fresh: Path = vec![Seg::key(&churn_key(1_000 + round * 40 + index))];
                hour.upsert(fresh.clone(), &parse("7")).expect("upsert");
                shadow.insert(fresh, ());
                hour.upsert(gone.clone(), &parse("8")).expect("upsert");
                shadow.insert(gone, ());
            }
            let observed: Vec<&Path> = hour.state().grain_paths().collect();
            assert!(
                observed.len() == shadow.len()
                    && observed
                        .iter()
                        .zip(shadow.keys())
                        .all(|(left, right)| *left == right),
                "round {round}: a reinstalled address did not come back at its own place"
            );
            // Retire the fresh ones again, so the next round opens with a free
            // list the order arrays have already released.
            for index in (0..40u64).step_by(3) {
                let fresh: Path = vec![Seg::key(&churn_key(1_000 + round * 40 + index))];
                assert!(hour.remove(&fresh).expect("removal is legal"));
                shadow.remove(&fresh);
            }
            hour.commit();
        }
        assert_eq!(
            state.state_root(),
            state.recomputed_state_root(),
            "round {round}"
        );
    }
    let observed: Vec<&Path> = state.grain_paths().collect();
    assert!(observed
        .iter()
        .zip(shadow.keys())
        .all(|(left, right)| *left == right));
    assert_eq!(state.grain_count(), shadow.len());
}

/// Two spellings of one member name find one another in the index.
///
/// The store hashes the ADDRESS -- the name's decoded ordering text -- and not
/// the spelling, because a hash taken over the spelling would put `a` and the
/// escaped spelling of `a` in different buckets, and the store would then hold
/// one address twice: two grains, two contributions in the state root, and a
/// projection with a duplicate member. The order tests above cannot see this,
/// because no escaped name appears in them.
#[test]
fn a_respelled_address_lands_in_the_same_bucket() {
    let escaped = format!("{ESCAPE}u0061");
    let mut state = object_state("countries", 1);
    let delta = {
        let mut hour = state.begin_hour().expect("hour opens");
        // Enough plain neighbours that the index has grown and rehashed several
        // times, so this is not a question answered by an empty table.
        for index in 0..200u64 {
            hour.upsert(vec![Seg::key(&churn_key(index))], &parse("1"))
                .expect("upsert");
        }
        hour.upsert(vec![Seg::key("a")], &parse("1"))
            .expect("upsert");
        assert_eq!(hour.state().grain_count(), 201);
        // The SAME address, spelled the other way.
        assert!(
            hour.state().grain_entry(&[Seg::key(&escaped)]).is_some(),
            "the escaped spelling did not find the grain the plain one installed"
        );
        hour.upsert(vec![Seg::key(&escaped)], &parse("2"))
            .expect("upsert");
        assert_eq!(
            hour.state().grain_count(),
            201,
            "a respelling installed a second grain at one address"
        );
        hour.commit()
    };
    let mut replica = object_state("countries", 1);
    replica.apply_delta(&delta.bytes).expect("replica applies");
    assert_eq!(replica.project(), state.project());
    assert_eq!(state.state_root(), state.recomputed_state_root());
    assert_eq!(replica.state_root(), delta.state_root);
    let projected = String::from_utf8(state.project()).expect("utf-8");
    assert!(
        projected.contains(&format!("{QUOTE}{escaped}{QUOTE}:2")),
        "the stored spelling was not replaced: {projected}"
    );
}

/// A section that retires an address and installs another reuses the row,
/// rather than growing its slot table once per address it has ever held.
///
/// This is the one property of the dense store that every other test is blind
/// to. A `compact` that stopped returning its tombstoned slots to the free list
/// would leave the iteration order, the projection, the state root and the
/// replica all correct and all agreeing -- and would leak one row, with its
/// path allocation, for every address ever removed. On a capture whose busiest
/// section retires ~1,925 addresses an hour that is the difference between a
/// bounded state and one that grows for the length of the campaign.
///
/// So the table is measured across six cycles of "retire the whole set in one
/// hour, install a fresh set of the same size in the next". Committing an hour
/// merges the store, so the retirement hour ends with every row reclaimed and
/// the installation hour has a free list to draw the whole set from: the table
/// is EXACTLY the set size, at every cycle, with no dependence on where the
/// merge thresholds happen to sit.
///
/// The row count alone would not be enough. A leak that stranded the first
/// cohort and then reclaimed correctly would settle at twice the set size and
/// hold there, which a plateau test would accept. So the assertion is the
/// IDENTITY `rows == live + tombstoned + free`: an orphaned row is a row that
/// is none of the three.
#[test]
fn a_retired_address_gives_its_row_back() {
    const ADDRESSES: u64 = 400;
    let mut state = object_state("countries", 1);
    let mut previous: Vec<Path> = Vec::new();

    fn every_row_is_accounted_for(state: &SectionState, at: &str) {
        let (rows, live, dead, free) = state.slot_accounting_for_test();
        assert_eq!(
            rows,
            live + dead + free,
            "{at}: {rows} rows are {live} live + {dead} tombstoned + {free} free, so \
             {} are orphaned",
            rows - (live + dead + free)
        );
    }

    for cycle in 0..6u64 {
        // The retirement hour. Committing it merges the store, so every row the
        // retired addresses held is back on the free list before the next hour
        // asks for one.
        {
            let mut hour = state.begin_hour().expect("hour opens");
            for path in &previous {
                assert!(hour.remove(path).expect("removal is legal"));
            }
            hour.commit();
        }
        assert_eq!(state.grain_count(), 0, "cycle {cycle}");
        every_row_is_accounted_for(&state, &format!("cycle {cycle} after retiring"));
        let (rows, _, dead, free) = state.slot_accounting_for_test();
        assert_eq!(dead, 0, "cycle {cycle}: commit left the store unmerged");
        assert_eq!(
            free, rows,
            "cycle {cycle}: {rows} rows but only {free} free after retiring everything"
        );

        // The installation hour, from addresses this section has never held.
        {
            let mut hour = state.begin_hour().expect("hour opens");
            let mut installed = Vec::with_capacity(ADDRESSES as usize);
            for index in 0..ADDRESSES {
                let path: Path = vec![Seg::key(&churn_key(cycle * ADDRESSES + index))];
                hour.upsert(path.clone(), &parse("1")).expect("upsert");
                installed.push(path);
            }
            previous = installed;
            hour.commit();
        }
        assert_eq!(state.grain_count(), ADDRESSES as usize, "cycle {cycle}");
        every_row_is_accounted_for(&state, &format!("cycle {cycle} after installing"));
        let (rows, _, _, _) = state.slot_accounting_for_test();
        assert_eq!(
            rows, ADDRESSES as usize,
            "cycle {cycle}: {rows} rows for {ADDRESSES} addresses, so retired rows are not \
             coming back"
        );
    }
}

/// The order holds through churn on NESTED addresses, not only on the
/// single-segment ones the churn test above uses.
///
/// A grain's address is a path, and the corpus's byte-dominant sections put
/// their megabytes two levels down. Ordering, hashing and the neighbour queries
/// all run over the whole path, so a defect that only shows up past the first
/// segment -- a hash that stopped mixing in the position, a comparison that
/// stopped at the first segment -- would be invisible to a test whose addresses
/// are all one segment long. The prefixes are disjoint from the leaves, because
/// a grain standing at another grain's ancestor is a path set `check_path`
/// refuses and this test is not about that.
#[test]
fn the_store_iterates_in_the_btreemap_order_through_nested_churn() {
    let mut rng = Rng(0x5eed_1234_abcd_0004);
    let mut state = object_state("map", 2);
    let mut shadow: BTreeMap<Path, ()> = BTreeMap::new();
    let mut hour = state.begin_hour().expect("hour opens");
    for step in 0..6_000u64 {
        let path: Path = vec![
            Seg::key(&format!("g{}", rng.below(12))),
            Seg::key(&churn_key(rng.below(120))),
        ];
        if rng.below(3) == 0 {
            hour.remove(&path).expect("removal is legal");
            shadow.remove(&path);
        } else {
            hour.upsert(path.clone(), &parse("1")).expect("upsert");
            shadow.insert(path, ());
        }
        let observed: Vec<&Path> = hour.state().grain_paths().collect();
        assert!(
            observed.len() == shadow.len()
                && observed
                    .iter()
                    .zip(shadow.keys())
                    .all(|(left, right)| *left == right),
            "step {step}: the store's order over nested addresses is not the map's"
        );
    }
    let delta = hour.commit();
    let mut replica = object_state("map", 2);
    replica.apply_delta(&delta.bytes).expect("replica applies");
    assert_eq!(replica.project(), state.project());
    assert_eq!(state.state_root(), state.recomputed_state_root());
}

/// The store's neighbour queries answer what a `BTreeMap` range answers, with
/// tombstones and unmerged insertions sitting in the order arrays.
///
/// `check_path` decides whether two siblings disagree about how their container
/// addresses them by asking for the entries either side of an incoming address.
/// Both queries walk two arrays and skip retired slots, and one that skipped an
/// entry too few or too many would admit a path set the projection cannot write
/// back.
#[test]
fn the_neighbour_queries_answer_what_a_range_answers() {
    let mut rng = Rng(0x5eed_1234_abcd_0003);
    let mut state = object_state("countries", 1);
    let mut shadow: BTreeMap<Path, ()> = BTreeMap::new();
    let mut hour = state.begin_hour().expect("hour opens");
    for step in 0..4_000u64 {
        let path: Path = vec![Seg::key(&churn_key(rng.below(300)))];
        if rng.below(3) == 0 {
            hour.remove(&path).expect("removal is legal");
            shadow.remove(&path);
        } else {
            hour.upsert(path.clone(), &parse("1")).expect("upsert");
            shadow.insert(path, ());
        }
        // A probe address that is often NOT in the state, so the queries have to
        // answer about a gap and not only about an entry.
        let probe: Path = vec![Seg::key(&churn_key(rng.below(400)))];
        assert_eq!(
            hour.state().first_at_or_after_for_test(&probe),
            shadow
                .range(probe.clone()..)
                .next()
                .map(|(path, _)| path.clone()),
            "step {step}: the first address at or after {probe:?} is wrong"
        );
        assert_eq!(
            hour.state().last_before_for_test(&probe),
            shadow
                .range(..probe.clone())
                .next_back()
                .map(|(path, _)| path.clone()),
            "step {step}: the last address before {probe:?} is wrong"
        );
    }
    hour.commit();
}

//! `d58_building_census_walk`: the H4CI building census, driven over a STAGED
//! `CBuildingDatabase` by the production reader.
//!
//! # Why these exist
//!
//! The building witness split's first two tests built a `BuildingWitnessSplit`
//! by hand and asserted arithmetic over it. That pins the accounting type, and
//! nothing else: an uncounted `continue` in `read_building_identities` - a slot
//! the reader attempts and then abandons without recording an outcome, which is
//! the exact defect the partition invariant exists to catch - leaves both of
//! them passing. Codex review round 1 of lane MZ said so; this file is the
//! answer.
//!
//! Everything below drives `read_building_identities` ITSELF over a synthetic
//! database in this process's memory, reached through the same guarded readers
//! the collector uses inside the game, at the same offsets from the same
//! production constants. No offset, stride or bound is restated here.
//!
//! # What a staged database can and cannot prove
//!
//! It proves what the READER does with a given memory shape: which category a
//! slot lands in, that the categories partition the attempted slots, that a
//! rejected pointer produces no witness text, and that the probe budget is
//! finite and says so. It proves NOTHING about what the running game's memory
//! actually looks like - that is what the live captures and the RE citations in
//! `runs/mz_d58_batch_b/RE58B_BUILDING_WITNESSES.md` are for.

use super::*;

use content_identity::{BuildingWitnessOutcome, BuildingWitnessSplit};

/// One page, and a reserve big enough to hold every RVA the walk reads.
const D58_FIX_PAGE: usize = 0x1000;

/// The building-database pointer and the four `CPdxLexer` globals. The lexer
/// globals sit above the database pointer, so they size the reserve.
const D58_FIX_RESERVE: usize = 0x0380_0000;

/// A synthetic module image: `base + rva` is readable for every RVA the walk
/// reads, exactly as it is for a loaded `hoi4.exe`.
struct D58ModuleImage {
    base: usize,
    reserve: *mut c_void,
}

impl Drop for D58ModuleImage {
    fn drop(&mut self) {
        unsafe {
            VirtualFree(self.reserve, 0, T_MEM_RELEASE);
        }
    }
}

fn d58_module_image() -> D58ModuleImage {
    assert!(D58_FIX_RESERVE > content_identity::CBUILDING_DATABASE_PTR_RVA + D58_FIX_PAGE);
    assert!(D58_FIX_RESERVE > PDX_LEXER_TOKEN_NAME_TABLE_CAPACITY_RVA + D58_FIX_PAGE);
    assert_eq!(
        PDX_LEXER_INITIALIZED_RVA & !(D58_FIX_PAGE - 1),
        PDX_LEXER_TOKEN_NAME_TABLE_CAPACITY_RVA & !(D58_FIX_PAGE - 1),
        "the lexer globals must share one page, or this fixture commits too few"
    );
    unsafe {
        let reserve = VirtualAlloc(
            ptr::null_mut(),
            D58_FIX_RESERVE,
            T_MEM_RESERVE,
            T_PAGE_READWRITE,
        );
        assert!(!reserve.is_null(), "reserve the synthetic module image");
        for rva in [
            content_identity::CBUILDING_DATABASE_PTR_RVA,
            PDX_LEXER_INITIALIZED_RVA,
        ] {
            let page_rva = rva & !(D58_FIX_PAGE - 1);
            let page = VirtualAlloc(
                (reserve as usize + page_rva) as *mut c_void,
                D58_FIX_PAGE,
                MEM_COMMIT,
                T_PAGE_READWRITE,
            );
            assert!(!page.is_null(), "commit the page for rva {rva:#x}");
        }
        D58ModuleImage {
            base: reserve as usize,
            reserve,
        }
    }
}

fn d58_collector_state(image: &D58ModuleImage) -> CollectorState {
    CollectorState {
        base: image.base,
        daily_addr: 0,
        long_string_addr: 0,
        daily_match: true,
        long_string_match: true,
        date_ptr: 0,
        country_database_ptr: 0,
        country_identity: CountryIdentityCache::not_attempted("d58-fixture"),
        special_projects: SpecialProjectsCache::not_attempted("d58-fixture"),
        weather: CompactWeatherRoot::default(),
    }
}

/// Write an MSVC `std::string` header at `offset`.
///
/// Short texts take the inline form the engine's own short keys take; anything
/// from 16 bytes up takes the heap form, with the payload in a region the
/// caller keeps alive. This is the layout `read_msvc_string` decodes, spelled
/// once.
fn write_msvc_string(region: &mut state_fixture::FixtureRegion, offset: usize, value: &str) {
    let bytes = value.as_bytes();
    assert!(bytes.len() < 16, "use write_msvc_heap_string for 16+ bytes");
    region.write_bytes(offset, bytes);
    region.write_u8(offset + bytes.len(), 0);
    region.write_u64(offset + 0x10, bytes.len() as u64);
    region.write_u64(offset + 0x18, 15);
}

/// The heap form: a payload region of its own, and a header pointing at it.
/// Returns the payload region, which the caller must keep alive for as long as
/// the reader may follow the pointer.
fn write_msvc_heap_string(
    region: &mut state_fixture::FixtureRegion,
    offset: usize,
    value: &str,
) -> state_fixture::FixtureRegion {
    let bytes = value.as_bytes();
    let mut payload = state_fixture::FixtureRegion::new(bytes.len() + 1);
    payload.write_bytes(0, bytes);
    payload.write_u8(bytes.len(), 0);
    region.write_u64(offset, payload.addr() as u64);
    region.write_u64(offset + 0x10, bytes.len() as u64);
    region.write_u64(offset + 0x18, bytes.len() as u64);
    payload
}

/// Either form, chosen by length. Returns the payload region when the heap
/// form was used; the caller must keep it alive.
fn write_msvc_any(
    region: &mut state_fixture::FixtureRegion,
    offset: usize,
    value: &str,
) -> Option<state_fixture::FixtureRegion> {
    if value.len() < 16 {
        write_msvc_string(region, offset, value);
        None
    } else {
        Some(write_msvc_heap_string(region, offset, value))
    }
}

/// What one staged slot is supposed to become.
enum Slot {
    /// A definition the reader must publish.
    Published { token_id: u32, key: &'static str },
    /// The element pointer is not a plausible heap object.
    RejectedPointer,
    /// `template + 0x270` does not decode.
    TokenUnreadable { token_id: u32, key: &'static str },
    /// The database's key record does not decode.
    DatabaseKeyUnreadable { token_id: u32, key: &'static str },
    /// The lexer has no admissible text for the id.
    NoLexerText { key: &'static str },
    /// The template's key and the database's key record differ.
    KeysDisagree {
        token_id: u32,
        on_template: &'static str,
        in_database: &'static str,
    },
    /// The template's key and the lexer's text for its id differ.
    LexerDisagrees { token_id: u32, key: &'static str },
    /// A key longer than the reader can return, shaped like a script token.
    OverBoundKey { token_id: u32, len: usize },
    /// A long run of characters script cannot produce: NOT a bound loss.
    OverBoundGarbage { token_id: u32 },
}

/// A staged `CBuildingDatabase` plus the lexer table that names its ids.
///
/// Everything it allocates is held here so nothing the reader may follow is
/// freed while the walk runs.
struct D58Fixture {
    image: D58ModuleImage,
    #[allow(dead_code)]
    owned: Vec<state_fixture::FixtureRegion>,
    #[allow(dead_code)]
    database: state_fixture::FixtureRegion,
}

/// The lexer entry stride is the unit the name table is subscripted in, so a
/// table for `max_token_id` needs one entry more than that.
fn lexer_table_len(max_token_id: u32) -> usize {
    (max_token_id as usize + 2) * PDX_LEXER_TOKEN_NAME_STRIDE
}

impl D58Fixture {
    /// Stage `slots` in order. Slot `k` of the array is element `k + 1`; the
    /// engine's null object sits in element 0 and is never questioned.
    fn build(slots: &[Slot]) -> Self {
        let image = d58_module_image();
        let mut owned: Vec<state_fixture::FixtureRegion> = Vec::new();

        // --- the lexer's name table -------------------------------------
        //
        // Every id any slot mentions gets its text here. `LexerDisagrees`
        // deliberately gets a DIFFERENT text from the key on its template, and
        // `NoLexerText` deliberately gets an id past the table's ceiling.
        let mut lexer_texts: Vec<(u32, String)> = Vec::new();
        for slot in slots {
            match slot {
                Slot::Published { token_id, key }
                | Slot::TokenUnreadable { token_id, key }
                | Slot::DatabaseKeyUnreadable { token_id, key } => {
                    lexer_texts.push((*token_id, (*key).to_string()))
                }
                Slot::KeysDisagree {
                    token_id,
                    on_template,
                    ..
                } => lexer_texts.push((*token_id, (*on_template).to_string())),
                Slot::LexerDisagrees { token_id, key } => {
                    lexer_texts.push((*token_id, format!("{key}_elsewhere")))
                }
                Slot::OverBoundKey { token_id, .. } | Slot::OverBoundGarbage { token_id } => {
                    lexer_texts.push((*token_id, format!("bound_{token_id}")))
                }
                // These two mention no id the lexer must name: the first is
                // refused before any witness is read, the second deliberately
                // carries an id past the table's ceiling.
                Slot::RejectedPointer | Slot::NoLexerText { .. } => {}
            }
        }
        let max_token_id = lexer_texts
            .iter()
            .map(|(id, _)| *id)
            .max()
            .expect("a fixture stages at least one named id");
        let mut table = state_fixture::FixtureRegion::new(lexer_table_len(max_token_id));
        for (token_id, text) in &lexer_texts {
            let entry = (*token_id as usize) * PDX_LEXER_TOKEN_NAME_STRIDE;
            if let Some(payload) = write_msvc_any(&mut table, entry, text) {
                owned.push(payload);
            }
        }
        let table_addr = table.addr();
        assert!(
            is_map_control_heap_pointer(table_addr),
            "the staged lexer table must satisfy the reader's heap-pointer gate"
        );
        owned.push(table);

        // --- templates and key records ----------------------------------
        //
        // One template region per slot, so a slot's own failure cannot make its
        // neighbour unreadable, and one key vector for the whole database
        // because the engine keeps exactly one.
        let key_stride = content_identity::CBUILDING_DATABASE_KEY_STRIDE;
        let mut keys = state_fixture::FixtureRegion::new(key_stride * (slots.len() + 1));
        let mut elements = state_fixture::FixtureRegion::new(8 * (slots.len() + 2));
        // Element 0 is the null object. The walk starts at 1 and must never
        // question it; giving it a readable address proves the walk SKIPS it
        // rather than merely failing on it.
        let null_object = state_fixture::FixtureRegion::new(0x300);
        elements.write_u64(0, null_object.addr() as u64);
        owned.push(null_object);

        for (index, slot) in slots.iter().enumerate() {
            let key_offset = index * key_stride + content_identity::CBUILDING_DATABASE_KEY_TEXT_OFF;
            let element_offset = (index + 1) * content_identity::CBUILDING_DATABASE_STRIDE;
            if let Slot::RejectedPointer = slot {
                // Eight-aligned and far below the reader's heap window, so the
                // element read SUCCEEDS and the pointer gate is what refuses.
                elements.write_u64(element_offset, 0x1000);
                write_msvc_string(&mut keys, key_offset, "unquestioned");
                continue;
            }
            let mut template = state_fixture::FixtureRegion::new(
                content_identity::CBUILDING_TEMPLATE_TOKEN_OFF + 0x40,
            );
            let token_off = content_identity::CBUILDING_TEMPLATE_TOKEN_OFF;
            let id_off = content_identity::CBUILDING_TEMPLATE_TYPE_ID_OFF;
            match slot {
                Slot::RejectedPointer => unreachable!("handled above"),
                Slot::Published { token_id, key } | Slot::LexerDisagrees { token_id, key } => {
                    template.write_u32(id_off, *token_id);
                    write_msvc_string(&mut template, token_off, key);
                    write_msvc_string(&mut keys, key_offset, key);
                }
                Slot::TokenUnreadable { token_id, key } => {
                    template.write_u32(id_off, *token_id);
                    // A header claiming a capacity the reader calls implausible:
                    // refused for a reason that is NOT the size bound, so the
                    // probe must not launder it into a bound loss.
                    template.write_u64(token_off + 0x10, 8);
                    template.write_u64(token_off + 0x18, (1usize << 20) as u64 + 1);
                    write_msvc_string(&mut keys, key_offset, key);
                }
                Slot::DatabaseKeyUnreadable { token_id, key } => {
                    template.write_u32(id_off, *token_id);
                    write_msvc_string(&mut template, token_off, key);
                    keys.write_u64(key_offset + 0x10, 8);
                    keys.write_u64(key_offset + 0x18, (1usize << 20) as u64 + 1);
                }
                Slot::NoLexerText { key } => {
                    // An id past the table's ceiling: the lexer has no text and
                    // the reader must not index there anyway.
                    template.write_u32(id_off, max_token_id + 4096);
                    write_msvc_string(&mut template, token_off, key);
                    write_msvc_string(&mut keys, key_offset, key);
                }
                Slot::KeysDisagree {
                    token_id,
                    on_template,
                    in_database,
                } => {
                    template.write_u32(id_off, *token_id);
                    write_msvc_string(&mut template, token_off, on_template);
                    write_msvc_string(&mut keys, key_offset, in_database);
                }
                Slot::OverBoundKey { token_id, len } => {
                    template.write_u32(id_off, *token_id);
                    let long = "b".repeat(*len);
                    owned.push(write_msvc_heap_string(&mut template, token_off, &long));
                    owned.push(write_msvc_heap_string(&mut keys, key_offset, &long));
                }
                Slot::OverBoundGarbage { token_id } => {
                    template.write_u32(id_off, *token_id);
                    // Printable, so it DECODES; spaces and slashes, so it is not
                    // a token any script could have produced.
                    let long = "a b/c ".repeat(content_identity::READER_MAX_TOKEN_LEN / 3);
                    owned.push(write_msvc_heap_string(&mut template, token_off, &long));
                    owned.push(write_msvc_heap_string(&mut keys, key_offset, &long));
                }
            }
            let template_addr = template.addr();
            assert!(
                is_map_control_heap_pointer(template_addr),
                "the staged template must satisfy the reader's heap-pointer gate"
            );
            elements.write_u64(element_offset, template_addr as u64);
            owned.push(template);
        }

        let keys_addr = keys.addr();
        let elements_addr = elements.addr();
        owned.push(keys);
        owned.push(elements);

        let mut database = state_fixture::FixtureRegion::new(
            content_identity::CBUILDING_DATABASE_KEY_COUNT_OFF + 0x20,
        );
        database.write_u64(
            content_identity::CBUILDING_DATABASE_ELEMENTS_OFF,
            elements_addr as u64,
        );
        database.write_i32(
            content_identity::CBUILDING_DATABASE_COUNT_OFF,
            slots.len() as i32 + 1,
        );
        database.write_u64(content_identity::CBUILDING_DATABASE_KEYS_OFF, keys_addr as u64);
        database.write_i32(
            content_identity::CBUILDING_DATABASE_KEY_COUNT_OFF,
            slots.len() as i32,
        );
        let database_addr = database.addr();
        assert!(
            is_map_control_heap_pointer(database_addr),
            "the staged database must satisfy the reader's heap-pointer gate"
        );

        unsafe {
            let put_u32 = |rva: usize, value: u32| {
                ptr::copy_nonoverlapping(
                    value.to_le_bytes().as_ptr(),
                    (image.base + rva) as *mut u8,
                    4,
                );
            };
            ptr::copy_nonoverlapping(
                (database_addr as u64).to_le_bytes().as_ptr(),
                (image.base + content_identity::CBUILDING_DATABASE_PTR_RVA) as *mut u8,
                8,
            );
            *((image.base + PDX_LEXER_INITIALIZED_RVA) as *mut u8) = 1;
            ptr::copy_nonoverlapping(
                (table_addr as u64).to_le_bytes().as_ptr(),
                (image.base + PDX_LEXER_TOKEN_NAME_TABLE_PTR_RVA) as *mut u8,
                8,
            );
            put_u32(PDX_LEXER_MAX_TOKEN_ID_RVA, max_token_id);
            put_u32(PDX_LEXER_TOKEN_NAME_TABLE_CAPACITY_RVA, max_token_id + 1);
        }
        page_cache_new_frame();

        D58Fixture {
            image,
            owned,
            database,
        }
    }

    /// Run the PRODUCTION walk over the staged database.
    fn walk(
        &self,
    ) -> (
        Vec<compact_contract::content_identity_census::ContentIdentityRow>,
        u32,
        usize,
        BuildingWitnessSplit,
    ) {
        let state = d58_collector_state(&self.image);
        let names = read_pdx_lexer_token_names(&state)
            .expect("the staged lexer globals are the shape the reader requires");
        content_identity::read_building_identities(&state, &names)
            .expect("the staged database is readable")
    }
}

/// Every category the split can report, staged once, walked once.
///
/// kills: an `continue` in `read_building_identities` that abandons an
/// attempted slot without recording an outcome. The accounting is taken from
/// the reader's own answer, so a slot the reader silently drops makes
/// `is_accounted()` false and this fails - which the hand-built split test
/// cannot do, because it never runs the reader.
///
/// kills: routing a slot to the wrong category. Each staged shape names the
/// category it must produce, so swapping two arms of the reader's match fails
/// here even though the totals are unchanged.
#[test]
fn the_reader_puts_every_attempted_slot_in_exactly_one_category() {
    use BuildingWitnessOutcome as Outcome;
    let fixture = D58Fixture::build(&[
        Slot::Published {
            token_id: 11,
            key: "arms_factory",
        },
        Slot::RejectedPointer,
        Slot::TokenUnreadable {
            token_id: 12,
            key: "dockyard",
        },
        Slot::DatabaseKeyUnreadable {
            token_id: 13,
            key: "bunker",
        },
        Slot::NoLexerText { key: "coastal_gun" },
        Slot::KeysDisagree {
            token_id: 14,
            on_template: "radar_station",
            in_database: "rocket_site",
        },
        Slot::LexerDisagrees {
            token_id: 15,
            key: "nuclear_reactor",
        },
        Slot::OverBoundKey {
            token_id: 16,
            len: content_identity::READER_MAX_TOKEN_LEN + 1,
        },
        Slot::OverBoundGarbage { token_id: 17 },
        Slot::Published {
            token_id: 18,
            key: "air_base",
        },
    ]);
    let (rows, declared, attempted, split) = fixture.walk();

    assert_eq!(declared, 10, "ten definitions past the null object");
    assert_eq!(attempted, 10, "all ten are inside the record's row bound");
    assert!(
        split.is_accounted(),
        "the reader lost a slot it attempted without saying where it went: {:?}",
        split.render()
    );
    assert_eq!(split.attempted(), 10);
    assert_eq!(split.declared(), 10);

    assert_eq!(
        rows.iter().map(|row| row.token_id).collect::<Vec<_>>(),
        vec![11, 18],
        "only the two sound definitions are published"
    );
    assert_eq!(split.published(), 2);

    assert_eq!(split.count_of(Outcome::TemplateRejectedBeforeWitnesses), 1);
    // TWO: the header the reader refuses as implausible, and the long run of
    // non-script characters, which the probe must NOT launder into a bound loss.
    assert_eq!(split.count_of(Outcome::TokenFieldUnreadable), 2);
    assert_eq!(split.count_of(Outcome::DatabaseKeyUnreadable), 1);
    assert_eq!(split.count_of(Outcome::LexerTextUnreadable), 1);
    assert_eq!(split.count_of(Outcome::TokenDisagreesWithDatabaseKey), 1);
    assert_eq!(split.count_of(Outcome::TokenDisagreesWithLexerText), 1);
    assert_eq!(split.count_of(Outcome::DuplicateIdDropped), 0);
    assert_eq!(
        split.count_of(Outcome::TokenOverReaderBound),
        1,
        "only the key SHAPED LIKE A NAME is a bound loss"
    );
    assert!(
        split
            .render()
            .iter()
            .any(|line| line.contains("token_over_reader_bound=1")),
        "the split prints the bound loss it found: {:?}",
        split.render()
    );
}

/// A rejected element pointer produces a category with NO witness text.
///
/// kills: filling the rejected-pointer example with a slot ordinal, an address,
/// or a witness the reader never obtained - asked of the example the READER
/// built, not of `BuildingWitnessExample::default()`.
#[test]
fn a_rejected_pointer_reports_a_category_with_nothing_read() {
    let fixture = D58Fixture::build(&[
        Slot::Published {
            token_id: 11,
            key: "arms_factory",
        },
        Slot::RejectedPointer,
    ]);
    let (_, _, _, split) = fixture.walk();
    let line = split
        .render()
        .into_iter()
        .find(|line| line.contains("template_rejected_before_witnesses="))
        .expect("the category the staged pointer produced is printed");
    assert!(
        line.ends_with("template_rejected_before_witnesses=1 example: nothing was read"),
        "the example must carry no witness and no address: {line}"
    );
}

/// Two definitions claiming one id lose BOTH copies, and the split says so.
///
/// kills: keeping one of a colliding pair (which publishes a name under an id
/// the reader has just proved it misunderstands), and counting the collision
/// once instead of counting the definitions lost.
#[test]
fn two_definitions_claiming_one_id_are_both_dropped_and_both_counted() {
    use BuildingWitnessOutcome as Outcome;
    let fixture = D58Fixture::build(&[
        Slot::Published {
            token_id: 11,
            key: "arms_factory",
        },
        Slot::Published {
            token_id: 11,
            key: "arms_factory",
        },
        Slot::Published {
            token_id: 12,
            key: "dockyard",
        },
    ]);
    let (rows, _, _, split) = fixture.walk();
    assert_eq!(
        rows.iter().map(|row| row.token_id).collect::<Vec<_>>(),
        vec![12],
        "both copies of the collided id go"
    );
    assert_eq!(split.count_of(Outcome::DuplicateIdDropped), 2);
    assert_eq!(split.published(), 1);
    assert!(split.is_accounted(), "{:?}", split.render());
}

/// The probe that decides "was that our own ceiling" is BUDGETED IN BYTES, and
/// the exhaustion is printed rather than inferred.
///
/// kills: an unbudgeted probe path. A namespace may offer thousands of slots
/// and each probe may read up to `PROBE_MAX_TOKEN_LEN`; attach is not a place
/// to spend gigabytes of guarded reads explaining a diagnostic. Past the budget
/// the reader must fall back to the conflict side (fail closed) and SAY it
/// stopped asking.
#[test]
fn the_probe_budget_is_finite_and_the_split_says_when_it_ran_out() {
    use BuildingWitnessOutcome as Outcome;
    // Each slot stages TWO keys of a whole probe ceiling each - the template's
    // and the database's - so a handful of slots outruns the byte budget.
    let per_slot = 2 * content_identity::PROBE_MAX_TOKEN_LEN;
    let slot_count = content_identity::PROBE_BUDGET_BYTES_PER_NAMESPACE / per_slot + 2;
    let slots: Vec<Slot> = (0..slot_count)
        .map(|index| Slot::OverBoundKey {
            token_id: index as u32 + 1,
            len: content_identity::PROBE_MAX_TOKEN_LEN,
        })
        .collect();
    let (_, _, _, split) = D58Fixture::build(&slots).walk();

    assert!(
        split.is_accounted(),
        "running out of budget must not lose a slot: {:?}",
        split.render()
    );
    let named = split.count_of(Outcome::TokenOverReaderBound);
    assert!(
        named > 0 && named < slots.len(),
        "the budget must name some and then stop: {named} of {}",
        slots.len()
    );
    let header = split.render().remove(0);
    assert!(
        header.contains("probe_budget_exhausted=true"),
        "the header states that the reader stopped asking: {header}"
    );

    // And a walk that never needed a probe says so too.
    let clean = D58Fixture::build(&[Slot::Published {
        token_id: 11,
        key: "arms_factory",
    }])
    .walk()
    .3;
    assert!(clean
        .render()
        .remove(0)
        .contains("probe_budget_exhausted=false"));
}

/// A whole content set of ordinary over-long mod keys is fully explained.
///
/// kills: budgeting the probe by COUNT. Codex round 6's scenario, as a test: a
/// mod whose keys are 161 bytes - valid script names, one byte past what this
/// reader returns - must all be named as BOUND losses. Under a 64-probe budget
/// the 33rd slot onward became `token_field_unreadable`, the census added
/// CONFLICT_DROPPED beside BOUND_TRUNCATED, and it said a disagreement had
/// happened that never did. A byte budget spends 161 bytes on each of them.
#[test]
fn a_thousand_ordinary_over_long_keys_are_all_named_as_bound_losses() {
    use BuildingWitnessOutcome as Outcome;
    let slot_count = 1_000;
    let slots: Vec<Slot> = (0..slot_count)
        .map(|index| Slot::OverBoundKey {
            token_id: index as u32 + 1,
            len: content_identity::READER_MAX_TOKEN_LEN + 1,
        })
        .collect();
    let (rows, _, attempted, split) = D58Fixture::build(&slots).walk();

    assert_eq!(attempted, slot_count);
    assert!(rows.is_empty(), "an over-long key publishes no row");
    assert_eq!(
        split.count_of(Outcome::TokenOverReaderBound),
        slot_count,
        "every one of them is this reader's own ceiling"
    );
    assert_eq!(
        split.count_of(Outcome::TokenFieldUnreadable),
        0,
        "not one of them is reported as unreadable"
    );
    let header = split.render().remove(0);
    assert!(
        header.contains("probe_budget_exhausted=false"),
        "1,000 x 161 bytes is nowhere near the budget: {header}"
    );
}

/// The RESOURCE namespace says when its own probe budget ran out.
///
/// kills: reporting only the building namespace's exhaustion. The resource walk
/// publishes no per-category split, so if its budget is silent an operator
/// reading CONFLICT_DROPPED has no way to learn that the reader stopped asking
/// (Codex round 6). The two flags are separate fields and separate budgets.
#[test]
fn the_resource_namespaces_exhaustion_is_reported_separately() {
    let header = D58Fixture::build(&[Slot::Published {
        token_id: 11,
        key: "arms_factory",
    }])
    .walk()
    .3
    .render()
    .remove(0);
    assert!(
        header.contains("resource_probe_budget_exhausted="),
        "the report names the resource namespace's budget at all: {header}"
    );
    assert!(
        header.contains("probe_budget_exhausted=false resource_probe_budget_exhausted=false"),
        "and the two flags are distinct fields, not one printed twice: {header}"
    );
}

/// The probe's ceiling is the SHARED STRING READER'S OWN, measured.
///
/// kills: moving `PROBE_MAX_TOKEN_LEN` to any other number. Below the reader's
/// capacity ceiling it would call a longer-but-valid key a conflict; above it,
/// it would claim a reach `read_msvc_string` does not have. This asks the
/// reader, so the constant cannot drift away from the thing it names.
#[test]
fn the_probe_bound_is_the_shared_readers_own_ceiling() {
    let bound = content_identity::PROBE_MAX_TOKEN_LEN;
    // A string exactly at the ceiling reads. Anything longer necessarily
    // declares a capacity above it, which the reader refuses as implausible -
    // so no readable string is longer, and the probe's bound is not a choice.
    let text = "k".repeat(bound);
    let mut header = state_fixture::FixtureRegion::new(0x20);
    let _payload = write_msvc_heap_string(&mut header, 0, &text);
    page_cache_new_frame();
    assert_eq!(
        read_msvc_string(header.addr(), bound).map(|read| read.len()),
        Some(bound),
        "the reader returns a string exactly at the probe's bound"
    );

    let mut over = state_fixture::FixtureRegion::new(0x20);
    let _over_payload = write_msvc_heap_string(&mut over, 0, &text);
    over.write_u64(0x10, bound as u64 + 1);
    over.write_u64(0x18, bound as u64 + 1);
    page_cache_new_frame();
    let refusal = read_msvc_string_with_reason(over.addr(), bound + 1)
        .expect_err("a capacity past the ceiling is refused");
    assert_eq!(
        refusal.reason,
        MsvcStringRefusalReason::CapacityImplausible,
        "the ceiling the probe names is the capacity rule, not a size rule"
    );
}

/// `is_script_token` is exactly the charset rule AND the length rule.
///
/// kills: the split changing what any existing caller admits. Forty domains ask
/// `is_script_token`; the probe needs the charset half alone, and the only safe
/// way to give it that is a decomposition that leaves the whole predicate
/// identical on every input.
#[test]
fn is_script_token_is_the_charset_rule_plus_the_length_rule() {
    let long = "a".repeat(MAX_POLITICS_TOKEN_LEN + 1);
    let at_bound = "a".repeat(MAX_POLITICS_TOKEN_LEN);
    for value in [
        "",
        "arms_factory",
        "AIR-BASE.2:x",
        "has space",
        "has/slash",
        "has\u{00e9}",
        at_bound.as_str(),
        long.as_str(),
    ] {
        assert_eq!(
            is_script_token(value),
            is_script_token_charset(value) && value.len() <= MAX_POLITICS_TOKEN_LEN,
            "the decomposition changed the predicate for {value:?}"
        );
    }
    assert!(
        is_script_token_charset(&long) && !is_script_token(&long),
        "the charset half must admit exactly what the probe needs: a name too \
         long for this reader but shaped like one"
    );
}

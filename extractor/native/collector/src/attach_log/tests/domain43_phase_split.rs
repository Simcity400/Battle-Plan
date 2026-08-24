use super::*;

const D43_PHASE_UNITS: [&str; 11] = [
    "d43_phase_census_us",
    "d43_phase_contract_ptrs_us",
    "d43_phase_contract_rows_us",
    "d43_phase_detail_rows_us",
    "d43_phase_descriptors_us",
    "d43_phase_resource_flow_us",
    "d43_phase_factory_header_us",
    "d43_phase_resource_rows_us",
    "d43_phase_classic_deals_us",
    "d43_phase_stockpile_us",
    "d43_phase_total_us",
];

const D43_PHASE_VALUES: [u64; 11] = [11, 22, 33, 44, 55, 66, 77, 88, 99, 110, 121];

fn read_with_phase(status_flags: u32) -> CompactMarketTradeRead {
    CompactMarketTradeRead {
        rows: Vec::new(),
        phase: Domain43PhaseTimes {
            census_us: 11,
            contract_ptrs_us: 22,
            contract_rows_us: 33,
            detail_rows_us: 44,
            descriptors_us: 55,
            resource_flow_us: 66,
            factory_header_us: 77,
            resource_rows_us: 88,
            classic_deals_us: 99,
            stockpile_us: 110,
            total_us: 121,
        },
        detail_rows: Vec::new(),
        resource_flow_rows: Vec::new(),
        resource_flow_present: false,
        factory_header_rows: Vec::new(),
        factory_header_present: false,
        resource_rows: Vec::new(),
        resource_rows_present: false,
        resource_row_resource_count: 0,
        resource_tokens: Vec::new(),
        classic_deal_rows: Vec::new(),
        classic_deal_country_witnesses: Vec::new(),
        classic_deal_incomplete_owners: Vec::new(),
        classic_deals_present: false,
        activation: None,
        stockpile_countries: Vec::new(),
        stockpile_items: Vec::new(),
        stockpile_expected_country_count: 0,
        stockpile_present: false,
        detail_skipped_rows: 0,
        read_time_us: 0,
        changed_rows: 0,
        status_flags,
        observed_contract_rows: 0,
        emitted_contract_rows: 0,
        buyer_ptr_valid_rows: 0,
        variant_valid_rows: 0,
        amount_valid_rows: 0,
        partial_field_rows: 0,
    }
}

#[test]
fn d43_phase_units_are_emitted_on_a_clean_status() {
    let units = compact_market_trade_diagnostic_units(&read_with_phase(0));
    let names: Vec<&str> = units.iter().map(|unit| unit.name).collect();
    assert_eq!(
        names,
        D43_PHASE_UNITS.to_vec(),
        "a clean d43 frame must carry exactly the phase split and no \
         failure-context word"
    );
    let values: Vec<u64> = units.iter().map(|unit| unit.value).collect();
    assert_eq!(values, D43_PHASE_VALUES.to_vec(), "phase unit wiring");
}

#[test]
fn d43_failure_context_precedes_the_phase_split() {
    let units = compact_market_trade_diagnostic_units(&read_with_phase(
        compact_contract::COMPACT_STATUS_SOURCE_UNAVAILABLE,
    ));
    let names: Vec<&str> = units.iter().map(|unit| unit.name).collect();
    assert!(
        names.len() > D43_PHASE_UNITS.len(),
        "a failing frame must carry failure context as well as the split"
    );
    assert!(
        !names[0].starts_with("d43_phase_"),
        "failure context must fill the per-offender floor before the phase \
         timers; first unit was {:?}",
        names[0]
    );
    assert_eq!(
        &names[names.len() - D43_PHASE_UNITS.len()..],
        &D43_PHASE_UNITS[..],
        "the phase split is appended last"
    );
}

/// Every cause bit is a diagnostic ABI: the reason string is assembled by
/// masking this table, so a bit without a name is a guard that fires and says
/// nothing, and two guards sharing a bit are indistinguishable in a capture.
///
/// kills: adding a guard's constant without adding its name, reusing a bit,
/// or writing one as a multi-bit mask.
#[test]
fn d43_degraded_cause_bits_are_single_bit_distinct_and_named() {
    use domain43_degraded_cause as cause;
    let bits = [
        cause::CENSUS_SHORT,
        cause::ROSTER_ABSENCE_UNBACKED,
        cause::CENSUS_EMPTY,
        cause::CONTRACT_CENSUS,
        cause::CONTRACT_EXCLUDED_SLOTS,
        cause::ACTIVATION_OWNER_UNRESOLVED,
        cause::DUPLICATE_CONTRACT_ROWS,
        cause::DETAIL_ROWS_SKIPPED,
        cause::RESOURCE_FLOW,
        cause::FACTORY_HEADER,
        cause::RESOURCE_ROWS,
        cause::OIL_NEEDED_UNAVAILABLE,
        cause::CLASSIC_INCOMPLETE_OWNERS,
        cause::CLASSIC_SKIPPED,
        cause::STOCKPILE,
        cause::CONTRACT_ROW_REPLACEMENT,
        cause::FRAME_PROLOGUE_REFUSED,
        cause::STATE_PREREQUISITES,
        cause::VTABLE_UNRESOLVED,
    ];
    let mut seen = std::collections::BTreeSet::new();
    for bit in bits {
        assert_eq!(bit.count_ones(), 1, "cause {bit:#x} is not a single bit");
        assert!(seen.insert(bit), "cause {bit:#x} is used twice");
        assert!(
            DOMAIN43_DEGRADED_CAUSE_NAMES
                .iter()
                .any(|(named, _)| *named == bit),
            "cause {bit:#x} has no name, so the reason cannot say it fired"
        );
    }
    assert_eq!(seen.len(), bits.len());
    assert_eq!(
        DOMAIN43_DEGRADED_CAUSE_NAMES.len(),
        bits.len(),
        "the name table names a cause this test does not know about"
    );
    let mut named_words = std::collections::BTreeSet::new();
    for (_, name) in DOMAIN43_DEGRADED_CAUSE_NAMES {
        assert!(!name.is_empty());
        assert!(named_words.insert(*name), "cause name {name} is used twice");
    }
}

/// The producer's own sentence, which is what the frame walk had to invent
/// for itself on all 324 frames of `rc_vanilla`: "domain 43 published a
/// failing PERF status 0x00000200 ... and its producer recorded no outcome of
/// its own".
///
/// kills: dropping the guard names, the root count, the first tag, or the
/// air/land SITE from the reason -- each one is a question the substantiated
/// stand-in could not answer, and answering "partial_data" again is not an
/// answer.
#[test]
fn d43_failure_reason_names_the_guard_the_root_and_the_site() {
    let facts = Domain43DegradedFacts {
        causes: domain43_degraded_cause::OIL_NEEDED_UNAVAILABLE
            | domain43_degraded_cause::RESOURCE_FLOW,
        oil_unavailable_roots: 45,
        oil_stage: domain43_oil_stage::air_land(
            domain43_oil_stage::AIR_LAND_CURRENT_BASE,
            domain43_oil_stage::AIR_LAND_SITE_MANAGER_ENTRY,
        ),
        oil_tag_db_id: 1,
        oil_stage_mask: 0b1_0000,
        oil_unmasked_roots: 2,
        skipped_roots: 0,
    };

    let reason = compact_market_trade_failure_reason_from(
        &read_with_phase(compact_contract::COMPACT_STATUS_PARTIAL_DATA),
        facts,
    );

    for needle in [
        "oil_needed_unavailable",
        "resource_flow",
        "45 root(s)",
        "first tag_db_id 1",
        "at site 10424",
        "frame site mask 0x10",
        // Without this the mask is unreadable: a single bit means "one cause"
        // only when no root refused outside the walk the mask covers.
        "2 of them outside that mask",
    ] {
        assert!(
            reason.contains(needle),
            "the producer's reason lost {needle:?}: {reason}"
        );
    }
}

/// kills: leaving the reason blank when no guard recorded a cause. A silent
/// producer is the finding this outcome exists to answer, so the sentence
/// says so in words rather than trailing off after the colon.
#[test]
fn d43_failure_reason_says_so_when_no_guard_recorded_a_cause() {
    let reason = compact_market_trade_failure_reason_from(
        &read_with_phase(compact_contract::COMPACT_STATUS_PARTIAL_DATA),
        Domain43DegradedFacts::default(),
    );

    assert!(
        reason.contains("no guard recorded a cause"),
        "reason was {reason}"
    );
}

/// The cause table is only worth what the PRODUCTION guards write into it.
///
/// The constants test above pins the table; this one pins the bindings. It
/// walks `read_compact_market_trade`'s own body with every comment and
/// string literal blanked out first -- `//`, `/* */` including nesting, and
/// `"..."` with its escapes -- so that no MENTION of a call can stand in for
/// one. It requires that
///
/// * every in-body line that ORs a failure bit into the status word has a
///   real `domain43_record_degraded_cause(` CALL within the four lines above
///   it;
/// * every early return that publishes `status_flags | COMPACT_STATUS_...`
///   as a struct field has one within four lines above the `return` that
///   carries it -- those three sites are forty lines from their status word
///   and would otherwise be bound by nothing;
/// * the causes the body names are exactly the ones the name table holds,
///   once each.
///
/// kills: adding a guard without a cause, deleting a cause write from any
/// guard including the three early returns, naming one cause from two
/// guards, or "recording" a cause in a comment. None of these is visible to
/// the reason test above, which is handed its facts.
#[test]
fn every_d43_status_guard_records_its_own_cause() {
    const SOURCE: &str = include_str!("../../attach_log.rs");
    let raw_lines: Vec<&str> = SOURCE.lines().collect();
    let start = raw_lines
        .iter()
        .position(|line| line.starts_with("fn read_compact_market_trade("))
        .expect("the d43 reader is in this file");
    let end = start
        + 1
        + raw_lines[start + 1..]
            .iter()
            .position(|line| *line == "}")
            .expect("the reader ends at a column-zero brace");
    // Code only, from here on: a comment or a string that spells a call is
    // not a call, and matching one would let the real call be deleted.
    let body_source = raw_lines[start..end].join("\n");
    // `code_only` does not model raw strings, and a raw string containing a
    // quote would desynchronise it -- which is a way for a MENTION to be
    // exposed as code. Refuse to run rather than trust a sanitiser outside
    // the grammar it covers. ANY hash count, not a list of the short ones.
    assert!(
        !contains_raw_string_prefix(&body_source),
        "the d43 reader gained a raw string; `code_only` does not model those, so this scan \
         must be taught them before its verdict means anything"
    );
    let owned = code_only(&body_source);
    let body: Vec<&str> = owned.lines().collect();
    let body = &body[..];

    // How far back a cause call may sit from what it explains: one line for
    // the common shape, more for the wrapped call the formatter produces
    // when the constant's name is long.
    const LOOKBACK: usize = 4;
    let records_cause = |offset: usize| -> bool {
        body[offset.saturating_sub(LOOKBACK)..offset]
            .iter()
            .any(|candidate| candidate.trim().starts_with("domain43_record_degraded_cause("))
    };

    let mut ored_guards = 0usize;
    let mut early_returns = 0usize;
    for (offset, line) in body.iter().enumerate() {
        let trimmed = line.trim();
        if trimmed.starts_with("status_flags |= COMPACT_STATUS_") {
            ored_guards += 1;
            assert!(
                records_cause(offset),
                "the d43 guard at attach_log.rs:{} sets a failure bit and records no cause. A \
                 bit with no cause is what made the release-candidate capture report that \
                 domain 43's producer recorded no outcome of its own.",
                start + offset + 1
            );
            continue;
        }
        if !trimmed.starts_with("status_flags: status_flags | COMPACT_STATUS_") {
            continue;
        }
        // An early return: its status word is a field of the struct literal,
        // so the binding is to the `return` that carries it.
        early_returns += 1;
        let return_offset = body[..offset]
            .iter()
            .rposition(|candidate| candidate.trim().starts_with("return CompactMarketTradeRead {"))
            .unwrap_or_else(|| {
                panic!(
                    "the d43 early-return status word at attach_log.rs:{} has no return above it",
                    start + offset + 1
                )
            });
        assert!(
            records_cause(return_offset),
            "the d43 early return at attach_log.rs:{} publishes a failure bit and records no \
             cause",
            start + return_offset + 1
        );
    }

    assert!(
        ored_guards >= 15,
        "the scan found only {ored_guards} in-body status guards; it used to find 15, so either \
         a guard left the function or this scan stopped recognising its shape"
    );
    assert_eq!(
        early_returns, 3,
        "the d43 reader has three early returns that publish a status word; the scan found \
         {early_returns}, so their shape changed and this test stopped covering them"
    );

    let joined = body.join("\n");
    // The contract-row guard has no `|=` line for the scan above to anchor
    // on: it assigns the result of `market_trade_contract_status`. Its cause
    // must therefore come from the PREDICATE, never from comparing the
    // status word before and after -- with `PARTIAL_DATA` already set by an
    // earlier guard the word does not change, and the cause would be lost on
    // exactly the frames that carry the most of them.
    assert!(
        joined.contains("if !market_trade_contract_rows_are_exact("),
        "the contract-row cause is no longer taken from its own predicate"
    );
    assert!(
        !joined.contains("contract_replacement_status != status_flags"),
        "the contract-row cause is back to watching the status word, which cannot see a \
         guard whose bit another guard already set"
    );
    let mut used: Vec<&str> = Vec::new();
    for piece in joined.split("domain43_degraded_cause::").skip(1) {
        let name = piece
            .split(|character: char| {
                !(character.is_ascii_uppercase() || character.is_ascii_digit() || character == '_')
            })
            .next()
            .unwrap_or("");
        assert!(!name.is_empty(), "a cause reference names nothing");
        used.push(name);
    }
    let mut distinct = used.clone();
    distinct.sort_unstable();
    distinct.dedup();
    assert_eq!(
        distinct.len(),
        used.len(),
        "two d43 guards share one cause, so a capture cannot tell them apart: {used:?}"
    );
    let mut declared: Vec<&str> = DOMAIN43_DEGRADED_CAUSE_NAMES
        .iter()
        .map(|(_, name)| *name)
        .collect();
    declared.sort_unstable();
    let mut lowered: Vec<String> = distinct.iter().map(|name| name.to_lowercase()).collect();
    lowered.sort();
    // The wire name of a cause is its constant's name in lower case; the two
    // lists being equal is what makes a capture's reason string a complete
    // account of the guards that can fire.
    assert_eq!(
        lowered,
        declared
            .iter()
            .map(|name| (*name).to_string())
            .collect::<Vec<_>>(),
        "the cause table and the guards that write it have drifted apart"
    );
}

/// The trap the contract-row guard fell into, in two lines of arithmetic.
///
/// kills: recording that guard's cause by comparing the status word before
/// and after `market_trade_contract_status`. The predicate can be FALSE
/// while the word is unchanged, and on a d43 frame that is the normal case:
/// the oil counter sets `PARTIAL_DATA` several guards earlier.
#[test]
fn d43_contract_row_replacement_cause_cannot_be_read_off_the_status_word() {
    assert!(market_trade_contract_rows_are_exact(3, 3, 0));
    assert!(!market_trade_contract_rows_are_exact(3, 2, 0));
    assert!(!market_trade_contract_rows_are_exact(3, 3, 1));

    let already_partial = compact_contract::COMPACT_STATUS_PARTIAL_DATA;
    assert_eq!(
        market_trade_contract_status(already_partial, 3, 2, 0),
        already_partial,
        "the word is unchanged, so a word-watching caller sees nothing"
    );
    assert_eq!(
        market_trade_contract_status(0, 3, 2, 0),
        already_partial,
        "...while the same inputs on a clean word DO set the bit"
    );
}

/// The sanitiser the guard scan rests on, exercised directly.
///
/// kills: weakening `code_only` back to a `//`-only strip, dropping its
/// nesting or its escape handling -- each of which lets a MENTION of
/// `domain43_record_degraded_cause(` stand in for the call it replaced, with
/// every count in the scan above still satisfied.
#[test]
fn code_only_blanks_every_shape_that_is_not_code() {
    let sample = concat!(
        "keep_one();\n",
        "// domain43_record_degraded_cause(line_comment);\n",
        "/* domain43_record_degraded_cause(block); /* domain43_record_degraded_cause(nested); */ still_comment */\n",
        "let text = \"domain43_record_degraded_cause(string);\";\n",
        "let escaped = \"he said \\\"domain43_record_degraded_cause(quoted);\\\" and stopped\";\n",
        "keep_two();\n",
    );

    let cleaned = code_only(sample);

    assert!(
        !cleaned.contains("domain43_record_degraded_cause"),
        "a mention survived the sanitiser: {cleaned}"
    );
    assert!(cleaned.contains("keep_one();"));
    assert!(cleaned.contains("keep_two();"));
    assert!(cleaned.contains("let text ="), "code outside the string must survive");
    assert_eq!(
        cleaned.lines().count(),
        sample.lines().count(),
        "line structure must survive, or the scan reports the wrong source line"
    );
}

/// Does this source hold a raw-string opener -- `r"`, `br"`, or either with
/// ANY number of hashes?
///
/// A quote counts only when the `r`/`br` in front of its hashes starts a
/// token, so ordinary prose ending in "r" inside a string or comment (`for"`)
/// is not mistaken for one.
fn contains_raw_string_prefix(source: &str) -> bool {
    source.char_indices().any(|(at, character)| {
        if character != '"' {
            return false;
        }
        let before = &source[..at];
        let without_hashes = before.trim_end_matches('#');
        for prefix in ["br", "r"] {
            if let Some(rest) = without_hashes.strip_suffix(prefix) {
                let starts_a_token = rest
                    .chars()
                    .next_back()
                    .is_none_or(|previous| !previous.is_alphanumeric() && previous != '_');
                if starts_a_token {
                    return true;
                }
            }
        }
        false
    })
}

/// kills: narrowing the raw-string refusal back to a list of the short
/// spellings. `r##"..."##` is as legal as `r"..."`, and a quote inside one
/// desynchronises `code_only` -- which is how a MENTION of a cause call gets
/// exposed as code and the guard scan stops meaning anything.
#[test]
fn raw_string_prefixes_are_recognised_at_any_hash_count() {
    for raw in [
        "let a = r\"x\";",
        "let b = r#\"x\"#;",
        "let c = r##\"x\"##;",
        "let d = br\"x\";",
        "let e = br###\"x\"###;",
        "(r\"x\")",
    ] {
        assert!(contains_raw_string_prefix(raw), "missed a raw string in {raw}");
    }
    for ordinary in [
        "let f = \"plain\";",
        "let g = \"ends in for\";",
        "let h = \"a#\";",
        "// domain43_record_degraded_cause(x);",
        "let counter = 1;",
    ] {
        assert!(
            !contains_raw_string_prefix(ordinary),
            "ordinary source refused as a raw string: {ordinary}"
        );
    }
}

/// Blank out everything that is not code: `//` to end of line, `/* */` with
/// nesting, and `"..."` string literals with their escapes. Characters are
/// replaced by spaces rather than removed so line and column structure
/// survives -- the scan above reports the source line a guard sits on.
///
/// The one shape it does not model is a raw string (`r"..."`, `r#"..."#`),
/// and `read_compact_market_trade` contains none; if one ever appears the
/// scan mis-tracks and its counts fall short of the asserted minimum, which
/// fails the test rather than passing it quietly.
fn code_only(source: &str) -> String {
    let mut out = String::with_capacity(source.len());
    let mut characters = source.chars().peekable();
    let mut block_depth = 0usize;
    let mut in_line_comment = false;
    let mut in_string = false;
    let mut escaped = false;
    while let Some(character) = characters.next() {
        if character == '\n' {
            out.push('\n');
            in_line_comment = false;
            continue;
        }
        if in_line_comment {
            out.push(' ');
            continue;
        }
        if block_depth > 0 {
            if character == '*' && characters.peek() == Some(&'/') {
                characters.next();
                block_depth -= 1;
                out.push_str("  ");
            } else if character == '/' && characters.peek() == Some(&'*') {
                characters.next();
                block_depth += 1;
                out.push_str("  ");
            } else {
                out.push(' ');
            }
            continue;
        }
        if in_string {
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == '"' {
                in_string = false;
            }
            out.push(' ');
            continue;
        }
        if character == '/' && characters.peek() == Some(&'/') {
            in_line_comment = true;
            out.push(' ');
            continue;
        }
        if character == '/' && characters.peek() == Some(&'*') {
            characters.next();
            block_depth = 1;
            out.push_str("  ");
            continue;
        }
        if character == '"' {
            in_string = true;
            out.push(' ');
            continue;
        }
        out.push(character);
    }
    out
}

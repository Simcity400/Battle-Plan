#!/usr/bin/env python3
"""C0 domain closure census v3 (PRODUCTION_ENDGAME_PLAN.md Wave C; blocks O8 wiring).

AUDIT INSTRUMENT: the exit code IS the C0 gate, so every path that could exit
0 without positive evidence is a false green. v2 closed the first tooling
gate's 5 CRITICALs; v3 closes the combined gate's 4 CRITICAL + 2 MEDIUM
(tids in ~/.claude/codex_threads.log: c0-tooling-gate, combined-stagea-census).

Fail-closed guarantees (each pinned by --selftest through the REAL entry
path, not a private helper):
  * the pinned roster is cross-checked at startup against the Rust registry
    SOURCE (compact_contract/src/lib.rs); any drift/parse failure blocks
    before any manifest is read (combined-gate finding 1)
  * hourly_requirement must be an object with required=True, met=True,
    problems == [] — absent/older-schema/met-false all block, matching the
    authoritative verification gate (finding 2)
  * strict schema: bools are NOT ints (type() is int), counters nonnegative,
    problems/expected_domains are exact lists, coverage errors exactly [],
    session_id a nonempty unique string (finding 3)
  * duplicate JSON object members anywhere are rejected (finding 4)
  * plus everything from v2: roster/coverage/dark-domain/fold-problem
    blocking, domain-aware bits (d49/d51/d53/d60 lane-local), unknown-bit
    blocking, bit13 validity, ATTRIBUTED policies that never clear C0
    (must_be_zero enforced; candidates label but still block).

Usage: python c0_domain_census.py [manifest.json ...]
       python c0_domain_census.py --selftest
       (no args: the built-in representative set)
"""
import json
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
RUNS = os.path.normpath(os.path.join(HERE, "..", "runs"))
REGISTRY_SRC = os.path.normpath(
    os.path.join(HERE, "..", "compact_contract", "src", "lib.rs"))

DEFAULT_MANIFESTS = [
    os.path.join(RUNS, "production_speed5_ShortSpeed5_20260711_113625",
                 "native_session_manifest.json"),
    os.path.join(RUNS, "production_speed5_ShortSpeed5_20260711_142948",
                 "native_session_manifest.json"),
]

# --- Pinned roster, cross-checked at startup against REGISTRY_SRC ----------
# wire_name -> (domain_id, in_production_profile)
ROSTER = {
    "clock": (1, False),
    "fuel_live": (10, True), "production_live": (11, True),
    "focus_live": (12, True), "tech_live": (13, True),
    "politics_live": (14, True), "politics_residuals": (15, True),
    "decisions_live": (16, True), "diplomacy_live": (17, True),
    "country_identity": (18, False),  # ActiveIntegration: profile-excluded
    "divisions": (20, True), "land_combats": (21, True),
    "map_control": (22, True), "map_detail": (23, True),
    "weather_live": (24, True), "division_equipment": (26, True),
    "army_orders": (27, True), "oob_command": (28, True),
    "ships": (29, True), "air_wings": (30, True),
    "air_region_activity": (31, True), "air_wing_loadouts": (32, True),
    "air_variant_defs": (33, True), "air_base_status": (34, True),
    "air_variant_stats": (35, True), "air_variant_reserves": (36, True),
    "stockpile_variant_defs": (37, True), "air_superiority_inputs": (38, True),
    "air_variant_upgrades": (39, True), "stockpile_variant_upgrades": (40, True),
    "projects_live": (41, True), "air_combat_losses_live": (42, True),
    "market_trade_live": (43, True),
    "supply_logistics_attrition_live": (44, True),
    "focus_research_decisions_depth_live": (45, True),
    "air_variant_archetypes": (46, True),
    "characters_leaders_cabinet_live": (47, True),
    "construction_buildings_state_damage_live": (48, True),
    "naval_combat_lifecycle_live": (49, True),
    "naval_combat_members_live": (50, True),
    "scripted_state_flags_variables_live": (51, True),
    "occupation_resistance_compliance_live": (52, True),
    "diplomacy_intel_operatives_live": (53, True),
    "army_battleplans_depth_live": (54, True),
    "land_combat_detail_events_live": (55, True),
    "naval_missions_status_live": (56, True),
    "session_multiplayer_metadata_live": (57, True),
    "country_economy_counters_live": (58, False),
    "templates_equipment_catalog_live": (59, False),
    "capitulation_war_state_live": (60, True),
    "air_loss_stats_live": (61, True), "hub_motorization_live": (62, True),
    "country_core_scalars_live": (63, True),
    "delayed_events_live": (64, True),
    "special_project_programs_live": (65, True),
    "occupation_state_extras_live": (66, True),
    "country_dynamic_modifiers_live": (67, True),
    "session_header_live": (68, True),
    "tech_sharing_groups_live": (69, True),
    "frame": (1000, False),
}
PROFILE = {n: i for n, (i, p) in ROSTER.items() if p}
PROFILE_EXCLUDED_NOTE = {
    "country_identity": "ActiveIntegration (documented profile exclusion; "
                        "pending integration - NOT silently dark)",
}
PROFILE_STATUSES = {
    "Existing", "ProductionComplete", "ProductionCompleteBase",
    "ValidatedSlice", "ValidatedPartialSlice", "ValidatedSliceOpenGaps",
}
# Round-3/4 gate finding 1: the parse must see what the COMPILER sees.
# Round 4 killed the regex comment-strip: Rust block comments NEST
# (`/* /* */ */` hides an entry from rustc but not from a non-nesting
# regex), and a block comment OPENED BEFORE the declaration made the raw
# `find` select a commented-out copy. The lexer below is character-level:
# nested block comments, line comments, string/char literals with escapes,
# and lifetimes (`&'static` is not a char literal) are all handled; comments
# become spaces so nothing inside them can match, and the ACTIVE declaration
# is located in the stripped text.
REGISTRY_ARRAY_DECL = "pub const ASSIGNED_COMPACT_DOMAINS:"
REGISTRY_ENTRY_RE = re.compile(
    r'domain\(\s*(\d+),\s*"([^"]+)",\s*"([^"]+)",\s*'
    r'CompactDomainStatus::(\w+),\s*(true|false),\s*(true|false),\s*'
    r'&\[[^\]]*\]\s*,?\s*\)')
# The `= &[` value opener (round-6: distinguish the array BODY from the type
# annotation `&[CompactDomainDescriptor]`; anchored on `=` so the type's own
# `&[` never matches).
REGISTRY_ARRAY_OPEN_RE = re.compile(r'=\s*&\s*\[')
CHAR_LITERAL_RE = re.compile(r"'(\\.|\\u\{[0-9a-fA-F_]+\}|[^\\'])'")
# All profile domains require data records (pinned: only clock/d59/frame
# carry requires_data_record=false, all non-profile). A registry change here
# must drift so the dark-domain check gets re-judged.
REQUIRES_DATA_FALSE = {"clock", "templates_equipment_catalog_live", "frame"}


def strip_rust_comments(source):
    """Blank every comment (nested block + line) to spaces, preserving
    newlines and string/char literal contents. Character-level so the result
    matches what rustc compiles; a truncation-relevant `];` inside a STRING
    is deliberately preserved (it can only shrink the scope -> fewer entries
    -> drift -> fail closed)."""
    out = []
    i, n = 0, len(source)
    depth = 0
    in_line = in_str = False
    while i < n:
        c = source[i]
        pair = source[i:i + 2]
        if in_line:
            if c == "\n":
                in_line = False
                out.append(c)
            else:
                out.append(" ")
            i += 1
        elif depth > 0:
            if pair == "/*":
                depth += 1
                out.append("  ")
                i += 2
            elif pair == "*/":
                depth -= 1
                out.append("  ")
                i += 2
            else:
                out.append(c if c == "\n" else " ")
                i += 1
        elif in_str:
            out.append(c)
            if c == "\\" and i + 1 < n:
                out.append(source[i + 1])
                i += 2
                continue
            if c == '"':
                in_str = False
            i += 1
        else:
            if pair == "//":
                in_line = True
                out.append("  ")
                i += 2
            elif pair == "/*":
                depth = 1
                out.append("  ")
                i += 2
            elif c == '"':
                in_str = True
                out.append(c)
                i += 1
            elif c == "'":
                literal = CHAR_LITERAL_RE.match(source, i)
                if literal:
                    out.append(literal.group(0))
                    i = literal.end()
                else:
                    # A lifetime ('static, 'a): just the tick.
                    out.append(c)
                    i += 1
            else:
                out.append(c)
                i += 1
    return "".join(out)


def hash_outside_strings(scoped):
    """True when a `#` occurs outside string literals — an attribute like
    #[cfg(debug_assertions)] inside the array means the compiled registry can
    differ per build profile, which a textual pin cannot evaluate (round-5
    gate CRITICAL: a cfg'd entry compiles in test builds — keeping the Rust
    exact-ID test green — while release omits it)."""
    in_str = False
    i, n = 0, len(scoped)
    while i < n:
        c = scoped[i]
        if in_str:
            if c == "\\":
                i += 2
                continue
            if c == '"':
                in_str = False
        elif c == '"':
            in_str = True
        elif c == "#":
            return True
        i += 1
    return False


def registry_scope(registry_text):
    """(scoped_array_text, None) or (None, reason). Fail closed on anything
    a textual parse cannot faithfully evaluate."""
    stripped = strip_rust_comments(registry_text)
    count = stripped.count(REGISTRY_ARRAY_DECL)
    if count != 1:
        return None, (f"ASSIGNED_COMPACT_DOMAINS declaration count {count} "
                      "!= 1 (hidden/renamed/ambiguous)")
    tail = stripped[stripped.find(REGISTRY_ARRAY_DECL):]
    end = tail.find("];")
    if end < 0:
        return None, "array unterminated"
    scoped = tail[:end]
    if hash_outside_strings(scoped):
        return None, ("attribute (#[...]) inside the registry array - "
                      "conditional compilation is not textually evaluable")
    # Round-6 gate CRITICAL: the `#` scan closes in-array attributes, but the
    # entry regex previously never proved it consumed the ENTIRE array body.
    # A non-literal element the pin cannot evaluate -- a wrapper macro
    # (conditional_domain!(domain(...))), a bare const name (EXTRA_DOMAIN,),
    # an include!("..."), or any additive expression producing a descriptor --
    # can compile-differ between test and release while the regex still
    # extracts an inner literal domain(...). Require FULL grammatical
    # consumption: after removing the array opener and every literal domain()
    # entry, only commas and whitespace may remain. Any other token = parse
    # shortfall = fail closed.
    opener = REGISTRY_ARRAY_OPEN_RE.search(scoped)
    if opener is None:
        return None, "registry array opener `= &[` not found"
    body = scoped[opener.end():]
    residue = REGISTRY_ENTRY_RE.sub("", body).replace(",", "").strip()
    if residue:
        snippet = " ".join(residue.split())[:60]
        return None, ("registry array body has non-literal tokens the pin "
                      "cannot evaluate (wrapper/additive/const/include "
                      f"expression): {snippet!r}")
    return scoped, None


def registry_entries(registry_text):
    """Active domain() calls exactly as the compiler sees them, or None on
    parse shortfall (= drift)."""
    scoped, _reason = registry_scope(registry_text)
    return None if scoped is None else REGISTRY_ENTRY_RE.findall(scoped)


def registry_drift(registry_text):
    """Compare ROSTER against the Rust registry source. Returns a list of
    drift findings (empty = exact match). Any parse shortfall is drift:
    an audit pin that cannot be verified is not a pin."""
    scoped, reason = registry_scope(registry_text)
    if scoped is None:
        return [f"registry parse shortfall (fail closed): {reason}"]
    entries = REGISTRY_ENTRY_RE.findall(scoped)
    findings = []
    if len(entries) != len(ROSTER):
        findings.append(f"registry has {len(entries)} active domain() "
                        f"entries, pinned roster has {len(ROSTER)}")
    seen = {}
    for domain_id, _internal, wire, status, schedulable, data in entries:
        domain_id = int(domain_id)
        in_profile = schedulable == "true" and status in PROFILE_STATUSES
        seen[wire] = (domain_id, in_profile)
        pinned = ROSTER.get(wire)
        if pinned is None:
            findings.append(f"registry domain {wire} (d{domain_id}) missing "
                            "from the pinned roster")
        elif pinned != (domain_id, in_profile):
            findings.append(f"pin mismatch for {wire}: pinned {pinned}, "
                            f"registry ({domain_id}, {in_profile})")
        if (data == "false") != (wire in REQUIRES_DATA_FALSE):
            findings.append(f"requires_data_record drift for {wire}: the "
                            "dark-domain check's assumptions changed")
    for wire in sorted(set(ROSTER) - set(seen)):
        findings.append(f"pinned domain {wire} absent from the registry")
    return findings


# --- Status bit model (compact_contract lib.rs:132-226) --------------------
STATUS_BITS = {
    0: "STATE_UNAVAILABLE", 1: "PROFILE_MISMATCH", 2: "NO_CLOCK",
    3: "NO_SCALAR_SOURCE", 4: "WAR_BRIDGE_APPLIED", 5: "EXTRACTION_PERF_GATE",
    6: "WORKER_PANIC", 7: "BRIDGE_UNAVAILABLE", 8: "SOURCE_UNAVAILABLE",
    9: "PARTIAL_DATA", 10: "DELTA_DATA", 11: "TRANSITION_BASELINE_RESET",
    12: "CONTROL_INVALID", 13: "WAR_SEGMENT_ABSENT",
}
FAILURE_BITS = {0, 1, 2, 3, 6, 7, 8, 9, 12}
INFORMATIONAL_BITS = {4, 5, 10}
MARKER_BITS = {11}
BIT13_VALID_DOMAINS = {"diplomacy_live", "diplomacy_intel_operatives_live"}
KNOWN_MASK = (1 << 14) - 1

# Lane-local overlays (d49/d51: compact_contract lib.rs:181-205; d53:
# diplomacy_intel_operatives_live/domain44_compact.rs; d60:
# capitulation_war_state_live/src/compact.rs).
LANE_LOCAL = {
    "naval_combat_lifecycle_live": {
        11: ("LANE:LOCATION_CANDIDATE_UNVALIDATED", True),
    },
    "scripted_state_flags_variables_live": {
        11: ("LANE:FLAG_NAMES_PARTIAL", True),
        12: ("LANE:DELAYED_EVENTS_ROOT_UNRESOLVED", True),
        13: ("LANE:SCRIPTED_GUI_ROOT_UNRESOLVED", True),
    },
    "diplomacy_intel_operatives_live": {
        16: ("LANE:OPERATIONS_DETAIL_RESISTED", True),
        17: ("LANE:INTEL_MAP_RESISTED", True),
        18: ("LANE:WAR_ROOT_UNVALIDATED", True),
        19: ("LANE:INCOMING_ACTION_QUEUE_UNVALIDATED", True),
        20: ("LANE:WAR_LIFECYCLE_DETAIL_UNVALIDATED", True),
        21: ("LANE:FACTION_GROUP_UNVALIDATED", True),
        22: ("LANE:INTEL_NETWORK_GRAPH_UNVALIDATED", True),
    },
    "capitulation_war_state_live": {
        # MANDATORY semantics badge whenever war rows exist - informational.
        16: ("LANE:WAR_CASUALTIES_RUNNING_ACCUMULATOR", False),
    },
}

# --- Attribution: NEVER clears C0. -----------------------------------------
# (wire_name, bit) -> (policy, attribution, note)
ATTRIBUTED = {
    ("market_trade_live", 8): ("candidate",
        "d43 cold first sweep frame (elapsed-0 warmup)",
        "reconcile vs so_d43_source_empty_cold/warm_known in pass 2"),
    ("characters_leaders_cabinet_live", 8): ("candidate",
        "d47 cold-start frames before first complete sweep+prewarm", ""),
    ("characters_leaders_cabinet_live", 7): ("candidate",
        "d47 early frames pre-prewarm", ""),
    ("characters_leaders_cabinet_live", 9): ("candidate",
        "d47 dead-root churn (~1/509h post-dedup), PARTIAL-upsert deferred "
        "496e6e9a9", ""),
    ("diplomacy_intel_operatives_live", 13): ("candidate",
        "O7 accounting: war_relation_bridge absent by design in production",
        "informational bit; valid on this domain"),
    ("tech_sharing_groups_live", 9): ("must_be_zero",
        "HISTORIC vacant/tombstone member bug, FIXED 2026-07-11 "
        "(ebdc5bc4b/1fbd9f3a4)",
        "post-fix incidence = regression"),
    ("land_combat_detail_events_live", 11): ("candidate",
        "d55 delta baseline reset marker (so_d55_transition_baseline_reset)",
        "marker bit, non-failure; reconcile counts in pass 2"),
    ("focus_research_decisions_depth_live", 13): ("candidate",
        "d45 ORs a dependency bundle's flags (collector lib.rs ~39359), "
        "bleeding d17-only WAR_SEGMENT_ABSENT into its own word",
        "combined-gate confirmed real bleed; fix = mask dependency bits"),
}


def strict_int(value):
    return type(value) is int


def strict_nonneg(value):
    return type(value) is int and value >= 0


PER_DOMAIN_REQUIRED = {
    "domain_id": strict_int,
    "record_count": strict_nonneg,
    "data_record_count": strict_nonneg,
    "row_count_total": strict_nonneg,
    "status_flags_or": strict_nonneg,
    "world_folded": lambda v: type(v) is bool,
}


def reject_duplicate_keys(pairs):
    """object_pairs_hook: a manifest with duplicate members is malformed
    evidence (combined finding 4: last-wins let a clean duplicate shadow a
    failing first member)."""
    obj = {}
    for key, value in pairs:
        if key in obj:
            raise ValueError(f"duplicate JSON object member {key!r}")
        obj[key] = value
    return obj


def string_list(value):
    return isinstance(value, list) and all(type(e) is str for e in value)


def classify_bits(name, flags, blocking, notes):
    """Domain-aware split of a status word into findings."""
    lane = LANE_LOCAL.get(name, {})
    lane_mask = sum(1 << bit for bit in lane)
    unknown = flags & ~(KNOWN_MASK | lane_mask)
    if unknown:
        blocking.append(f"{name}: UNKNOWN status bits 0x{unknown:x} "
                        "(contract drift - census cannot classify)")
    for bit in sorted(set(range(14)) | set(lane)):
        if not flags & (1 << bit):
            continue
        if bit in lane:
            lane_name, is_blocking = lane[bit]
            entry = f"{name}: bit{bit} {lane_name} (lane-local)"
            (blocking if is_blocking else notes).append(entry + (
                "" if is_blocking else " (informational)"))
            continue
        tag = STATUS_BITS[bit]
        att = ATTRIBUTED.get((name, bit))
        if bit == 13 and name not in BIT13_VALID_DOMAINS and att is None:
            blocking.append(f"{name}: bit13 {tag} on a domain that never "
                            "emits it - INVALID domain/bit combination")
            continue
        if att:
            policy, cause, note = att
            if policy == "must_be_zero":
                blocking.append(f"{name}: bit{bit} {tag} REGRESSION - "
                                f"must-be-zero pair fired ({cause})")
            else:
                blocking.append(f"{name}: bit{bit} {tag} attributed-candidate "
                                f"({cause}) - blocks until per-origin "
                                f"reconciliation{'; ' + note if note else ''}")
            continue
        if bit in FAILURE_BITS:
            blocking.append(f"{name}: bit{bit} {tag} *** UNEXPLAINED ***")
        elif bit in MARKER_BITS:
            blocking.append(f"{name}: bit{bit} {tag} marker - unattributed")
        else:
            notes.append(f"{name}: bit{bit} {tag} (informational)")


def census_manifest(manifest, label, blocking, notes):
    """Validate + census one parsed manifest. Appends findings in place."""
    cs = manifest.get("compact_stream")
    if not isinstance(cs, dict):
        blocking.append(f"{label}: compact_stream missing/not an object")
        return None
    for field in ("decodable", "ordered"):
        if cs.get(field) is not True:
            blocking.append(f"{label}: compact_stream.{field} is not true")
    coverage = cs.get("domain_coverage")
    if not isinstance(coverage, dict) or coverage.get("ok") is not True:
        blocking.append(f"{label}: domain_coverage missing or not ok: "
                        f"{coverage!r}")
    else:
        if coverage.get("errors") != []:
            blocking.append(f"{label}: domain_coverage.errors not exactly "
                            f"[]: {coverage.get('errors')!r}")
        count = coverage.get("expected_domain_count")
        if not strict_int(count) or count != len(PROFILE):
            blocking.append(f"{label}: expected_domain_count {count!r} is "
                            f"not exactly the integer {len(PROFILE)} "
                            "(roster drift)")
    expected = cs.get("expected_domains")
    # Length AND set equality: a duplicate-padded list must not pass via set
    # collapse (round-3 gate finding 3).
    if (not string_list(expected) or len(expected) != len(PROFILE)
            or set(expected) != set(PROFILE)):
        missing = sorted(set(PROFILE) - set(expected or []))
        extra = sorted(set(e for e in (expected or []) if type(e) is str)
                       - set(PROFILE))
        blocking.append(f"{label}: expected_domains != pinned profile "
                        f"(len {len(expected) if isinstance(expected, list) else 'n/a'}, "
                        f"missing {missing}, extra {extra})")
    # Round-3 gate finding 2: parse/ordering evidence must be POSITIVELY
    # empty — absent/null/mistyped is unproven cleanliness, not cleanliness.
    for field in ("parse_errors", "ordering_errors"):
        errors = manifest.get(field, cs.get(field))
        if errors != []:
            blocking.append(f"{label}: {field} not exactly [] "
                            f"(absent/mistyped blocks): {errors!r}")
    # Hourly gate (combined finding 2): must POSITIVELY prove required+met
    # with zero problems - absent/older-schema/met-false all block, matching
    # the authoritative verification gate.
    hourly = manifest.get("hourly_requirement")
    if (not isinstance(hourly, dict)
            or hourly.get("required") is not True
            or hourly.get("met") is not True
            or hourly.get("problems") != []):
        blocking.append(f"{label}: hourly_requirement does not positively "
                        f"prove required+met with zero problems: {hourly!r}")
    per_domain = cs.get("per_domain")
    if not isinstance(per_domain, dict) or not per_domain:
        blocking.append(f"{label}: per_domain missing/empty")
        return None

    seen = set(per_domain)
    for name in sorted(set(PROFILE) - seen):
        blocking.append(f"{label}: profile domain {name} ABSENT from "
                        "per_domain (dark domain)")
    for name in sorted(seen - set(ROSTER)):
        blocking.append(f"{label}: unknown domain {name} in per_domain "
                        "(roster drift)")
    for name in sorted(seen & set(ROSTER)):
        if name not in PROFILE:
            blocking.append(f"{label}: non-profile domain {name} emitted "
                            "records (roster/profile drift)")

    for name in sorted(seen & set(PROFILE)):
        d = per_domain[name]
        if not isinstance(d, dict):
            blocking.append(f"{label}/{name}: per_domain entry not an object")
            continue
        bad = [f for f, check in PER_DOMAIN_REQUIRED.items()
               if not check(d.get(f))]
        if bad:
            blocking.append(f"{label}/{name}: missing/mistyped fields {bad}")
            continue
        problems = d.get("problems")
        if not isinstance(problems, (list, dict)):
            blocking.append(f"{label}/{name}: problems not a list/object: "
                            f"{problems!r}")
            problems = []
        if d["domain_id"] != PROFILE[name]:
            blocking.append(f"{label}/{name}: domain_id {d['domain_id']} != "
                            f"registry {PROFILE[name]}")
        if d["record_count"] <= 0 or d["data_record_count"] <= 0:
            blocking.append(f"{label}/{name}: DARK domain "
                            f"(records={d['record_count']}, "
                            f"data={d['data_record_count']})")
        # world_folded is STATIC fold-support membership; all profile domains
        # are fold-supported, so False is drift.
        if d["world_folded"] is not True:
            blocking.append(f"{label}/{name}: fold_supported=false on a "
                            "profile domain (FOLDED_DOMAIN_IDS drift)")
        for pr in problems:
            blocking.append(f"{label}/{name}: fold problem: {pr}")
        classify_bits(name, d["status_flags_or"], blocking, notes)
    return cs


def print_status_origins(cs, label):
    origins = (cs or {}).get("status_origins")
    if isinstance(origins, dict) and origins:
        print(f"  status-origin ledger ({label}): "
              f"{len(origins)} units (pass-2 reconciliation evidence)")
        for unit, agg in sorted(origins.items()):
            print(f"    {unit}: {agg}")


def evaluate_source(label, text, session_ids, blocking, notes, quiet=False):
    """The REAL evaluation path for one manifest source (selftest enters
    here too - combined finding 6): duplicate-key-safe parse, session-id
    validation, then census. Returns the compact_stream or None."""
    try:
        manifest = json.loads(text, object_pairs_hook=reject_duplicate_keys)
    except ValueError as e:
        blocking.append(f"{label}: manifest unreadable/malformed: {e}")
        return None
    if not isinstance(manifest, dict):
        blocking.append(f"{label}: manifest is not an object")
        return None
    sid = manifest.get("session_id")
    if type(sid) is not str or not sid:
        blocking.append(f"{label}: session_id missing/not a nonempty string")
    elif sid in session_ids:
        blocking.append(f"{label}: duplicate session_id {sid!r}")
    else:
        session_ids.add(sid)
    cs = census_manifest(manifest, label, blocking, notes)
    if not quiet:
        print(f"censused {label} (session {sid!r})")
        print_status_origins(cs, label)
    return cs


def run_paths(paths, registry_text, quiet=False):
    """Full run: registry cross-check, then every manifest. Returns
    (blocking, notes)."""
    blocking, notes = [], []
    for drift in registry_drift(registry_text):
        blocking.append(f"ROSTER DRIFT (audit pin unverifiable): {drift}")
    if blocking:
        # A wrong pin invalidates every downstream judgment - stop here.
        return blocking, notes
    session_ids = set()
    usable = 0
    for p in paths:
        label = os.path.basename(os.path.dirname(p)) or p
        try:
            with open(p, encoding="utf-8") as f:
                text = f.read()
        except OSError as e:
            blocking.append(f"{label}: manifest unreadable: {e}")
            continue
        evaluate_source(label, text, session_ids, blocking, notes,
                        quiet=quiet)
        usable += 1
    if usable == 0:
        blocking.append("ZERO usable manifests (audit ran against nothing)")
    return blocking, notes


def main(argv):
    if argv and argv[0] == "--selftest":
        return selftest()
    paths = argv or DEFAULT_MANIFESTS
    try:
        with open(REGISTRY_SRC, encoding="utf-8") as f:
            registry_text = f.read()
    except OSError as e:
        print(f"BLOCKING: registry source unreadable ({e}); the roster pin "
              "cannot be verified")
        return 1
    blocking, notes = run_paths(paths, registry_text)
    for name, note in PROFILE_EXCLUDED_NOTE.items():
        print(f"profile-excluded (visible, non-blocking): d{ROSTER[name][0]} "
              f"{name} - {note}")
    if notes:
        print(f"\n--- informational ({len(notes)}) ---")
        for n in notes:
            print(f"  {n}")
    if blocking:
        print(f"\n=== BLOCKING findings ({len(blocking)}) - C0 NOT closed ===")
        for b in blocking:
            print(f"  {b}")
        return 1
    print("\n=== C0 CLOSED on this set: full profile coverage, zero "
          "failure/marker incidence, zero fold problems ===")
    return 0


# --- Self-test: every claim above, exercised through the REAL paths --------
def selftest():
    global REGISTRY_SRC

    def domain(name, **kw):
        base = {"domain_id": PROFILE[name], "record_count": 10,
                "data_record_count": 5, "row_count_total": 100,
                "status_flags_or": 0, "world_folded": True, "problems": []}
        base.update(kw)
        return base

    def full_manifest(mutate=None, session="s1"):
        m = {"session_id": session,
             "parse_errors": [], "ordering_errors": [],
             "hourly_requirement": {"met": True, "problems": [],
                                    "required": True},
             "compact_stream": {
                 "decodable": True, "ordered": True,
                 "domain_coverage": {"ok": True, "errors": [],
                                     "expected_domain_count": len(PROFILE)},
                 "expected_domains": sorted(PROFILE),
                 "per_domain": {n: domain(n) for n in PROFILE}}}
        if mutate:
            mutate(m)
        return m

    def run_texts(*texts):
        blocking, notes = [], []
        session_ids = set()
        for index, text in enumerate(texts):
            evaluate_source(f"self{index}", text, session_ids, blocking,
                            notes, quiet=True)
        return blocking

    def run_m(mutate=None):
        return run_texts(json.dumps(full_manifest(mutate)))

    def pd(m):
        return m["compact_stream"]["per_domain"]

    registry_text = open(REGISTRY_SRC, encoding="utf-8").read()
    cases = []
    # Clean manifest through the REAL path -> zero blocking.
    cases.append(("clean", run_m(), 0))
    # v2 pins (tooling-gate findings).
    cases.append(("dark-domain", run_m(
        lambda m: pd(m).pop("fuel_live")), 1))
    cases.append(("zero-data", run_m(
        lambda m: pd(m)["fuel_live"].update(data_record_count=0)), 1))
    cases.append(("bad-stream", run_m(lambda m: (
        m["compact_stream"].update(decodable=False),
        m["compact_stream"]["domain_coverage"].update(ok=False))), 2))
    cases.append(("fold-problem", run_m(
        lambda m: pd(m)["divisions"].update(
            problems=["delta_before_baseline"])), 1))
    cases.append(("unknown-bit", run_m(
        lambda m: pd(m)["fuel_live"].update(status_flags_or=1 << 14)), 1))
    cases.append(("d51-lane-bit13", run_m(
        lambda m: pd(m)["scripted_state_flags_variables_live"].update(
            status_flags_or=1 << 13)), 1))
    cases.append(("d69-regression", run_m(
        lambda m: pd(m)["tech_sharing_groups_live"].update(
            status_flags_or=1 << 9)), 1))
    cases.append(("candidate-blocks", run_m(
        lambda m: pd(m)["characters_leaders_cabinet_live"].update(
            status_flags_or=1 << 9)), 1))
    cases.append(("d17-bit13-informational", run_m(
        lambda m: pd(m)["diplomacy_live"].update(
            status_flags_or=1 << 13)), 0))
    # v3: d53 lane badge blocks; d60 badge informational.
    cases.append(("d53-lane-bit16", run_m(
        lambda m: pd(m)["diplomacy_intel_operatives_live"].update(
            status_flags_or=1 << 16)), 1))
    cases.append(("d60-badge-informational", run_m(
        lambda m: pd(m)["capitulation_war_state_live"].update(
            status_flags_or=1 << 16)), 0))
    # combined finding 2: hourly evidence is mandatory + positive.
    cases.append(("hourly-absent", run_m(
        lambda m: m.pop("hourly_requirement")), 1))
    cases.append(("hourly-wrong-type", run_m(
        lambda m: m.update(hourly_requirement=[])), 1))
    cases.append(("hourly-met-false", run_m(
        lambda m: m.update(hourly_requirement={
            "required": True, "met": False, "problems": []})), 1))
    # combined finding 3: strict types.
    cases.append(("bool-counts", run_m(
        lambda m: pd(m)["fuel_live"].update(
            record_count=True, data_record_count=True)), 1))
    cases.append(("negative-count", run_m(
        lambda m: pd(m)["fuel_live"].update(row_count_total=-1)), 1))
    cases.append(("problems-false", run_m(
        lambda m: pd(m)["fuel_live"].update(problems=False)), 1))
    cases.append(("coverage-errors-nonempty", run_m(
        lambda m: m["compact_stream"]["domain_coverage"].update(
            errors=["missing d10"])), 1))
    cases.append(("session-id-missing", run_m(
        lambda m: m.pop("session_id")), 1))
    # combined finding 4: duplicate JSON members are malformed evidence.
    clean = json.dumps(full_manifest())
    dup_member = clean.replace(
        '"decodable": true', '"decodable": false, "decodable": true', 1)
    cases.append(("duplicate-json-member", run_texts(dup_member), 1))
    # Duplicate sessions across manifests.
    cases.append(("duplicate-session", run_texts(
        json.dumps(full_manifest()), json.dumps(full_manifest())), 1))
    # combined finding 1: roster drift vs registry blocks before manifests.
    synthetic_registry = registry_text.replace(
        'domain(\n        69,', 'domain(\n        70,', 1)
    drift_blocking, _ = run_paths([], synthetic_registry, quiet=True)
    cases.append(("registry-drift",
                  [b for b in drift_blocking if "ROSTER DRIFT" in b], 1))
    # Real registry matches the pin exactly.
    cases.append(("registry-exact", registry_drift(registry_text), 0))
    # Round-3 finding 1: a BLOCK-COMMENTED active entry must drift (the
    # compiler would build 59 descriptors while the raw-text regex saw 60).
    entry = re.search(r"domain\(\s*10,.*?\),", registry_text, re.S)
    commented = (registry_text[:entry.start()] + "/*"
                 + registry_text[entry.start():entry.end()] + "*/"
                 + registry_text[entry.end():])
    cases.append(("registry-commented-entry", registry_drift(commented), 2))
    # ...and an extra trailing argument (arity change) must drift, never
    # silently prefix-match. Round-6: the arity-changed entry no longer matches
    # the literal 7-arg domain() form, so the full-consumption residue check
    # now fails it closed as a parse shortfall (1 finding) BEFORE the old
    # count-mismatch path -- still drift, caught earlier and more precisely.
    extra_arg = registry_text.replace("&[],\n    ),", "&[],\n        true,\n    ),", 1)
    cases.append(("registry-extra-arg", registry_drift(extra_arg), 1))
    # Round-5 CRITICAL: a cfg-attributed entry compiles conditionally —
    # release can omit it while test builds (and a textual pin) keep it.
    # ANY attribute inside the array = fail closed.
    for attr in ("#[cfg(test)]", "#[cfg(debug_assertions)]"):
        cfg_entry = (registry_text[:entry.start()] + attr + "\n    "
                     + registry_text[entry.start():])
        cases.append((f"registry-attr-{attr[6:-2]}",
                      [d for d in registry_drift(cfg_entry)
                       if "not textually evaluable" in d], 1))
    # Round-4 finding 1a: NESTED block comments hide an entry from rustc;
    # the lexer must agree (a non-nesting strip left the entry visible).
    nested = (registry_text[:entry.start()] + "/* /* x */ "
              + registry_text[entry.start():entry.end()] + " */"
              + registry_text[entry.end():])
    cases.append(("registry-nested-comment", registry_drift(nested), 2))
    # Round-4 finding 1b: the WHOLE original declaration block-commented out
    # with a modified active copy appended — the census must judge the ACTIVE
    # copy (raw-text find previously selected the commented corpse). The
    # lexer is length-preserving, so stripped-text offsets are valid raw
    # offsets — locating the TRUE array end this way (a raw `];` find hits
    # the literal `];` inside the lib.rs:1268 comment and truncates).
    stripped_all = strip_rust_comments(registry_text)
    decl_start = stripped_all.find(REGISTRY_ARRAY_DECL)
    decl_end = stripped_all.find("];", decl_start) + 2
    active_copy = registry_text[decl_start:decl_end].replace(
        "\n        69,", "\n        70,", 1)
    shadowed = (registry_text[:decl_start] + "/*"
                + registry_text[decl_start:decl_end] + "*/\n"
                + active_copy + registry_text[decl_end:])
    cases.append(("registry-shadowed-decl",
                  [d for d in registry_drift(shadowed) if "mismatch" in d],
                  1))
    # Round-6 CRITICAL: the entry regex must not leave non-literal array-body
    # tokens unconsumed. Each of these wraps/augments the array with an
    # element the pin cannot evaluate (all compile-different-from-textual);
    # every one must fail closed via the full-consumption residue check.
    def _wrap_first_entry(prefix, suffix):
        return (registry_text[:entry.start()] + prefix
                + registry_text[entry.start():entry.end() - 1]  # drop entry ','
                + suffix + registry_text[entry.end() - 1:])
    non_literal = {
        # wrapper macro around a real inner domain() literal
        "wrapper-macro": _wrap_first_entry("conditional_domain!(", ")"),
        # additive bare const element
        "additive-const": (registry_text[:entry.start()]
                           + "EXTRA_DOMAIN,\n    "
                           + registry_text[entry.start():]),
        # additive descriptor-producing macro call
        "additive-macro": (registry_text[:entry.start()]
                          + "extra_domain!(),\n    "
                          + registry_text[entry.start():]),
        # additive include! of an out-of-file expression
        "additive-include": (registry_text[:entry.start()]
                            + 'include!("extra_domain_expr.rs"),\n    '
                            + registry_text[entry.start():]),
    }
    for name, mutant in non_literal.items():
        cases.append((f"registry-nonliteral-{name}",
                      [d for d in registry_drift(mutant)
                       if "non-literal tokens" in d], 1))
    # ...and a duplicate-padded expected_domains must not pass via set
    # collapse (round-3 finding 3), nor a float count.
    cases.append(("expected-domains-dup-padded", run_m(
        lambda m: m["compact_stream"].update(
            expected_domains=sorted(PROFILE) + ["fuel_live"])), 1))
    cases.append(("expected-count-float", run_m(
        lambda m: m["compact_stream"]["domain_coverage"].update(
            expected_domain_count=float(len(PROFILE)))), 1))
    # Round-3 finding 2: ABSENT parse/ordering evidence blocks.
    cases.append(("parse-errors-absent", run_m(
        lambda m: m.pop("parse_errors")), 1))
    cases.append(("ordering-errors-null", run_m(
        lambda m: m.update(ordering_errors=None)), 1))
    # Zero usable manifests blocks (through run_paths' real path).
    zero_blocking, _ = run_paths(
        [os.path.join(HERE, "definitely_missing.json")], registry_text,
        quiet=True)
    cases.append(("zero-usable",
                  [b for b in zero_blocking
                   if "ZERO usable" in b or "unreadable" in b], 2))
    # Round-3 finding 4: pin main()'s OWN exit-code gate (the decisive
    # return), including the registry-unreadable arm.
    import tempfile
    with tempfile.TemporaryDirectory() as td:
        clean_path = os.path.join(td, "clean.json")
        with open(clean_path, "w", encoding="utf-8") as f:
            f.write(json.dumps(full_manifest()))
        blocked_path = os.path.join(td, "blocked.json")
        with open(blocked_path, "w", encoding="utf-8") as f:
            f.write(json.dumps(full_manifest(
                lambda m: pd(m).pop("fuel_live"), session="s2")))
        cases.append(("main-clean-exit0",
                      [] if main([clean_path]) == 0 else ["wrong exit"], 0))
        cases.append(("main-blocking-exit1",
                      [] if main([blocked_path]) == 1 else ["wrong exit"], 0))
        # Round-4 finding 2: pin the PROCESS exit code (a dropped sys.exit
        # wrapper printed 47 blockers and exited 0). Subprocess = the real
        # CLI contract.
        import subprocess
        clean_rc = subprocess.run(
            [sys.executable, os.path.abspath(__file__), clean_path],
            capture_output=True).returncode
        blocked_rc = subprocess.run(
            [sys.executable, os.path.abspath(__file__), blocked_path],
            capture_output=True).returncode
        cases.append(("process-clean-exit0",
                      [] if clean_rc == 0 else [f"rc={clean_rc}"], 0))
        cases.append(("process-blocking-exit1",
                      [] if blocked_rc == 1 else [f"rc={blocked_rc}"], 0))
        saved_registry = REGISTRY_SRC
        REGISTRY_SRC = os.path.join(td, "missing_registry.rs")
        try:
            cases.append(("main-registry-unreadable-exit1",
                          [] if main([clean_path]) == 1 else ["wrong exit"],
                          0))
        finally:
            REGISTRY_SRC = saved_registry

    failures = 0
    for name, blocking, want in cases:
        ok = len(blocking) == want
        print(f"selftest {name}: {'ok' if ok else 'FAIL'} "
              f"(blocking={len(blocking)}, want={want})")
        if not ok:
            for b in blocking:
                print(f"    {b}")
            failures += 1
    print(f"selftest: {len(cases) - failures}/{len(cases)} passed")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))

#!/usr/bin/env python3
"""C0 domain closure census v3 (PRODUCTION_ENDGAME_PLAN.md Wave C; blocks O8 wiring).

AUDIT INSTRUMENT: the exit code IS the C0 gate, so every path that could exit
0 without positive evidence is a false green. v2 closed the first tooling
gate's 5 CRITICALs; v3 closes the combined gate's 4 CRITICAL + 2 MEDIUM
(tids in ~/.claude/codex_threads.log: c0-tooling-gate, combined-stagea-census).

Fail-closed guarantees (each pinned by --selftest through the REAL entry
path, not a private helper):
  * the pinned roster is cross-checked at startup against a deterministic
    JSONL dump of the RELEASE-COMPILED Rust registry; build/run/dump failure
    or any drift blocks before any manifest is read
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
       python c0_domain_census.py --build-and-attest [artifact options]
       python c0_domain_census.py --verify-artifact-only [artifact options]
       (no args: the built-in representative set)
"""
import argparse
import hashlib
import io
import json
import os
import shutil
import subprocess
import sys
import tempfile
from dataclasses import dataclass

HERE = os.path.dirname(os.path.abspath(__file__))
NATIVE_ROOT = os.path.normpath(os.path.join(HERE, ".."))
RUNS = os.path.join(NATIVE_ROOT, "runs")
REGISTRY_GOLDEN = os.path.join(NATIVE_ROOT, "c0_registry_golden.json")

ORACLE_PACKAGE = "collector"
PRODUCTION_PACKAGES = ("collector", "recorder_host")
ATTESTATION_SCHEMA_VERSION = 2
BUILD_INPUT_SUFFIXES = {".rs", ".c", ".h", ".toml", ".lock", ".json"}
CARGO_CACHE_TAG_SIGNATURE = "Signature: 8a477f597d28d172789f06886806bc55"

DEFAULT_MANIFESTS = [
    os.path.join(RUNS, "production_speed5_ShortSpeed5_20260711_113625",
                 "native_session_manifest.json"),
    os.path.join(RUNS, "production_speed5_ShortSpeed5_20260711_142948",
                 "native_session_manifest.json"),
]

# --- Pinned roster, cross-checked against the release-compiled registry ----
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
    "sunk_convoy_history_live": (70, False),
    "dynamic_collection_trace_live": (71, True),
    "resolved_tooltip_instances_live": (72, True),
    "politics_tooltip_inputs_live": (73, False),
    "power_balance_presentation_live": (74, False),
    "fired_event_popups_live": (75, True),
    "runtime_text_inputs_live": (76, False),
    "production_render_submits_live": (77, False),
    "design_catalog_context_live": (78, True),
    "hud_surface_activation_live": (79, True),
    "production_map_content_snapshot_live": (80, False),
    "rail_replay_state_live": (81, True),
    "character_presentation_live": (82, False),
    "nuke_stockpile_live": (83, True),
    "scripted_gui_runtime_live": (84, False),
    "selected_designer_identity_live": (85, False),
    "sunk_ship_history_live": (86, False),
    "decision_panel_live": (87, False),
    "active_command_abilities_live": (88, False),
    "national_focus_presentation_live": (89, False),
    "convoy_transport_attack_live": (90, True),
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
# All profile domains require data records (pinned: only clock/d59/frame
# carry requires_data_record=false, all non-profile). A registry change here
# must drift so the dark-domain check gets re-judged.
REQUIRES_DATA_FALSE = {"clock", "templates_equipment_catalog_live", "frame"}


def _failure_detail(completed):
    detail = (completed.stderr or completed.stdout or "").strip()
    return " ".join(detail.split())[:240] or "no diagnostic output"


@dataclass(frozen=True)
class BuildEvidence:
    dump_text: str
    collector_path: str
    collector_facts: tuple
    registry_dump_path: str


def _reported_path(path):
    if type(path) is not str or not path:
        raise ValueError("build-reported artifact path is not a nonempty string")
    if not os.path.isabs(path):
        path = os.path.join(NATIVE_ROOT, path)
    return os.path.abspath(os.path.normpath(path))


def _path_identity(path):
    return os.path.normcase(os.path.abspath(os.path.normpath(path)))


def _source_identity(path):
    return os.path.normcase(os.path.realpath(os.path.abspath(path)))


def _target_is(message, name, kind, source_path, manifest_path):
    if type(message) is not dict or message.get("reason") != "compiler-artifact":
        return False
    target = message.get("target")
    if type(target) is not dict:
        return False
    # Round-10: bind to PACKAGE identity, not just target name/kind/src_path.
    # Cargo lets a target's src_path be configured relative to ANOTHER package's
    # manifest, so a recorder_host-package `registry_dump` pointing at collector's
    # source (compiled against a different contract) would otherwise match. The
    # message's manifest_path MUST resolve to the collector package's Cargo.toml.
    reported_manifest = message.get("manifest_path")
    if type(reported_manifest) is not str:
        return False
    if _source_identity(reported_manifest) != _source_identity(manifest_path):
        return False
    kinds = target.get("kind")
    return (target.get("name") == name
            and type(kinds) is list and kinds
            and all(type(value) is str for value in kinds) and kind in kinds
            and type(target.get("src_path")) is str
            and _source_identity(target["src_path"])
            == _source_identity(source_path))


def _artifact_outputs(message):
    filenames = message.get("filenames")
    if type(filenames) is not list or any(type(path) is not str
                                           for path in filenames):
        raise ValueError("compiler-artifact filenames is not a string array")
    return [_reported_path(path) for path in filenames]


def _read_stderr(stderr_file):
    stderr_file.seek(0)
    try:
        return stderr_file.read().decode("utf-8", errors="strict")
    except UnicodeError as e:
        raise ValueError(f"production build stderr was not UTF-8: {e}") from e


def obtain_compiler_truth(requested_collector_path=None,
                          popen_factory=subprocess.Popen,
                          run_command=subprocess.run,
                          find_executable=shutil.which):
    """Build production's package set once and use only its reported outputs.

    Cargo JSON is parsed as it streams. The unique collector cdylib is the
    artifact attested by callers, and the unique registry_dump executable is
    invoked directly; no second Cargo command can change feature unification or
    redirect the artifact path between the build and the oracle.
    """
    cargo = find_executable("cargo")
    if cargo is None:
        return None, "cargo/toolchain absent from PATH"
    command = [cargo, "build", "--release"]
    for package in PRODUCTION_PACKAGES:
        command.extend(("-p", package))
    command.append("--message-format=json")
    messages = []
    try:
        with tempfile.TemporaryFile(mode="w+b") as stderr_file:
            process = popen_factory(
                command, cwd=NATIVE_ROOT, stdout=subprocess.PIPE,
                stderr=stderr_file, text=True, encoding="utf-8",
                errors="strict")
            if process.stdout is None:
                return None, "production build stdout pipe was unavailable"
            parse_error = None
            try:
                for line_number, raw_line in enumerate(process.stdout, 1):
                    line = raw_line.strip()
                    if not line:
                        continue
                    try:
                        message = json.loads(
                            line, object_pairs_hook=reject_duplicate_keys)
                    except (UnicodeError, ValueError) as e:
                        parse_error = ("production build JSON line "
                                       f"{line_number} was malformed: {e}")
                        continue
                    if (type(message) is not dict
                            or type(message.get("reason")) is not str):
                        parse_error = ("production build JSON line "
                                       f"{line_number} had an unexpected shape")
                        continue
                    messages.append(message)
            except UnicodeError as e:
                parse_error = f"production build JSON was not UTF-8: {e}"
            returncode = process.wait()
            try:
                stderr = _read_stderr(stderr_file)
            except ValueError as e:
                return None, str(e)
    except OSError as e:
        return None, f"production build could not start: {e}"
    except UnicodeError as e:
        return None, f"production build JSON was not UTF-8: {e}"
    if returncode != 0:
        detail = " ".join(stderr.split())[:240] or "no diagnostic output"
        return None, (f"production build failed (exit {returncode}): {detail}")
    if parse_error is not None:
        return None, parse_error

    collector_source = os.path.join(NATIVE_ROOT, "collector", "src", "lib.rs")
    dump_source = os.path.join(
        NATIVE_ROOT, "collector", "src", "bin", "registry_dump.rs")
    collector_manifest = os.path.join(NATIVE_ROOT, "collector", "Cargo.toml")
    collector_messages = [
        message for message in messages
        if _target_is(message, "collector", "cdylib", collector_source,
                      collector_manifest)]
    dump_messages = [
        message for message in messages
        if _target_is(message, "registry_dump", "bin", dump_source,
                      collector_manifest)]
    if len(collector_messages) != 1:
        return None, ("production build reported "
                      f"{len(collector_messages)} collector cdylib artifacts; "
                      "exactly one required")
    if len(dump_messages) != 1:
        return None, ("production build reported "
                      f"{len(dump_messages)} registry_dump binary artifacts; "
                      "exactly one required")
    # Round-10: the attested DLL and the dump we trust must be the SAME nonempty
    # collector package. Both already bind to collector/Cargo.toml via _target_is;
    # cross-check their package_id so the dump can never come from a sibling
    # package that merely reuses collector's source path.
    collector_pid = collector_messages[0].get("package_id")
    dump_pid = dump_messages[0].get("package_id")
    if not (type(collector_pid) is str and collector_pid
            and type(dump_pid) is str and dump_pid
            and collector_pid == dump_pid):
        return None, ("collector cdylib and registry_dump report different or "
                      f"empty package_id ({collector_pid!r} vs {dump_pid!r}); "
                      "both must be the collector package")

    try:
        collector_outputs = _artifact_outputs(collector_messages[0])
        cdylib_names = {"collector.dll", "libcollector.so", "libcollector.dylib"}
        collector_paths = [
            path for path in collector_outputs
            if os.path.basename(path).lower() in cdylib_names]
        if len(collector_paths) != 1:
            raise ValueError(
                "collector compiler-artifact reported "
                f"{len(collector_paths)} shipped cdylib outputs; exactly one required")
        collector_path = collector_paths[0]
        if (requested_collector_path is not None
                and _path_identity(requested_collector_path)
                != _path_identity(collector_path)):
            raise ValueError(
                "externally supplied collector artifact is not the build-reported "
                f"cdylib output: supplied {os.path.abspath(requested_collector_path)}, "
                f"reported {collector_path}")
        collector_facts = _artifact_facts(collector_path)

        dump_message = dump_messages[0]
        dump_outputs = _artifact_outputs(dump_message)
        executable = _reported_path(dump_message.get("executable"))
        if _path_identity(executable) not in {
                _path_identity(path) for path in dump_outputs}:
            raise ValueError(
                "registry_dump executable is not among its compiler-artifact filenames")
        registry_dump_facts = _artifact_facts(executable)
    except (OSError, ValueError) as e:
        return None, f"cannot obtain build's exact artifacts: {e}"

    try:
        dumped = run_command(
            [executable], cwd=NATIVE_ROOT,
            capture_output=True, text=True, encoding="utf-8",
            errors="strict", check=False)
    except OSError as e:
        return None, f"build-reported registry_dump could not run: {e}"
    except UnicodeError as e:
        return None, f"build-reported registry_dump output was not UTF-8: {e}"
    if dumped.returncode != 0:
        return None, (f"build-reported registry_dump failed (exit "
                      f"{dumped.returncode}): "
                      f"{_failure_detail(dumped)}")
    if not dumped.stdout.strip():
        return None, "build-reported registry_dump stdout was empty"
    try:
        if _artifact_facts(executable) != registry_dump_facts:
            raise ValueError("registry_dump changed during its execution")
        if _artifact_facts(collector_path) != collector_facts:
            raise ValueError("collector cdylib changed during registry_dump execution")
    except ValueError as e:
        return None, f"cannot obtain build's exact artifacts: {e}"
    return BuildEvidence(
        dumped.stdout, collector_path, collector_facts, executable), None


DESCRIPTOR_FIELDS = (
    "id", "wire_name", "internal_name", "status", "schedulable",
    "requires_data_record", "tags",
)


def _validate_descriptor(entry, location):
    required = set(DESCRIPTOR_FIELDS)
    if type(entry) is not dict:
        raise ValueError(f"{location} is not a JSON object")
    if set(entry) != required:
        missing = sorted(required - set(entry))
        extra = sorted(set(entry) - required)
        raise ValueError(
            f"{location} descriptor keys differ "
            f"(missing={missing}, extra={extra})")
    if type(entry["id"]) is not int or entry["id"] <= 0:
        raise ValueError(f"{location} id is not a positive int")
    for key in ("wire_name", "internal_name", "status"):
        if type(entry[key]) is not str or not entry[key]:
            raise ValueError(f"{location} {key} is not a nonempty string")
    for key in ("schedulable", "requires_data_record"):
        if type(entry[key]) is not bool:
            raise ValueError(f"{location} {key} is not a bool")
    tags = entry["tags"]
    if (type(tags) is not list
            or any(type(tag) is not str or not tag for tag in tags)):
        raise ValueError(f"{location} tags is not a nonempty-string array")
    return entry


def parse_registry_dump(dump_text):
    """Parse strict one-object-per-line compiler truth, or raise ValueError."""
    if type(dump_text) is not str or not dump_text.strip():
        raise ValueError("release oracle stdout was empty")
    entries = []
    for line_number, line in enumerate(dump_text.splitlines(), 1):
        if not line.strip():
            raise ValueError(f"blank JSONL record at line {line_number}")
        try:
            entry = json.loads(line, object_pairs_hook=reject_duplicate_keys)
        except ValueError as e:
            raise ValueError(f"invalid JSON at line {line_number}: {e}") from e
        entries.append(_validate_descriptor(entry, f"line {line_number}"))
    return entries


def load_registry_golden(path=REGISTRY_GOLDEN):
    """Load the checked-in full-tuple authority with strict JSON semantics."""
    try:
        with open(path, encoding="utf-8") as golden_file:
            golden = json.load(
                golden_file, object_pairs_hook=reject_duplicate_keys)
    except (OSError, UnicodeError, ValueError) as e:
        raise ValueError(f"full registry golden unreadable/malformed: {e}") from e
    if type(golden) is not list or not golden:
        raise ValueError("full registry golden is not a nonempty array")
    return [_validate_descriptor(entry, f"golden entry {index}")
            for index, entry in enumerate(golden)]


def descriptor_uniqueness_findings(entries, label):
    """Reject ambiguous IDs and scheduled-name/alias routing."""
    findings = []
    ids = {}
    scheduled_names = {}
    lookup_names = {}
    for entry in entries:
        wire = entry["wire_name"]
        domain_id = entry["id"]
        prior_wire = ids.get(domain_id)
        if prior_wire is not None:
            findings.append(
                f"{label} duplicates id {domain_id}: {prior_wire!r} and "
                f"{wire!r}")
        else:
            ids[domain_id] = wire

        # matches_scheduled_name() accepts wire_name plus aliases. Check every
        # descriptor, including currently-unschedulable entries, so making one
        # schedulable later cannot silently activate a pre-existing collision.
        local_names = set()
        for name in [wire, *entry["tags"]]:
            if name in local_names:
                findings.append(
                    f"{label} descriptor {wire!r} repeats scheduled name/alias "
                    f"{name!r}")
                continue
            local_names.add(name)
            prior = scheduled_names.get(name)
            if prior is not None and prior != wire:
                findings.append(
                    f"{label} scheduled name/alias {name!r} is shared by "
                    f"{prior!r} and {wire!r}")
            else:
                scheduled_names[name] = wire
        # assigned_descriptor_by_name()/matches_any_name() additionally accept
        # internal_name. Same-descriptor equality is normal; cross-descriptor
        # equality is ambiguous and must fail even if the golden was updated.
        for name in set([wire, entry["internal_name"], *entry["tags"]]):
            prior = lookup_names.get(name)
            if prior is not None and prior != wire:
                findings.append(
                    f"{label} lookup name/alias {name!r} is shared by "
                    f"{prior!r} and {wire!r}")
            else:
                lookup_names[name] = wire
    return findings


def registry_digest(entries):
    canonical = json.dumps(
        entries, ensure_ascii=True, separators=(",", ":"), sort_keys=True)
    return hashlib.sha256(canonical.encode("utf-8")).hexdigest()


def registry_drift(dump_text, golden_loader=load_registry_golden):
    """Compare every compiled descriptor value to the full golden and pins."""
    try:
        entries = parse_registry_dump(dump_text)
        golden = golden_loader()
    except ValueError as e:
        return [f"cannot obtain compiler truth (fail closed): {e}"]

    findings = []
    findings.extend(descriptor_uniqueness_findings(entries, "compiled registry"))
    findings.extend(descriptor_uniqueness_findings(golden, "full registry golden"))
    if len(entries) != len(golden):
        findings.append(
            f"compiled registry has {len(entries)} descriptors, full golden "
            f"has {len(golden)}")
    for index, (compiled, pinned) in enumerate(zip(entries, golden)):
        for field in DESCRIPTOR_FIELDS:
            if compiled[field] != pinned[field]:
                findings.append(
                    f"full descriptor drift at index {index} "
                    f"({pinned['wire_name']!r}) field {field}: golden "
                    f"{pinned[field]!r}, compiled {compiled[field]!r}")

    # Keep the full semantic pins as an independent cross-check.
    if len(entries) != len(ROSTER):
        findings.append(f"compiled registry has {len(entries)} descriptors, "
                        f"semantic roster has {len(ROSTER)}")
    seen = {}
    for entry in entries:
        domain_id = entry["id"]
        wire = entry["wire_name"]
        in_profile = (entry["schedulable"]
                      and entry["status"] in PROFILE_STATUSES)
        if wire in seen:
            findings.append(f"compiled registry duplicates wire name {wire!r}")
        seen[wire] = (domain_id, in_profile)
        pinned = ROSTER.get(wire)
        if pinned is None:
            findings.append(
                f"compiled registry domain {wire} (d{domain_id}) missing "
                "from the semantic roster")
        elif pinned != (domain_id, in_profile):
            findings.append(f"semantic pin mismatch for {wire}: pinned "
                            f"{pinned}, compiled ({domain_id}, {in_profile})")
        if ((not entry["requires_data_record"])
                != (wire in REQUIRES_DATA_FALSE)):
            findings.append(f"requires_data_record drift for {wire}: the "
                            "dark-domain check's assumptions changed")
    for wire in sorted(set(ROSTER) - set(seen)):
        findings.append(f"pinned domain {wire} absent from compiled registry")
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
        11: ("LANE:LOCATION_CANDIDATE_UNVALIDATED", True,
             "Failed/UnvalidatedCandidate"),
    },
    "scripted_state_flags_variables_live": {
        11: ("LANE:FLAG_NAMES_PARTIAL", True,
             "CapturedPartial: textual flag names absent; token/date/value retained"),
        12: ("LANE:DELAYED_EVENTS_ROOT_UNRESOLVED", True,
             "Failed/UnresolvedRoot: country roster or shared timed-effects members not exhaustively classified"),
        13: ("LANE:SCRIPTED_GUI_ROOT_UNRESOLVED", True,
             "Failed/UnresolvedRoot: no measured scripted-GUI root"),
    },
    "diplomacy_intel_operatives_live": {
        16: ("LANE:OPERATIONS_DETAIL_RESISTED", True, "Failed/Resisted"),
        17: ("LANE:INTEL_MAP_RESISTED", True, "Failed/Resisted"),
        18: ("LANE:WAR_ROOT_UNVALIDATED", True, "Failed/UnvalidatedRoot"),
        19: ("LANE:INCOMING_ACTION_QUEUE_UNVALIDATED", True,
             "Failed/UnvalidatedRoot"),
        20: ("LANE:WAR_LIFECYCLE_DETAIL_UNVALIDATED", True,
             "Failed/UnvalidatedDetail"),
        21: ("LANE:FACTION_GROUP_UNVALIDATED", True,
             "Failed/UnvalidatedDetail"),
        22: ("LANE:INTEL_NETWORK_GRAPH_UNVALIDATED", True,
             "Failed/UnvalidatedDetail"),
    },
    "capitulation_war_state_live": {
        # MANDATORY semantics badge whenever war rows exist - informational.
        16: ("LANE:WAR_CASUALTIES_RUNNING_ACCUMULATOR", False,
             "CapturedSemantic: running accumulator"),
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


ATTESTATION_FIELDS = {
    "schema_version", "oracle_package", "production_packages",
    "registry_sha256", "source_sha256", "collector_sha256",
    "collector_size", "collector_path",
}


def _sha256_file(path):
    digest = hashlib.sha256()
    with open(path, "rb") as source:
        while True:
            chunk = source.read(1024 * 1024)
            if not chunk:
                break
            digest.update(chunk)
    return digest.hexdigest()


def _is_cargo_cache_root(path):
    marker = os.path.join(path, "CACHEDIR.TAG")
    try:
        with open(marker, encoding="ascii", errors="strict") as source:
            return source.readline().strip() == CARGO_CACHE_TAG_SIGNATURE
    except (OSError, UnicodeError):
        return False


def build_input_digest(native_root=NATIVE_ROOT):
    """Hash deterministic native compiler inputs, excluding outputs/corpora."""
    inputs = []
    native_root = os.path.abspath(native_root)
    try:
        for root, dirs, files in os.walk(native_root):
            dirs.sort()
            if os.path.abspath(root) == native_root:
                dirs[:] = [d for d in dirs if d != "runs"]
            dirs[:] = [
                directory for directory in dirs
                if not _is_cargo_cache_root(os.path.join(root, directory))]
            for filename in sorted(files):
                suffix = os.path.splitext(filename)[1].lower()
                if (suffix not in BUILD_INPUT_SUFFIXES
                        and filename not in {"build.rs", "rust-toolchain"}):
                    continue
                path = os.path.join(root, filename)
                relative = os.path.relpath(path, native_root).replace(os.sep, "/")
                inputs.append((relative, _sha256_file(path)))
    except OSError as e:
        raise ValueError(f"native build inputs unreadable: {e}") from e
    if not inputs:
        raise ValueError("native build-input set was empty")
    canonical = json.dumps(inputs, ensure_ascii=True, separators=(",", ":"))
    return hashlib.sha256(canonical.encode("utf-8")).hexdigest()


def _artifact_facts(path):
    try:
        if not os.path.isfile(path):
            raise ValueError(f"collector artifact missing/not a file: {path}")
        size = os.path.getsize(path)
        if size <= 0:
            raise ValueError(f"collector artifact is empty: {path}")
        return size, _sha256_file(path)
    except OSError as e:
        raise ValueError(f"collector artifact unreadable: {e}") from e


def _default_attestation_path(collector_path):
    return os.path.join(
        os.path.dirname(os.path.abspath(collector_path)),
        "collector.c0-attestation.json")


def create_artifact_attestation(dump_text, artifact_path, attestation_path,
                                expected_source_hash=None,
                                expected_artifact_facts=None):
    """Bind the exact DLL to compiled registry truth and current source."""
    drift = registry_drift(dump_text)
    if drift:
        return [f"cannot attest collector artifact: {finding}"
                for finding in drift]
    try:
        if _path_identity(artifact_path) == _path_identity(attestation_path):
            raise ValueError("attestation path aliases the collector artifact")
        entries = parse_registry_dump(dump_text)
        size, artifact_hash = _artifact_facts(artifact_path)
        source_hash = build_input_digest()
        if (expected_source_hash is not None
                and source_hash != expected_source_hash):
            raise ValueError(
                "native source changed before attestation write: expected "
                f"{expected_source_hash}, current {source_hash}")
        if (expected_artifact_facts is not None
                and (size, artifact_hash) != expected_artifact_facts):
            raise ValueError(
                "collector artifact changed before attestation write: expected "
                f"{expected_artifact_facts!r}, current {(size, artifact_hash)!r}")
        attestation = {
            "schema_version": ATTESTATION_SCHEMA_VERSION,
            "oracle_package": ORACLE_PACKAGE,
            "production_packages": list(PRODUCTION_PACKAGES),
            "registry_sha256": registry_digest(entries),
            "source_sha256": source_hash,
            "collector_sha256": artifact_hash,
            "collector_size": size,
            "collector_path": os.path.abspath(os.path.normpath(artifact_path)),
        }
        parent = os.path.dirname(os.path.abspath(attestation_path))
        os.makedirs(parent, exist_ok=True)
        fd, temporary = tempfile.mkstemp(
            prefix=".c0-attestation-", suffix=".tmp", dir=parent, text=True)
        try:
            with os.fdopen(fd, "w", encoding="utf-8", newline="\n") as output:
                json.dump(attestation, output, sort_keys=True, separators=(",", ":"))
                output.write("\n")
            os.replace(temporary, attestation_path)
        except BaseException:
            try:
                os.unlink(temporary)
            except OSError:
                pass
            raise
    except (OSError, ValueError) as e:
        return [f"collector artifact attestation could not be written: {e}"]
    return []


def _load_artifact_attestation(path):
    try:
        with open(path, encoding="utf-8") as source:
            value = json.load(source, object_pairs_hook=reject_duplicate_keys)
    except (OSError, UnicodeError, ValueError) as e:
        raise ValueError(f"attestation unreadable/malformed: {e}") from e
    if type(value) is not dict or set(value) != ATTESTATION_FIELDS:
        keys = set(value) if type(value) is dict else set()
        raise ValueError(
            "attestation fields differ "
            f"(missing={sorted(ATTESTATION_FIELDS - keys)}, "
            f"extra={sorted(keys - ATTESTATION_FIELDS)})")
    if type(value["schema_version"]) is not int:
        raise ValueError("attestation schema_version is not an int")
    if type(value["oracle_package"]) is not str:
        raise ValueError("attestation oracle_package is not a string")
    if (type(value["production_packages"]) is not list
            or any(type(item) is not str
                   for item in value["production_packages"])):
        raise ValueError("attestation production_packages is not a string array")
    for field in ("registry_sha256", "source_sha256", "collector_sha256"):
        digest = value[field]
        if (type(digest) is not str or len(digest) != 64
                or any(ch not in "0123456789abcdef" for ch in digest)):
            raise ValueError(f"attestation {field} is not lowercase SHA-256")
    if type(value["collector_size"]) is not int or value["collector_size"] <= 0:
        raise ValueError("attestation collector_size is not a positive int")
    collector_path = value["collector_path"]
    if (type(collector_path) is not str or not collector_path
            or not os.path.isabs(collector_path)):
        raise ValueError("attestation collector_path is not an absolute path")
    return value


def artifact_attestation_findings(dump_text, artifact_path, attestation_path):
    """Verify source, full registry, and exact DLL against strict evidence."""
    drift = registry_drift(dump_text)
    if drift:
        return [f"artifact registry truth invalid: {finding}"
                for finding in drift]
    try:
        entries = parse_registry_dump(dump_text)
        attestation = _load_artifact_attestation(attestation_path)
        attested_path = attestation["collector_path"]
        if (artifact_path is not None
                and _path_identity(artifact_path) != _path_identity(attested_path)):
            raise ValueError(
                "externally supplied collector artifact does not match the "
                f"build-reported attested path: supplied {os.path.abspath(artifact_path)}, "
                f"attested {attested_path}")
        artifact_path = attested_path
        size, artifact_hash = _artifact_facts(artifact_path)
        source_hash = build_input_digest()
    except ValueError as e:
        return [str(e)]

    expected = {
        "schema_version": ATTESTATION_SCHEMA_VERSION,
        "oracle_package": ORACLE_PACKAGE,
        "production_packages": list(PRODUCTION_PACKAGES),
        "registry_sha256": registry_digest(entries),
        "source_sha256": source_hash,
        "collector_sha256": artifact_hash,
        "collector_size": size,
        "collector_path": os.path.abspath(os.path.normpath(artifact_path)),
    }
    findings = []
    for field in sorted(ATTESTATION_FIELDS):
        if attestation[field] != expected[field]:
            findings.append(
                f"artifact attestation mismatch for {field}: attested "
                f"{attestation[field]!r}, current {expected[field]!r}")
    return findings


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
            lane_name, is_blocking, disposition = lane[bit]
            entry = (f"{name}: bit{bit} {lane_name} (lane-local; "
                     f"{disposition})")
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


def run_paths(paths, dump_text, oracle_error=None, artifact_findings=None,
              quiet=False):
    """Full run: compiled-registry cross-check, then every manifest. Returns
    (blocking, notes)."""
    blocking, notes = [], []
    if oracle_error is not None:
        blocking.append(
            f"COMPILER TRUTH UNAVAILABLE (fail closed): {oracle_error}")
        return blocking, notes
    for drift in registry_drift(dump_text):
        blocking.append(f"ROSTER DRIFT (audit pin unverifiable): {drift}")
    if blocking:
        # A wrong pin invalidates every downstream judgment - stop here.
        return blocking, notes
    for finding in artifact_findings or []:
        blocking.append(f"ARTIFACT ATTESTATION FAILED: {finding}")
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


def _argument_parser():
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument("--build-and-attest", action="store_true",
                      help="build production packages, then attest collector")
    mode.add_argument("--verify-artifact-only", action="store_true",
                      help="verify collector attestation without manifests")
    parser.add_argument(
        "--collector-artifact", default=None,
        help="optional expected path; must equal this build's reported cdylib")
    parser.add_argument(
        "--artifact-attestation", default=None,
        help="sidecar path (default: beside the build-reported collector)")
    parser.add_argument("manifests", nargs="*")
    return parser


def main(argv, oracle_loader=obtain_compiler_truth,
         artifact_verifier=artifact_attestation_findings,
         attestation_writer=create_artifact_attestation):
    if argv and argv[0] == "--selftest":
        return selftest()
    args = _argument_parser().parse_args(argv)

    if args.build_and_attest:
        try:
            source_before = build_input_digest()
        except ValueError as e:
            print(f"ARTIFACT ATTESTATION BLOCKED: {e}")
            return 1
        evidence, oracle_error = oracle_loader(args.collector_artifact)
        if oracle_error is not None:
            print(f"ARTIFACT ATTESTATION BLOCKED: {oracle_error}")
            return 1
        dump_text = evidence.dump_text
        artifact_path = evidence.collector_path
        attestation_path = (args.artifact_attestation
                            or _default_attestation_path(artifact_path))
        if (_path_identity(attestation_path)
                == _path_identity(evidence.registry_dump_path)):
            print("ARTIFACT ATTESTATION BLOCKED: attestation path aliases "
                  "the build-reported registry_dump executable")
            return 1
        try:
            source_after = build_input_digest()
            artifact_after_oracle = _artifact_facts(artifact_path)
        except ValueError as e:
            print(f"ARTIFACT ATTESTATION BLOCKED: {e}")
            return 1
        if source_after != source_before:
            print("ARTIFACT ATTESTATION BLOCKED: native source changed "
                  f"during build/oracle ({source_before} -> {source_after})")
            return 1
        if artifact_after_oracle != evidence.collector_facts:
            print("ARTIFACT ATTESTATION BLOCKED: collector artifact changed "
                  "during registry oracle execution")
            return 1
        findings = attestation_writer(
            dump_text, artifact_path, attestation_path,
            expected_source_hash=source_after,
            expected_artifact_facts=artifact_after_oracle)
        if findings:
            for finding in findings:
                print(f"ARTIFACT ATTESTATION BLOCKED: {finding}")
            return 1
        verification = artifact_verifier(
            dump_text, artifact_path, attestation_path)
        if verification:
            for finding in verification:
                print(f"ARTIFACT ATTESTATION BLOCKED: post-write {finding}")
            return 1
        summary = {
            "artifact_attestation": os.path.abspath(attestation_path),
            "collector_path": artifact_path,
        }
        print("C0_BUILD_ATTESTATION=" + json.dumps(
            summary, sort_keys=True, separators=(",", ":")))
        print("collector artifact attested: one production-equivalent JSON "
              "build + full registry + exact reported artifact bound")
        return 0

    evidence, oracle_error = oracle_loader(args.collector_artifact)
    dump_text = evidence.dump_text if evidence is not None else None
    artifact_findings = []
    if oracle_error is None:
        artifact_path = evidence.collector_path
        attestation_path = (args.artifact_attestation
                            or _default_attestation_path(artifact_path))
        artifact_findings = artifact_verifier(
            dump_text, artifact_path, attestation_path)
    if args.verify_artifact_only:
        if oracle_error is not None:
            print(f"ARTIFACT ATTESTATION BLOCKED: {oracle_error}")
            return 1
        if artifact_findings:
            for finding in artifact_findings:
                print(f"ARTIFACT ATTESTATION BLOCKED: {finding}")
            return 1
        print("collector artifact attestation verified against this build's "
              "reported cdylib")
        return 0

    paths = args.manifests or DEFAULT_MANIFESTS
    blocking, notes = run_paths(
        paths, dump_text, oracle_error, artifact_findings)
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

    def compiled_descriptors():
        # JSON round-trip makes nested alias arrays independent for mutations.
        return json.loads(json.dumps(load_registry_golden()))

    def dump_descriptors(descriptors):
        return "".join(json.dumps(descriptor, separators=(",", ":")) + "\n"
                       for descriptor in descriptors)

    collector_manifest = os.path.join(NATIVE_ROOT, "collector", "Cargo.toml")
    recorder_manifest = os.path.join(NATIVE_ROOT, "recorder_host", "Cargo.toml")
    collector_pkg_id = "path+file:///native/collector#collector@0.1.0"

    def build_messages(collector_path, dump_path, duplicate_collector=False,
                       duplicate_dump=False, omit_collector=False,
                       omit_dump=False, dump_manifest=collector_manifest,
                       dump_package_id=collector_pkg_id,
                       collector_package_id=collector_pkg_id):
        collector = {
            "reason": "compiler-artifact",
            "package_id": collector_package_id,
            "manifest_path": collector_manifest,
            "target": {
                "name": "collector", "kind": ["cdylib", "rlib"],
                "src_path": os.path.join(
                    NATIVE_ROOT, "collector", "src", "lib.rs"),
            },
            "filenames": [collector_path],
            "executable": None,
        }
        dump = {
            "reason": "compiler-artifact",
            "package_id": dump_package_id,
            "manifest_path": dump_manifest,
            "target": {
                "name": "registry_dump", "kind": ["bin"],
                "src_path": os.path.join(
                    NATIVE_ROOT, "collector", "src", "bin",
                    "registry_dump.rs"),
            },
            "filenames": [dump_path],
            "executable": dump_path,
        }
        messages = []
        if not omit_collector:
            messages.append(collector)
        if not omit_dump:
            messages.append(dump)
        if duplicate_collector:
            messages.append(collector)
        if duplicate_dump:
            messages.append(dump)
        return "".join(json.dumps(message) + "\n" for message in messages)

    class FakePopen:
        def __init__(self, stdout_text, returncode=0, stderr=b""):
            self.stdout = io.StringIO(stdout_text)
            self.returncode = returncode
            self.stderr_bytes = stderr

        def wait(self):
            return self.returncode

    def fake_oracle(dump_text=None, available=True, build_rc=0, run_rc=0,
                    decode_phase=None, requested_path=None,
                    duplicate_collector=False, duplicate_dump=False,
                    omit_collector=False, omit_dump=False,
                    malformed_json=False, unexpected_json=False,
                    dump_manifest=collector_manifest,
                    dump_package_id=collector_pkg_id,
                    command_sink=None):
        if dump_text is None:
            dump_text = registry_dump
        with tempfile.TemporaryDirectory() as td:
            collector_name = ("collector.dll" if os.name == "nt"
                              else "libcollector.so")
            collector_path = os.path.join(td, collector_name)
            dump_path = os.path.join(
                td, "registry_dump.exe" if os.name == "nt"
                else "registry_dump")
            with open(collector_path, "wb") as artifact:
                artifact.write(b"synthetic collector from build JSON")
            with open(dump_path, "wb") as executable:
                executable.write(b"synthetic registry executable")

            json_stream = build_messages(
                collector_path, dump_path, duplicate_collector,
                duplicate_dump, omit_collector, omit_dump,
                dump_manifest=dump_manifest,
                dump_package_id=dump_package_id)
            if malformed_json:
                json_stream += "not-json\n"
            if unexpected_json:
                json_stream += "[]\n"

            def fake_popen(command, stderr=None, **_kwargs):
                if command_sink is not None:
                    command_sink.append(command)
                if decode_phase == "build":
                    raise UnicodeDecodeError(
                        "utf-8", b"\xff", 0, 1, "synthetic invalid byte")
                if stderr is not None:
                    stderr.write(b"synthetic build failure")
                return FakePopen(json_stream, build_rc)

            def fake_run(command, **_kwargs):
                if decode_phase == "run":
                    raise UnicodeDecodeError(
                        "utf-8", b"\xff", 0, 1, "synthetic invalid byte")
                if command_sink is not None:
                    command_sink.append(command)
                return subprocess.CompletedProcess(
                    command, run_rc, dump_text, "synthetic run failure")

            return obtain_compiler_truth(
                requested_path, fake_popen, fake_run,
                lambda _name: "cargo-stub" if available else None)

    def caught_oracle_error(result, needle):
        evidence, error = result
        return ([error] if evidence is None and error is not None
                and needle in error else [])

    def caught(findings, needle):
        return [next(finding for finding in findings if needle in finding)] \
            if any(needle in finding for finding in findings) else []

    descriptors = compiled_descriptors()
    registry_dump = dump_descriptors(descriptors)
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
    d51_all_badges = run_m(
        lambda m: pd(m)["scripted_state_flags_variables_live"].update(
            status_flags_or=(1 << 11) | (1 << 12) | (1 << 13)))
    cases.append(("d51-all-badges-blocking", d51_all_badges, 3))
    for bit, name, disposition in (
        (11, "FLAG_NAMES_PARTIAL", "CapturedPartial"),
        (12, "DELAYED_EVENTS_ROOT_UNRESOLVED", "Failed/UnresolvedRoot"),
        (13, "SCRIPTED_GUI_ROOT_UNRESOLVED", "Failed/UnresolvedRoot"),
    ):
        cases.append((f"d51-bit{bit}-domain-policy",
                      [finding for finding in d51_all_badges
                       if f"bit{bit} LANE:{name}" in finding
                       and disposition in finding], 1))
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
    # Compiler-truth pin: mutations are applied to the emitted descriptors,
    # never inferred from Rust source text.
    id_mutant = [dict(descriptor) for descriptor in descriptors]
    next(d for d in id_mutant if d["wire_name"] == "tech_sharing_groups_live")[
        "id"] = 70
    cases.append(("registry-drift", caught(
        registry_drift(dump_descriptors(id_mutant)),
        "field id"), 1))
    cases.append(("registry-exact", registry_drift(registry_dump), 0))

    missing_fuel = [
        d for d in descriptors if d["wire_name"] != "fuel_live"]
    missing_findings = registry_drift(dump_descriptors(missing_fuel))
    cases.append(("registry-commented-entry",
                  caught(missing_findings, "compiled registry has 59")
                  + caught(missing_findings, "fuel_live absent"), 2))
    cases.append(("registry-extra-arg",
                  registry_drift("{not-json}\n"), 1))

    # Round 7 mutation A: an outer macro can make the release registry empty.
    # The compiled oracle observes empty stdout and blocks.
    cases.append(("registry-attr-test-outer-macro-release-empty",
                  registry_drift(""), 1))

    # Round 7 mutation B: domain() can rewrite schedulable under cfg!(test).
    constructor_mutant = [dict(descriptor) for descriptor in descriptors]
    for descriptor in constructor_mutant:
        descriptor["schedulable"] = False
    cases.append(("registry-attr-debug_assertions-constructor-cfg",
                  caught(registry_drift(
                      dump_descriptors(constructor_mutant)),
                      "semantic pin mismatch"), 1))

    # Round 8 critical: under the collector-package oracle, compact_contract is
    # a dependency. A CARGO_PRIMARY_PACKAGE-gated constructor therefore emits
    # false here and must be caught, unlike the old primary-package oracle.
    cases.append(("registry-cargo-primary-package-dependency-mutation",
                  caught(registry_drift(
                      dump_descriptors(constructor_mutant)),
                      "field schedulable"), 1))

    internal_mutant = json.loads(json.dumps(descriptors))
    next(d for d in internal_mutant if d["wire_name"] == "fuel_live")[
        "internal_name"] = "fuel_internal_mutant"
    cases.append(("registry-internal-name-mutation",
                  caught(registry_drift(dump_descriptors(internal_mutant)),
                         "field internal_name"), 1))

    status_mutant = json.loads(json.dumps(descriptors))
    next(d for d in status_mutant if d["wire_name"] == "fuel_live")[
        "status"] = "ProductionComplete"
    cases.append(("registry-exact-status-mutation",
                  caught(registry_drift(dump_descriptors(status_mutant)),
                         "field status"), 1))

    alias_mutant = json.loads(json.dumps(descriptors))
    next(d for d in alias_mutant if d["wire_name"] == "fuel_live")[
        "tags"] = ["fuel_alias_mutant"]
    cases.append(("registry-ordered-alias-mutation",
                  caught(registry_drift(dump_descriptors(alias_mutant)),
                         "field tags"), 1))

    duplicate_alias = json.loads(json.dumps(descriptors))
    next(d for d in duplicate_alias if d["wire_name"] == "fuel_live")[
        "tags"] = ["production_live"]
    cases.append(("registry-duplicate-name-alias",
                  caught(registry_drift(dump_descriptors(duplicate_alias)),
                         "scheduled name/alias 'production_live' is shared"),
                  1))

    missing_production = [
        d for d in descriptors if d["wire_name"] != "production_live"]
    missing_production_findings = registry_drift(
        dump_descriptors(missing_production))
    cases.append(("registry-nested-comment",
                  caught(missing_production_findings,
                         "compiled registry has 59")
                  + caught(missing_production_findings,
                           "production_live absent"), 2))

    shadowed = [dict(descriptor) for descriptor in descriptors]
    next(d for d in shadowed if d["wire_name"] == "fuel_live")["id"] = 999
    cases.append(("registry-shadowed-decl",
                  caught(registry_drift(dump_descriptors(shadowed)),
                         "field id"), 1))

    additive = dict(descriptors[0])
    additive.update(id=2000, wire_name="extra_domain",
                    internal_name="extra_domain")
    duplicate = [dict(descriptor) for descriptor in descriptors]
    duplicate.append(dict(descriptors[1]))
    cases.append(("registry-nonliteral-wrapper-macro",
                  registry_drift(""), 1))
    cases.append(("registry-nonliteral-additive-const",
                  caught(registry_drift(dump_descriptors(
                      descriptors + [additive])),
                      "compiled registry has 61"), 1))
    cases.append(("registry-nonliteral-additive-macro",
                  caught(registry_drift(dump_descriptors(duplicate)),
                         "duplicates id"), 1))
    cases.append(("registry-nonliteral-additive-include",
                  registry_drift(registry_dump + "include!()\n"), 1))

    # Legal Rust raw strings are irrelevant to JSONL compiler truth and cannot
    # false-positive merely because the source spelling changed.
    cases.append(("registry-raw-string-wire-name",
                  registry_drift(registry_dump), 0))

    # Acquisition failures are each distinct and fail closed without cargo.
    cases.append(("oracle-toolchain-absent",
                  caught_oracle_error(
                      fake_oracle(available=False), "absent from PATH"), 1))
    cases.append(("oracle-build-failure",
                  caught_oracle_error(
                      fake_oracle(build_rc=17), "build failed (exit 17)"), 1))
    cases.append(("oracle-run-nonzero",
                  caught_oracle_error(
                      fake_oracle(run_rc=23),
                      "registry_dump failed (exit 23)"), 1))
    cases.append(("oracle-build-non-utf8",
                  caught_oracle_error(
                      fake_oracle(decode_phase="build"), "not UTF-8"), 1))
    cases.append(("oracle-run-non-utf8",
                  caught_oracle_error(
                      fake_oracle(decode_phase="run"), "not UTF-8"), 1))
    cases.append(("oracle-empty-dump",
                  caught_oracle_error(
                      fake_oracle(dump_text=""), "stdout was empty"), 1))
    cases.append(("oracle-count-mismatch",
                  caught(missing_findings, "compiled registry has 59")
                  + caught(missing_findings, "fuel_live absent"), 2))
    cases.append(("oracle-unparseable-dump",
                  registry_drift("not-json\n"), 1))
    oracle_commands = []
    role_evidence, role_error = fake_oracle(command_sink=oracle_commands)
    expected_build_command = [
        "cargo-stub", "build", "--release", "-p", "collector",
        "-p", "recorder_host", "--message-format=json",
    ]
    command_shape_ok = (
        role_error is None and role_evidence.dump_text == registry_dump
        and len(oracle_commands) == 2
        and oracle_commands[0] == expected_build_command
        and oracle_commands[1] == [role_evidence.registry_dump_path])
    cases.append(("oracle-production-json-artifact-command",
                  [] if command_shape_ok else [str(oracle_commands)], 0))
    cases.append(("oracle-duplicate-collector-artifact-blocks",
                  caught_oracle_error(
                      fake_oracle(duplicate_collector=True),
                      "2 collector cdylib artifacts"), 1))
    cases.append(("oracle-zero-collector-artifact-blocks",
                  caught_oracle_error(
                      fake_oracle(omit_collector=True),
                      "0 collector cdylib artifacts"), 1))
    cases.append(("oracle-duplicate-dump-artifact-blocks",
                  caught_oracle_error(
                      fake_oracle(duplicate_dump=True),
                      "2 registry_dump binary artifacts"), 1))
    cases.append(("oracle-zero-dump-artifact-blocks",
                  caught_oracle_error(
                      fake_oracle(omit_dump=True),
                      "0 registry_dump binary artifacts"), 1))
    cases.append(("oracle-malformed-build-json-blocks",
                  caught_oracle_error(
                      fake_oracle(malformed_json=True), "was malformed"), 1))
    cases.append(("oracle-unexpected-build-json-shape-blocks",
                  caught_oracle_error(
                      fake_oracle(unexpected_json=True),
                      "had an unexpected shape"), 1))
    # Round-10: a registry_dump whose manifest_path is a SIBLING package
    # (recorder_host reusing collector's src_path) must not be accepted as
    # collector's dump -> it is filtered out and the count check blocks.
    cases.append(("oracle-foreign-manifest-dump-blocks",
                  caught_oracle_error(
                      fake_oracle(dump_manifest=recorder_manifest),
                      "0 registry_dump binary artifacts"), 1))
    # Round-10: a dump binding to collector's manifest but reporting a DIFFERENT
    # package_id than the attested cdylib must block on the cross-check.
    cases.append(("oracle-mismatched-package-id-dump-blocks",
                  caught_oracle_error(
                      fake_oracle(dump_package_id=(
                          "path+file:///native/recorder_host"
                          "#recorder_host@0.1.0")),
                      "different or empty package_id"), 1))
    wrong_external = os.path.join(
        tempfile.gettempdir(), "stale-default-target", "collector.dll")
    cases.append(("oracle-wrong-external-artifact-path-blocks",
                  caught_oracle_error(
                      fake_oracle(requested_path=wrong_external),
                      "not the build-reported cdylib output"), 1))

    # Round 9 CRITICAL: a recorder_host-only feature may change the unified
    # compact_contract descriptors. The collector-only dump would be golden,
    # but the one selected two-package build's emitted dump is authoritative.
    feature_unified_dump = dump_descriptors(constructor_mutant)
    feature_commands = []
    feature_evidence, feature_error = fake_oracle(
        dump_text=feature_unified_dump, command_sink=feature_commands)
    feature_findings = ([] if feature_error is not None else registry_drift(
        feature_evidence.dump_text))
    feature_caught = (
        feature_error is None
        and feature_commands[0] == expected_build_command
        and registry_drift(registry_dump) == []
        and bool(caught(feature_findings, "field schedulable")))
    cases.append(("registry-recorder-host-feature-unification-mutation",
                  [] if feature_caught else [
                      feature_error or str(feature_findings)], 0))
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
        [os.path.join(HERE, "definitely_missing.json")], registry_dump,
        quiet=True)
    cases.append(("zero-usable",
                  [b for b in zero_blocking
                   if "ZERO usable" in b or "unreadable" in b], 2))
    # Round-3 finding 4: pin main()'s OWN exit-code gate (the decisive
    # return), including the registry-unreadable arm.
    with tempfile.TemporaryDirectory() as td:
        digest_root = os.path.join(td, "digest-root")
        cache_root = os.path.join(digest_root, "redirected-cargo-output")
        os.makedirs(cache_root)
        with open(os.path.join(digest_root, "source.rs"), "w",
                  encoding="utf-8") as source:
            source.write("pub const PIN: u8 = 1;\n")
        with open(os.path.join(cache_root, "CACHEDIR.TAG"), "w",
                  encoding="ascii") as marker:
            marker.write(CARGO_CACHE_TAG_SIGNATURE + "\n")
        digest_before_output = build_input_digest(digest_root)
        with open(os.path.join(cache_root, "compiler-output.json"), "w",
                  encoding="utf-8") as output:
            output.write('{"fresh":false}\n')
        digest_after_output = build_input_digest(digest_root)
        cases.append(("source-digest-redirected-cargo-target-excluded",
                      [] if digest_before_output == digest_after_output
                      else [f"{digest_before_output} != {digest_after_output}"],
                      0))

        clean_path = os.path.join(td, "clean.json")
        with open(clean_path, "w", encoding="utf-8") as f:
            f.write(json.dumps(full_manifest()))
        blocked_path = os.path.join(td, "blocked.json")
        with open(blocked_path, "w", encoding="utf-8") as f:
            f.write(json.dumps(full_manifest(
                lambda m: pd(m).pop("fuel_live"), session="s2")))
        custom_target = os.path.join(td, "custom-target", "release")
        os.makedirs(custom_target)
        artifact_path = os.path.join(
            custom_target, "collector.dll" if os.name == "nt"
            else "libcollector.so")
        with open(artifact_path, "wb") as artifact:
            artifact.write(b"synthetic collector artifact v1")
        synthetic_dump_path = os.path.join(custom_target, "registry_dump")
        with open(synthetic_dump_path, "wb") as executable:
            executable.write(b"synthetic registry executable")
        evidence = BuildEvidence(
            registry_dump, artifact_path, _artifact_facts(artifact_path),
            synthetic_dump_path)

        def loader(requested_path=None):
            if (requested_path is not None
                    and _path_identity(requested_path)
                    != _path_identity(artifact_path)):
                return None, "synthetic wrong build-reported artifact"
            return evidence, None

        no_artifact_findings = lambda *_args: []
        cases.append(("main-clean-exit0",
                      [] if main([clean_path], loader,
                                 no_artifact_findings) == 0
                      else ["wrong exit"], 0))
        cases.append(("main-blocking-exit1",
                      [] if main([blocked_path], loader,
                                 no_artifact_findings) == 1
                      else ["wrong exit"], 0))

        attestation_path = os.path.join(td, "collector.attestation.json")
        attestation_errors = create_artifact_attestation(
            registry_dump, artifact_path, attestation_path)
        cases.append(("artifact-attestation-create", attestation_errors, 0))
        cases.append(("artifact-attestation-cannot-alias-collector", caught(
            create_artifact_attestation(
                registry_dump, artifact_path, artifact_path),
            "aliases the collector artifact"), 1))
        alias_dump_rc = main([
            "--build-and-attest", "--artifact-attestation",
            synthetic_dump_path,
        ], loader)
        cases.append(("artifact-attestation-cannot-alias-registry-dump",
                      [] if alias_dump_rc == 1 else [f"rc={alias_dump_rc}"],
                      0))
        attested = _load_artifact_attestation(attestation_path)
        cases.append(("artifact-cargo-target-dir-reported-path-bound",
                      [] if (_path_identity(attested["collector_path"])
                             == _path_identity(artifact_path))
                      else [str(attested)], 0))
        stale_default_path = os.path.join(
            td, "default-target", "release",
            "collector.dll" if os.name == "nt" else "libcollector.so")
        os.makedirs(os.path.dirname(stale_default_path))
        with open(stale_default_path, "wb") as stale:
            stale.write(b"stale default-path collector")
        cases.append(("artifact-wrong-path-substitution-blocks", caught(
            artifact_attestation_findings(
                registry_dump, stale_default_path, attestation_path),
            "does not match the build-reported attested path"), 1))
        cases.append(("artifact-attestation-source-race-blocks", caught(
            create_artifact_attestation(
                registry_dump, artifact_path, attestation_path,
                expected_source_hash="0" * 64),
            "source changed before attestation write"), 1))
        cases.append(("artifact-attestation-exact",
                      artifact_attestation_findings(
                          registry_dump, artifact_path, attestation_path), 0))
        with open(artifact_path, "ab") as artifact:
            artifact.write(b"-mutated")
        mismatch_findings = artifact_attestation_findings(
            registry_dump, artifact_path, attestation_path)
        cases.append(("artifact-digest-mismatch-blocks",
                      caught(mismatch_findings, "collector_sha256"), 1))
        verify_mismatch_rc = main([
            "--verify-artifact-only",
            "--collector-artifact", artifact_path,
            "--artifact-attestation", attestation_path,
        ], loader)
        cases.append(("artifact-digest-mismatch-exit1",
                      [] if verify_mismatch_rc == 1
                      else [f"rc={verify_mismatch_rc}"], 0))
        with open(artifact_path, "wb") as artifact:
            artifact.write(b"synthetic collector artifact v1")

        build_attest_args = [
            "--build-and-attest", "--collector-artifact", artifact_path,
            "--artifact-attestation", attestation_path,
        ]
        build_attest_rc = main(
            build_attest_args, loader, artifact_attestation_findings,
            create_artifact_attestation)
        cases.append(("main-build-and-attest-hermetic",
                      [] if build_attest_rc == 0 else [f"rc={build_attest_rc}"],
                      0))

        # Pin the actual command-line sys.exit wrapper with a stub cargo found
        # only through this child process's temporary PATH.
        dump_path = os.path.join(td, "registry_dump.jsonl")
        with open(dump_path, "w", encoding="utf-8") as f:
            f.write(registry_dump)
        if os.name == "nt":
            registry_executable = os.path.join(td, "registry_dump.bat")
            registry_stub = "@echo off\ntype \"%C0_CENSUS_TEST_DUMP%\"\n"
            cargo_path = os.path.join(td, "cargo.bat")
            cargo_stub = (
                "@echo off\n"
                "if \"%1\"==\"build\" (\n"
                "  type \"%C0_CENSUS_BUILD_JSON%\"\n"
                "  exit /b 0\n"
                ")\n"
                "exit /b 9\n")
        else:
            registry_executable = os.path.join(td, "registry_dump")
            registry_stub = (
                "#!/bin/sh\n"
                "while IFS= read -r line || [ -n \"$line\" ]; do\n"
                "  printf '%s\\n' \"$line\"\n"
                "done < \"$C0_CENSUS_TEST_DUMP\"\n")
            cargo_path = os.path.join(td, "cargo")
            cargo_stub = (
                "#!/bin/sh\n"
                "if [ \"$1\" = build ]; then\n"
                "  while IFS= read -r line || [ -n \"$line\" ]; do\n"
                "    printf '%s\\n' \"$line\"\n"
                "  done < \"$C0_CENSUS_BUILD_JSON\"\n"
                "  exit 0\n"
                "fi\n"
                "exit 9\n")
        with open(registry_executable, "w", encoding="utf-8", newline="\n") as f:
            f.write(registry_stub)
        os.chmod(registry_executable, 0o755)
        build_json_path = os.path.join(td, "cargo_build.jsonl")
        with open(build_json_path, "w", encoding="utf-8", newline="\n") as f:
            f.write(build_messages(artifact_path, registry_executable))
        with open(cargo_path, "w", encoding="utf-8", newline="\n") as f:
            f.write(cargo_stub)
        os.chmod(cargo_path, 0o755)
        child_env = os.environ.copy()
        child_env["PATH"] = td + os.pathsep + child_env.get("PATH", "")
        child_env["C0_CENSUS_TEST_DUMP"] = dump_path
        child_env["C0_CENSUS_BUILD_JSON"] = build_json_path
        artifact_args = [
            "--collector-artifact", artifact_path,
            "--artifact-attestation", attestation_path,
        ]
        clean_rc = subprocess.run(
            [sys.executable, os.path.abspath(__file__), *artifact_args,
             clean_path],
            capture_output=True, env=child_env).returncode
        blocked_rc = subprocess.run(
            [sys.executable, os.path.abspath(__file__), *artifact_args,
             blocked_path],
            capture_output=True, env=child_env).returncode
        cases.append(("process-clean-exit0",
                      [] if clean_rc == 0 else [f"rc={clean_rc}"], 0))
        cases.append(("process-blocking-exit1",
                      [] if blocked_rc == 1 else [f"rc={blocked_rc}"], 0))
        unavailable = lambda _requested=None: (
            None, "synthetic oracle unavailable")
        cases.append(("main-oracle-unavailable-exit1",
                      [] if main([clean_path], unavailable,
                                 no_artifact_findings) == 1
                      else ["wrong exit"], 0))

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

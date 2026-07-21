from __future__ import annotations

import hashlib
import json
from pathlib import Path


ROOT = Path(__file__).resolve().parents[4]
LANE = Path(__file__).resolve().parent


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


checks: list[dict[str, object]] = []


def check(name: str, condition: bool) -> None:
    checks.append({"check": name, "pass": bool(condition)})
    if not condition:
        raise AssertionError(name)


dossier = json.loads(
    (ROOT / "extractor/documents/dossiers/A24_POLITICS_TOOLTIP_INPUTS.json").read_text(
        encoding="utf-8"
    )
)
check(
    "authority executable hash exact",
    dossier["authority"]["executable_sha256"]
    == "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad",
)
check("domain id exact", dossier["carrier"]["domain_id"] == 73)
check("all six required line kinds", len(dossier["required_line_kinds"]) == 6)
check(
    "dossier records fail-closed pending activation",
    dossier["status"]
    == "STATIC_GRAPH_DETOURS_OWNER_CAPTURE_AND_REPLAY_BINDING_COMPLETE_LIVE_FIXTURE_PENDING_FAIL_CLOSED",
)

lane_source = (LANE / "src/lib.rs").read_text(encoding="utf-8")
hook_source = (LANE / "src/a729_hook.rs").read_text(encoding="utf-8")
for token in [
    "pub const DOMAIN_ID: u16 = 73;",
    "pub const MAGIC: u32 = u32::from_le_bytes(*b\"PTI1\");",
    "pub trait ExactPoliticsTooltipSource",
    "producer_manifest_sha256",
    "completeness_record_identity",
    "expected_row_count",
    "duplicate politics tooltip row key",
    "reason parent must precede child",
    "numeric terms do not equal aggregate",
]:
    check(f"lane source contains {token}", token in lane_source)

for token in [
    "pub const POLITICS_ENTRY_REFRESH_RVA: u32 = 0x01C3_3FE0;",
    "pub struct A729PoliticsHookSource;",
    "impl ExactPoliticsTooltipSource for A729PoliticsHookSource",
    "install_a729_politics_hook",
    "begin_exact_politics_frame",
    "submit_exact_politics_batch",
    "seal_exact_politics_frame",
    "required politics families incomplete",
    "PRODUCER_MANIFEST_SHA256",
    "LIVE_PRODUCER_TAPS_COMPLETE: bool = false",
    "REQUIRED_PRODUCER_SENTINELS",
    "uninstall_a729_politics_hook",
    "arm_a24_capture_epoch",
    "MODIFIER_VALUE_LOOKUP_RVA",
    "TOOLTIP_FORMATTER_CALLSITE_A_RVA",
    "TOOLTIP_DISPATCH_CHAIN_TAIL",
]:
    check(f"native hook source contains {token}", token in hook_source)
check("native hook module exported", "pub mod a729_hook;" in lane_source)
check(
    "native hook requires all six families",
    hook_source.count("PoliticsLineKind::") >= 6,
)
check(
    "dossier records incomplete disabled native source",
    dossier["live_producer_install"]["state"] == "IMPLEMENTED_FIXTURE_PENDING_DISABLED_FAIL_CLOSED",
)

workspace = (ROOT / "extractor/native/Cargo.toml").read_text(encoding="utf-8")
collector_manifest = (ROOT / "extractor/native/collector/Cargo.toml").read_text(
    encoding="utf-8"
)
collector_source = (ROOT / "extractor/native/collector/src/lib.rs").read_text(
    encoding="utf-8"
)
registry = json.loads(
    (ROOT / "extractor/native/domain_registry.json").read_text(encoding="utf-8")
)
compact = (ROOT / "extractor/native/compact_contract/src/lib.rs").read_text(
    encoding="utf-8"
)
world_fold = (ROOT / "extractor/native/recorder_host/src/world_fold.rs").read_text(
    encoding="utf-8"
)
integration = (ROOT / "extractor/native/recorder_host/tests/a24_d73.rs").read_text(
    encoding="utf-8"
)
replay_world = (
    ROOT / "replay_interface/native/replay_core/src/world.rs"
).read_text(encoding="utf-8")
replay_exports = (
    ROOT / "replay_interface/native/replay_core/src/lib.rs"
).read_text(encoding="utf-8")
game_ui = (
    ROOT / "replay_interface/native/game_ui/src/politics_tooltips.rs"
).read_text(encoding="utf-8")
game_ui_exports = (
    ROOT / "replay_interface/native/game_ui/src/lib.rs"
).read_text(encoding="utf-8")
capture = json.loads(
    (LANE / "H_A24_CAPTURE_AUTOMATION_20260720.json").read_text(encoding="utf-8")
)
report = (LANE / "A24_RE_REPORT.md").read_text(encoding="utf-8")

check("workspace member installed", "lanes/politics_tooltip_inputs_live" in workspace)
check("collector dependency installed", "politics_tooltip_inputs_live_lane" in collector_manifest)
check("collector exact seam installed", "encode_a24_exact_politics_tooltip_frame" in collector_source)
domain73 = next(row for row in registry["domains"] if row["id"] == 73)
check("registry d73 exact name", domain73["name"] == "politics_tooltip_inputs_live")
check("registry d73 unscheduled", domain73["schedulable"] is False)
check("registry d73 requires data record", domain73["requires_data_record"] is True)
check(
    "compact descriptor installed",
    'domain(\n        73,\n        "politics_tooltip_inputs_live"' in compact,
)
check("production fold decoder installed", "fn fold_politics_tooltip_inputs" in world_fold)
check("production fold preserves status flags", '"status_flags": decoded.status_flags' in world_fold)
check("production fold rejects cross-hour attachment", "frame_matches_snapshot" in world_fold)
check("production snapshot leaf installed", "_r2_politics_tooltip_inputs_live" in world_fold)
check(
    "integration covers all six families",
    "all_six_a24_families_fold_with_exact_provenance" in integration,
)
check(
    "integration covers cross-hour rejection",
    "pti1_frame_cannot_attach_to_a_different_snapshot_hour" in integration,
)
check("typed replay leaf installed", "pub tooltip_inputs: Leaf<Vec<PoliticsTooltipInputExact>>" in replay_world)
check("strict PTI1 replay builder installed", "fn build_politics_tooltip_inputs(" in replay_world)
check("unknown country join poisons frame", "producer row names a country absent" in replay_world)
check("A24 replay types exported", "PoliticsTooltipInputExact" in replay_exports)
check("fail-closed game UI selector installed", "PoliticsTooltipSelection" in game_ui)
check("game UI forbids trigger inference", "never evaluates Clausewitz triggers" in game_ui)
check("game UI A24 module exported", "pub mod politics_tooltips;" in game_ui_exports)
check(
    "capture automation pending and fail closed",
    capture["status"] == "OWNER_CAPTURE_IMPLEMENTED_LIVE_FIXTURE_PENDING_FAIL_CLOSED",
)
check("capture matrix covers required taps", len(capture["required_taps"]) == 9)
check(
    "capture automation requires no current human input",
    capture["human_input"]["required_now"] is False,
)

manifest_hash = "18bac19dc5de7fc0ef90a9341e0bc5ed1ac73c745a1dc96f6177ed08c8f70b35"
for address in dossier["producer_graph"]["function_addresses"]:
    check(f"report binds function {address}", address in report.lower())
check("report binds producer manifest hash", manifest_hash in report)
check(
    "report binds string artifact hash",
    "4096785ffe2701327a1702ddc5e4640255f8f0505c050cf684376ddbbc07a3c6"
    in report,
)

gui = Path(dossier["game_file_evidence"]["gui_path"])
loc = Path(dossier["game_file_evidence"]["localisation_path"])
check(
    "installed politics GUI hash exact",
    sha256(gui) == dossier["game_file_evidence"]["gui_sha256"],
)
check(
    "installed politics localisation hash exact",
    sha256(loc) == dossier["game_file_evidence"]["localisation_sha256"],
)
loc_text = loc.read_text(encoding="utf-8-sig")
for token in dossier["game_file_evidence"]["pinned_tokens"]:
    check(f"localisation contains {token}", token in loc_text)

xui11 = json.loads(
    (
        ROOT
        / "replay_interface/re_evidence/rendering_laws/XUI11/verification.json"
    ).read_text(encoding="utf-8")
)
check(
    "retained RDC census exact",
    xui11["retained_capture_census"]["raw_rdc_count"] == 214,
)
check(
    "retained politics-named capture list empty",
    xui11["retained_capture_census"]["politics_named_capture_hits"] == [],
)

artifact_manifest = json.loads((LANE / "manifest.json").read_text(encoding="utf-8"))
for artifact in artifact_manifest["artifacts"]:
    artifact_path = ROOT / artifact["path"]
    check(
        f"artifact hash exact: {artifact['path']}",
        sha256(artifact_path) == artifact["sha256"],
    )

result = {
    "schema": "hoi4-extractor-a24-verification-v2",
    "status": "PASS_STATIC_GRAPH_REPLAY_BINDING_LIVE_TAPS_PENDING_FAIL_CLOSED",
    "checks_passed": sum(1 for row in checks if row["pass"]),
    "checks_total": len(checks),
    "checks": checks,
}
print(json.dumps(result, indent=2, sort_keys=True))

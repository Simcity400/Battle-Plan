"""Lane-local guard for future battleplan geometry/execution integration.

This does not integrate anything. It rejects proposed integration contracts that
would turn the current blockers into bad production data: passive scan rows,
save-only/cache-only fields, or row-width changes to existing proved domains.
"""

from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any

from battleplan_fixture_gate import validate_fixture


EXPECTED_SCHEMA = "army_oob_orders_battleplans_integration_guard_v1"
PROTECTED_DOMAINS = {
    "army_orders": 27,
    "oob_command": 28,
}
ALLOWED_NEW_DOMAINS = {
    "army_battleplans_depth_live",
}
ALLOWED_NEW_DOMAIN_IDS = {
    "army_battleplans_depth_live": 54,
}
FORBIDDEN_SOURCE_KINDS = {
    "passive_scan",
    "save_cached_plan_value",
    "save_cached_our_power",
    "save_cached_enemy_power",
    "save_instance_id",
    "scheduled_member_cache",
}


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("proposal_json", type=Path)
    ap.add_argument(
        "--allow-rejection",
        action="store_true",
        help="allow fixture rows that exactly reject a candidate",
    )
    args = ap.parse_args()

    proposal = json.loads(args.proposal_json.read_text(encoding="utf-8"))
    errors = validate_proposal(proposal, args.proposal_json.parent, allow_rejection=args.allow_rejection)
    print(json.dumps({"status": "PASS" if not errors else "FAIL", "errors": errors}, indent=2, sort_keys=True))
    return 0 if not errors else 1


def validate_proposal(proposal: dict[str, Any], base_dir: Path, *, allow_rejection: bool) -> list[str]:
    errors = validate_proposal_shape(proposal)

    fixture_path = resolve_path(proposal.get("fixture_json"), base_dir)
    if fixture_path is None:
        errors.append("fixture_json is required")
    elif not fixture_path.exists():
        errors.append(f"fixture_json does not exist: {fixture_path}")
    else:
        fixture = json.loads(fixture_path.read_text(encoding="utf-8"))
        fixture_errors = validate_fixture(fixture, allow_rejection=allow_rejection)
        errors.extend(f"fixture: {err}" for err in fixture_errors)
    return errors


def validate_proposal_with_fixture(
    proposal: dict[str, Any],
    fixture: dict[str, Any],
    *,
    allow_rejection: bool,
) -> list[str]:
    errors = validate_proposal_shape(proposal)
    errors.extend(f"fixture: {err}" for err in validate_fixture(fixture, allow_rejection=allow_rejection))
    return errors


def validate_proposal_shape(proposal: dict[str, Any]) -> list[str]:
    errors: list[str] = []
    if proposal.get("schema") != EXPECTED_SCHEMA:
        errors.append(f"schema must be {EXPECTED_SCHEMA}")
    if proposal.get("shared_entrypoint_slot_required") is not True:
        errors.append("shared_entrypoint_slot_required must be true")
    if proposal.get("touches_shared_entrypoints_now") is not False:
        errors.append("touches_shared_entrypoints_now must be false while another lane owns the slot")

    domain_changes = proposal.get("domain_changes", [])
    if not isinstance(domain_changes, list) or not domain_changes:
        errors.append("domain_changes must be a non-empty list")
        domain_changes = []
    for index, change in enumerate(domain_changes):
        validate_domain_change(errors, change, f"domain_changes[{index}]")

    for index, source in enumerate(proposal.get("candidate_sources", [])):
        validate_candidate_source(errors, source, f"candidate_sources[{index}]")
    return errors


def validate_domain_change(errors: list[str], change: Any, name: str) -> None:
    if not isinstance(change, dict):
        errors.append(f"{name} must be an object")
        return
    domain_name = change.get("domain_name")
    domain_id = change.get("domain_id")
    if domain_name in PROTECTED_DOMAINS:
        if domain_id != PROTECTED_DOMAINS[domain_name]:
            errors.append(f"{name} changes protected domain id for {domain_name}")
        if change.get("row_width_change") is not False:
            errors.append(f"{name} must not change row width for protected domain {domain_name}")
        if change.get("version_bump") is True:
            errors.append(f"{name} must not version-bump protected domain {domain_name} for this blocker")
        if change.get("mode") != "unchanged":
            errors.append(f"{name}.mode must be unchanged for protected domain {domain_name}")
        return
    if domain_name not in ALLOWED_NEW_DOMAINS:
        errors.append(f"{name}.domain_name must be protected unchanged or army_battleplans_depth_live")
    if change.get("mode") != "additive":
        errors.append(f"{name}.mode must be additive for new battleplan domains")
    if ALLOWED_NEW_DOMAIN_IDS.get(domain_name) != domain_id:
        errors.append(f"{name}.domain_id must be 54 for army_battleplans_depth_live")
    if change.get("row_width_change") is not False:
        errors.append(f"{name} must not change existing row widths; use a new additive payload")


def validate_candidate_source(errors: list[str], source: Any, name: str) -> None:
    if not isinstance(source, dict):
        errors.append(f"{name} must be an object")
        return
    source_kind = source.get("source_kind")
    if source_kind in FORBIDDEN_SOURCE_KINDS:
        errors.append(f"{name}.source_kind {source_kind} is not accepted as integration evidence")
    if source.get("status") == "candidate":
        errors.append(f"{name}.status candidate is not accepted; provide exact or rejected fixture evidence")
    if source.get("requires_fixture") is not True:
        errors.append(f"{name}.requires_fixture must be true for geometry/execution fields")
    if source.get("owner_lane") != "army_oob_orders_battleplans_live":
        errors.append(f"{name}.owner_lane must be army_oob_orders_battleplans_live")


def resolve_path(value: Any, base_dir: Path) -> Path | None:
    if not isinstance(value, str) or not value:
        return None
    path = Path(value)
    if path.is_absolute():
        return path
    return (base_dir / path).resolve()


if __name__ == "__main__":
    raise SystemExit(main())

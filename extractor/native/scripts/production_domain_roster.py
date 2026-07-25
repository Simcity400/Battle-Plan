#!/usr/bin/env python3
"""Print the production-profile compact domain roster as WIRE names.

`--scheduled-domain` takes wire names, and `domain_registry.json` carries the
descriptor `name`, which is not always the wire name (`divisions_live` vs
`divisions`). This joins the two authorities by domain id:

  * `domain_registry.json`  -> which ids are schedulable and production-status
  * `compact_contract/src/lib.rs` `domain(...)` table -> id to wire name

Every schedulable id must resolve to a wire name or this fails closed, so a
drifted table is a loud error rather than a silently short roster.

Usage:
    python production_domain_roster.py [--native-root <dir>] [--json]
"""
from __future__ import annotations

import argparse
import json
import os
import re
import sys

# domain(<id>, "<name>", "<wire_name>", CompactDomainStatus::<Status>, ...)
DOMAIN_ENTRY = re.compile(
    r"\bdomain\(\s*"
    r"(?P<id>\d+)\s*,\s*"
    r'"(?P<name>[A-Za-z0-9_]+)"\s*,\s*'
    r'"(?P<wire>[A-Za-z0-9_]+)"\s*,\s*'
    r"CompactDomainStatus::(?P<status>[A-Za-z]+)\s*,",
    re.MULTILINE,
)

PRODUCTION_STATUSES = {
    "existing",
    "production_complete",
    "production_complete_base",
    "validated_slice",
    "validated_partial_slice",
    "validated_slice_open_gaps",
}


def wire_names_by_id(contract_source: str) -> dict[int, str]:
    with open(contract_source, encoding="utf-8") as handle:
        text = handle.read()
    table: dict[int, str] = {}
    for match in DOMAIN_ENTRY.finditer(text):
        table[int(match.group("id"))] = match.group("wire")
    if not table:
        raise SystemExit(f"no domain(...) descriptors parsed from {contract_source}")
    return table


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--native-root",
        default=os.path.normpath(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..")),
    )
    parser.add_argument("--json", action="store_true")
    args = parser.parse_args()

    registry_path = os.path.join(args.native_root, "domain_registry.json")
    contract_path = os.path.join(args.native_root, "compact_contract", "src", "lib.rs")
    for path in (registry_path, contract_path):
        if not os.path.isfile(path):
            raise SystemExit(f"missing authority file: {path}")

    with open(registry_path, encoding="utf-8") as handle:
        registry = json.load(handle)
    wire = wire_names_by_id(contract_path)

    roster = []
    unresolved = []
    for entry in registry["domains"]:
        if not entry.get("schedulable"):
            continue
        if entry.get("status") not in PRODUCTION_STATUSES:
            continue
        domain_id = int(entry["id"])
        if domain_id not in wire:
            unresolved.append((domain_id, entry.get("name")))
            continue
        roster.append({"id": domain_id, "name": entry["name"], "wire_name": wire[domain_id]})

    if unresolved:
        for domain_id, name in unresolved:
            print(f"unresolved wire name for domain {domain_id} ({name})", file=sys.stderr)
        raise SystemExit("domain registry and compact_contract descriptor table disagree")

    roster.sort(key=lambda row: row["id"])
    if args.json:
        json.dump(roster, sys.stdout, indent=2)
        print()
    else:
        for row in roster:
            print(row["wire_name"])
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

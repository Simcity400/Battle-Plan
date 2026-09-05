"""Re-verify every embedded artifact in the engine_evidence crate AGAINST THE BINARY.

WHY THIS EXISTS
---------------
The crate's unit tests pin each artifact to a recorded `(address, bytes, sha256)`
tuple. Codex review round 10 (lane WR, findings 13 and 14) showed what that
cannot do: an artifact's tie to a real function is a `// function=FUN_<addr>`
COMMENT INSIDE THE ARTIFACT, and the hash it is checked against lives in the same
repository. Keep the header, swap the body, restamp the tuple, restage the blob,
and every in-tree test passes on another function's body.

No test that reads only this tree can close that, because the tree does not
contain hoi4.exe. Only re-deriving the artifact from the binary can. That is what
this script does: for every embedded artifact it asks the Ghidra broker's
read-only index what the canonical export for that address actually is, and
compares the broker's sha256 to the bytes the crate carries.

It is intended to run at LANDING for any change to the evidence crate. It is
read-only: it opens the index `mode=ro`, launches no analysis, and writes only
its own report.

USAGE
-----
    python extractor/native/scripts/reverify_engine_evidence.py [--out PATH]

Exit codes:
    0  every embedded artifact matches the broker
    1  at least one artifact does NOT match - a real finding, report it
    2  the broker or the index is unavailable, so nothing was verified

A MISMATCH IS NOT TO BE SILENTLY FIXED. It means the tree's bytes and the
binary's bytes disagree, which is exactly the condition the in-tree pins cannot
see.
"""

from __future__ import annotations

import argparse
import hashlib
import io
import json
import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[3]
CRATE = REPO / "portable_replay" / "native" / "engine_evidence"
ARTIFACTS = CRATE / "artifacts"
SRC = CRATE / "src"

BROKER_TOOLS = REPO / "replay_interface" / "tools" / "ghidra"
STORE_ROOT = Path(r"C:\hoi4_evidence\ghidra_agent_exports")
INDEX_PATH = Path(r"C:\hoi4_evidence\agent_re\function_index.sqlite3")

# The build this crate's evidence is pinned to.
BUILD_ID = "hoi4.exe 1.19.2.0.a729"
BUILD_SHA256 = "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"

# `("1413f4fd0", 1718, "ff03...")` as the crate's pin tables spell it.
TUPLE = re.compile(
    r'\(\s*"([0-9a-f]{6,})"\s*,\s*(\d+)\s*,\s*"([0-9a-f]{64})"\s*,?\s*\)',
    re.MULTILINE,
)

# `("func_14234ccf0.instructions.txt", 1437, "bd3e...")` - the sidecar tables.
SIDECAR_TUPLE = re.compile(
    r'\(\s*"(func_[0-9a-zA-Z_.]+)"\s*,\s*(\d+)\s*,\s*"([0-9a-f]{64})"\s*,?\s*\)',
    re.MULTILINE,
)

KIND_BY_SUFFIX = {
    ".decompile.c": "decompile",
    ".instructions.txt": "instructions",
    ".pcode.txt": "pcode",
    ".graph.txt": "graph",
}


def recorded_tuples() -> dict[tuple[str, str], tuple[int, str]]:
    """Every hash the crate records for an artifact, keyed by (address, sha256).

    The crate pins in TWO places and both count: most modules carry a Rust
    `(address, bytes, sha256)` table, while the VC set is pinned by filename in
    artifacts/EVIDENCE_MANIFEST.json. A script that read only the first would
    report 34 artifacts as unpinned when they are pinned, which is a false
    alarm - and a check that cries wolf stops being read."""
    found: dict[tuple[str, str], tuple[int, str]] = {}
    for path in sorted(SRC.glob("*.rs")):
        text = io.open(path, encoding="utf-8", newline="").read()
        for address, size, digest in TUPLE.findall(text):
            found[(address, digest)] = (int(size), path.name)

    # Sidecars are pinned by FILENAME rather than by address, in their own
    # `(name, bytes, sha256)` tables.
    for path in sorted(SRC.glob("*.rs")):
        text = io.open(path, encoding="utf-8", newline="").read()
        for name, size, digest in SIDECAR_TUPLE.findall(text):
            for suffix in KIND_BY_SUFFIX:
                if name.startswith("func_") and name.endswith(suffix):
                    address = name[len("func_") : -len(suffix)]
                    found.setdefault((address, digest), (int(size), path.name))
                    break

    manifest_path = ARTIFACTS / "EVIDENCE_MANIFEST.json"
    if manifest_path.is_file():
        manifest = json.loads(io.open(manifest_path, encoding="utf-8").read())
        for name, entry in (manifest.get("files") or {}).items():
            if not (name.startswith("func_") and isinstance(entry, dict)):
                continue
            digest = entry.get("sha256")
            for suffix in KIND_BY_SUFFIX:
                if name.endswith(suffix) and digest:
                    address = name[len("func_") : -len(suffix)]
                    found.setdefault(
                        (address, digest), (entry.get("bytes", 0), manifest_path.name)
                    )
                    break
    return found


def embedded_artifacts() -> list[dict]:
    """Every artifact file in the crate, with its address and kind where the
    name carries them."""
    rows = []
    for path in sorted(ARTIFACTS.iterdir()):
        if not path.is_file() or path.name.startswith("."):
            continue
        raw = path.read_bytes()
        row = {
            "artifact": path.name,
            "bytes": len(raw),
            "sha256": hashlib.sha256(raw).hexdigest(),
            "address": None,
            "kind": None,
        }
        for suffix, kind in KIND_BY_SUFFIX.items():
            if path.name.startswith("func_") and path.name.endswith(suffix):
                row["address"] = path.name[len("func_") : -len(suffix)]
                row["kind"] = kind
                break
        rows.append(row)
    return rows


def broker_verdicts(requests: list[tuple[str, str]]) -> dict[tuple[str, str], dict]:
    """Ask the broker's read-only index for the canonical export of each
    (address, kind). Uses FunctionIndex directly so this script needs no MCP
    session and can run unattended at landing."""
    sys.path.insert(0, str(BROKER_TOOLS))
    from function_index import FunctionIndex  # type: ignore

    index = FunctionIndex(store_root=STORE_ROOT, index_path=INDEX_PATH)
    by_kind: dict[str, list[str]] = {}
    for address, kind in requests:
        by_kind.setdefault(kind, []).append(address)

    verdicts: dict[tuple[str, str], dict] = {}
    for kind, addresses in by_kind.items():
        # Chunked so one oversized reply cannot lose the whole run.
        for start in range(0, len(addresses), 40):
            chunk = addresses[start : start + 40]
            reply = index.verify({"addresses": chunk, "kinds": [kind]})
            for result in reply.get("results", []):
                address = str(result.get("address", "")).removeprefix("0x")
                verdicts[(address, result.get("kind", kind))] = result
    return verdicts


def matches_lf_export(
    committed: bytes,
    exported: bytes,
    raw_pin: tuple[int, str] | None,
    committed_pin: tuple[int, str] | None,
) -> bool:
    if raw_pin is None or committed_pin is None:
        return False
    return (
        len(exported) == raw_pin[0]
        and hashlib.sha256(exported).hexdigest() == raw_pin[1]
        and len(committed) == committed_pin[0]
        and hashlib.sha256(committed).hexdigest() == committed_pin[1]
        and b"\r" not in committed
        and committed == exported.replace(b"\r\n", b"\n")
    )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--out",
        default=str(REPO / "extractor" / "native" / "runs" / "wr_re_residue" / "RE_VERIFICATION.json"),
    )
    args = parser.parse_args()

    artifacts = embedded_artifacts()
    pins = recorded_tuples()

    verifiable = [
        (row["address"], row["kind"]) for row in artifacts if row["address"] and row["kind"]
    ]
    try:
        verdicts = broker_verdicts(verifiable)
    except Exception as error:  # noqa: BLE001 - the report has to say why
        report = {
            "status": "BROKER_UNAVAILABLE",
            "error": repr(error),
            "verified": 0,
            "note": "nothing was verified; this is not a pass",
        }
        io.open(args.out, "w", encoding="utf-8", newline="\n").write(
            json.dumps(report, indent=1) + "\n"
        )
        print("BROKER UNAVAILABLE:", error)
        return 2

    rows = []
    mismatches = []
    unverifiable = []
    for row in artifacts:
        address, kind = row["address"], row["kind"]
        if not address:
            unverifiable.append(
                {
                    "artifact": row["artifact"],
                    "sha256": row["sha256"],
                    "why": "not a function export - no address in its name, so the broker has "
                    "nothing to compare it to by address",
                }
            )
            continue
        verdict = verdicts.get((address, kind))
        pin = pins.get((address, row["sha256"]))
        entry = {
            "address": address,
            "kind": kind,
            "artifact": row["artifact"],
            "crate_bytes": row["bytes"],
            "crate_sha256": row["sha256"],
            "pinned_in_source": pin[1] if pin else None,
            "pin_matches_file": pin is not None,
            "broker_sha256": verdict.get("actual_sha256") if verdict else None,
            "broker_bytes": verdict.get("actual_bytes") if verdict else None,
            "broker_indexed_sha256": verdict.get("indexed_sha256") if verdict else None,
            "broker_agrees_with_its_own_index": verdict.get("agrees") if verdict else None,
            "canonical_path": verdict.get("canonical_path") if verdict else None,
        }
        if verdict is None:
            entry["verdict"] = "NOT_IN_BROKER"
            mismatches.append(entry)
        elif not verdict.get("agrees"):
            entry["verdict"] = "BROKER_DISAGREES_WITH_ITS_OWN_INDEX"
            mismatches.append(entry)
        elif verdict.get("actual_sha256") != row["sha256"]:
            exported = Path(verdict["canonical_path"]).read_bytes()
            committed = (ARTIFACTS / row["artifact"]).read_bytes()
            raw_pin = pins.get((address, verdict["actual_sha256"]))
            if (
                len(exported) == verdict["actual_bytes"]
                and matches_lf_export(
                    committed, exported,
                    (raw_pin[0], verdict["actual_sha256"]) if raw_pin else None,
                    (pin[0], row["sha256"]) if pin else None,
                )
            ):
                entry["comparison"] = "lf_normalized"
                entry["raw_export_pinned_in_source"] = raw_pin[1]
                entry["verdict"] = "MATCHES"
            else:
                entry["verdict"] = "CRATE_BYTES_DIFFER_FROM_BINARY_EXPORT"
                mismatches.append(entry)
        elif not pin:
            entry["verdict"] = "MATCHES_BINARY_BUT_NO_SOURCE_PIN"
            mismatches.append(entry)
        else:
            entry["verdict"] = "MATCHES"
        rows.append(entry)

    report = {
        "what_this_is": "every embedded artifact in the engine_evidence crate, re-derived "
        "from the Ghidra broker's canonical export for its address and compared byte for "
        "byte. This is the only check in the project that ties the crate's bytes to "
        "hoi4.exe rather than to another file in the same repository.",
        "closes": "Codex review round 10, lane WR, findings 13 and 14: the in-tree pins "
        "cannot detect a body swapped under a retained header with the tuple restamped.",
        "build": {"binary_id": BUILD_ID, "executable_sha256": BUILD_SHA256},
        "index_path": str(INDEX_PATH),
        "store_root": str(STORE_ROOT),
        "status": "PASS" if not mismatches else "FAIL",
        "counts": {
            "artifacts_in_crate": len(artifacts),
            "verified_against_the_binary": len(rows),
            "matches": sum(1 for row in rows if row["verdict"] == "MATCHES"),
            "mismatches": len(mismatches),
            "not_function_exports": len(unverifiable),
        },
        "mismatches": mismatches,
        "not_function_exports": unverifiable,
        "artifacts": rows,
    }
    io.open(args.out, "w", encoding="utf-8", newline="\n").write(
        json.dumps(report, indent=1) + "\n"
    )
    print(
        "%s: %d/%d artifacts match the binary's export, %d mismatches, %d not function exports"
        % (
            report["status"],
            report["counts"]["matches"],
            report["counts"]["verified_against_the_binary"],
            report["counts"]["mismatches"],
            report["counts"]["not_function_exports"],
        )
    )
    for row in mismatches:
        print("  MISMATCH", row["address"], row["kind"], row["verdict"])
    return 0 if not mismatches else 1


if __name__ == "__main__":
    raise SystemExit(main())

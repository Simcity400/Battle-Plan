from __future__ import annotations

import hashlib
import json
import re
from pathlib import Path
from typing import Any

from common import sha256_file
from registry import Registry


INSTRUCTION_RE = re.compile(
    r"^([0-9a-fA-F]+)\s+([0-9a-fA-F]+)\s+([A-Z][A-Z0-9]*)\s*(.*)$"
)
REGISTER_RE = re.compile(
    r"^(?:R(?:AX|BX|CX|DX|SI|DI|SP|BP|8|9|1[0-5])|E(?:AX|BX|CX|DX|SI|DI|SP|BP)|"
    r"(?:AX|BX|CX|DX|SI|DI|SP|BP)|[A-D][HL]|XMM\d+|YMM\d+|ZMM\d+)$",
    re.IGNORECASE,
)


def operand_shape(operand: str) -> str:
    value = operand.strip()
    if not value:
        return ""
    if "[" in value or "PTR" in value.upper():
        return "MEM"
    if REGISTER_RE.fullmatch(value):
        return "REG"
    if re.search(r"(?:0x[0-9a-f]+|\b\d+\b)", value, re.IGNORECASE):
        return "IMM"
    return "OTHER"


def parse_instruction_artifact(
    path: Path, build_id: str, image_base: int, job_id: str,
    artifact_hash: str | None = None,
) -> dict[str, Any]:
    headers: dict[str, str] = {}
    byte_chunks: list[bytes] = []
    shapes: list[str] = []
    first_address: int | None = None
    for raw in path.read_text(encoding="utf-8", errors="replace").splitlines():
        line = raw.strip()
        if not line:
            continue
        if "=" in line and line[0].isalnum():
            key, value = line.split("=", 1)
            if key in {"function", "body_min", "body_max", "image_base", "binary_id", "executable_sha256"}:
                headers[key] = value
                continue
        match = INSTRUCTION_RE.match(line)
        if match is None:
            continue
        address = int(match.group(1), 16)
        if first_address is None:
            first_address = address
        byte_chunks.append(bytes.fromhex(match.group(2)))
        mnemonic = match.group(3).upper()
        operands = [operand_shape(value) for value in match.group(4).split(",")]
        operands = [value for value in operands if value]
        shapes.append(mnemonic + (":" + ",".join(operands) if operands else ""))
    if not shapes or first_address is None:
        raise ValueError(f"no instructions found in {path}")
    # Decorated C++ names can themselves contain '@'; the exporter appends the
    # entry address after the final delimiter.
    function = headers.get("function", path.stem).rsplit("@", 1)
    name = function[0]
    entry = int(function[1], 16) if len(function) == 2 else first_address
    body_min = int(headers.get("body_min", f"{first_address:x}"), 16)
    body_max = int(headers.get("body_max", f"{first_address:x}"), 16)
    shape_text = " ".join(shapes)
    return {
        "build_id": build_id,
        "rva": entry - image_base,
        "name": name,
        "body_min": body_min - image_base,
        "body_max": body_max - image_base,
        "instruction_count": len(shapes),
        "call_count": sum(1 for value in shapes if value.startswith("CALL")),
        "byte_sha256": hashlib.sha256(b"".join(byte_chunks)).hexdigest(),
        "shape_sha256": hashlib.sha256(shape_text.encode("utf-8")).hexdigest(),
        "shape_text": shape_text,
        "source_job_id": job_id,
        "source_artifact": str(path),
        "source_artifact_sha256": artifact_hash or sha256_file(path),
    }


def ingest_ghidra_jobs(registry: Registry, output_root: Path) -> dict[str, Any]:
    if not output_root.is_dir():
        raise FileNotFoundError(f"Ghidra output root not found: {output_root}")
    jobs_seen = jobs_ingested = manifests_seen = manifests_skipped = functions_ingested = invalid = 0
    invalid_details: list[dict[str, str]] = []
    builds: set[str] = set()
    for directory in output_root.iterdir():
        if not directory.is_dir() or directory.name.startswith("."):
            continue
        job_path = directory / "job.json"
        if not job_path.is_file():
            continue
        jobs_seen += 1
        try:
            job = json.loads(job_path.read_text(encoding="utf-8"))
            if job.get("status") != "complete":
                continue
            manifest_paths = list(directory.rglob("_FUNCTION_EVIDENCE_MANIFEST.json"))
            if not manifest_paths:
                continue
            for manifest_path in manifest_paths:
                manifests_seen += 1
                manifest_key = str(manifest_path.resolve())
                manifest_digest = sha256_file(manifest_path)
                receipt = registry.current_manifest(manifest_key, manifest_digest)
                if receipt is not None:
                    manifests_skipped += 1
                    builds.add(str(receipt["build_id"]))
                    continue
                manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
                sha = str(manifest["executable_sha256"]).lower()
                build_id = registry.upsert_build(
                    str(manifest["binary_id"]), sha, str(manifest["image_base"]),
                )
                builds.add(build_id)
                image_base = int(str(manifest["image_base"]), 16)
                hashes = {row["path"]: row["sha256"] for row in manifest.get("artifacts", [])}
                rows = []
                for artifact in manifest.get("artifacts", []):
                    relative = artifact.get("path", "")
                    artifact_path = manifest_path.parent / relative
                    if not relative.endswith(".instructions.txt") or not artifact_path.is_file():
                        continue
                    if artifact_path.stat().st_size != artifact["bytes"] or sha256_file(artifact_path) != artifact["sha256"]:
                        raise ValueError(f"artifact hash mismatch: {artifact_path}")
                    rows.append(parse_instruction_artifact(
                        artifact_path, build_id, image_base, directory.name, hashes.get(relative)
                    ))
                functions_ingested += registry.upsert_functions(rows)
                registry.mark_inventory(build_id, "partial_ghidra_exports")
                registry.record_manifest(
                    manifest_key, manifest_digest, build_id, directory.name, len(rows),
                )
            jobs_ingested += 1
        except Exception as error:
            invalid += 1
            invalid_details.append({
                "job_id": directory.name,
                "error": f"{type(error).__name__}: {error}",
            })
    return {
        "schema": "hoi4-re-ghidra-ingest-v1",
        "output_root": str(output_root),
        "jobs_seen": jobs_seen,
        "jobs_ingested": jobs_ingested,
        "manifests_seen": manifests_seen,
        "manifests_skipped": manifests_skipped,
        "functions_ingested": functions_ingested,
        "invalid_jobs": invalid,
        "invalid_details": invalid_details,
        "build_ids": sorted(builds),
    }


def seed_manifest_receipts(
    registry: Registry, output_root: Path, verified_result: dict[str, Any], source_job_id: str,
) -> dict[str, Any]:
    if verified_result.get("schema") != "hoi4-re-ghidra-ingest-v1":
        raise ValueError("source job is not a Ghidra ingest result")
    if int(verified_result.get("invalid_jobs", -1)) != 0:
        raise ValueError("receipt seeding requires a completed ingest with zero invalid jobs")
    if Path(str(verified_result.get("output_root", ""))).resolve() != output_root.resolve():
        raise ValueError("source ingest used a different Ghidra output root")
    receipts = skipped = jobs_seen = 0
    builds: set[str] = set()
    for directory in output_root.iterdir():
        job_path = directory / "job.json"
        if not directory.is_dir() or not job_path.is_file():
            continue
        job = json.loads(job_path.read_text(encoding="utf-8"))
        if job.get("status") != "complete":
            continue
        jobs_seen += 1
        for manifest_path in directory.rglob("_FUNCTION_EVIDENCE_MANIFEST.json"):
            manifest_key = str(manifest_path.resolve())
            manifest_digest = sha256_file(manifest_path)
            if registry.current_manifest(manifest_key, manifest_digest) is not None:
                skipped += 1
                continue
            manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
            build_id = registry.upsert_build(
                str(manifest["binary_id"]), str(manifest["executable_sha256"]).lower(),
                str(manifest["image_base"]),
            )
            function_count = sum(
                1 for row in manifest.get("artifacts", [])
                if str(row.get("path", "")).endswith(".instructions.txt")
            )
            registry.record_manifest(
                manifest_key, manifest_digest, build_id, directory.name, function_count,
            )
            builds.add(build_id)
            receipts += 1
    return {
        "schema": "hoi4-re-ghidra-receipt-seed-v1", "source_job_id": source_job_id,
        "output_root": str(output_root), "jobs_seen": jobs_seen,
        "receipts_seeded": receipts, "receipts_skipped": skipped,
        "build_ids": sorted(builds),
    }

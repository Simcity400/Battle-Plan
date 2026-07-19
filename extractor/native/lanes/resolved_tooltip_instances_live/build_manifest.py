#!/usr/bin/env python3
"""Build deterministic A26 artifact and report manifests."""

from __future__ import annotations

import hashlib
import json
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[3]

ARTIFACT_PATHS = [
    "extractor/native/Cargo.toml",
    "extractor/native/Cargo.lock",
    "extractor/native/collector/Cargo.toml",
    "extractor/native/collector/src/lib.rs",
    "extractor/native/compact_contract/src/lib.rs",
    "extractor/native/domain_registry.json",
    "extractor/native/recorder_host/Cargo.toml",
    "extractor/native/recorder_host/src/world_fold.rs",
    "extractor/native/recorder_host/src/world_fold/tests.rs",
    "extractor/native/lanes/resolved_tooltip_instances_live/Cargo.toml",
    "extractor/native/lanes/resolved_tooltip_instances_live/src/lib.rs",
    "extractor/native/lanes/resolved_tooltip_instances_live/src/a729_hook.rs",
    "extractor/native/lanes/resolved_tooltip_instances_live/A26_GHIDRA_HANDOFF.md",
    "extractor/native/lanes/resolved_tooltip_instances_live/analyze_retained_cpu_traces.py",
    "extractor/native/lanes/resolved_tooltip_instances_live/retained_cpu_tooltip_analysis.json",
    "extractor/native/lanes/resolved_tooltip_instances_live/cleanup_report.json",
    "extractor/native/lanes/resolved_tooltip_instances_live/verify_a26.py",
    "extractor/native/lanes/resolved_tooltip_instances_live/verification.json",
    "extractor/native/lanes/runtime_text_inputs_live/Cargo.toml",
    "extractor/native/lanes/runtime_text_inputs_live/src/a729_hook.rs",
    "extractor/documents/dossiers/A26_RESOLVED_TOOLTIP_INSTANCES.json",
    "extractor/documents/RE_REMAINING.md",
    "extractor/documents/production_replay_handoff/replay_payload.schema.json",
    "replay_interface/re_evidence/rendering_laws/XUI05/capture_oracles.json",
]


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def rows(paths: list[str]) -> list[dict]:
    return [
        {"path": path, "bytes": (ROOT / path).stat().st_size, "sha256": digest(ROOT / path)}
        for path in sorted(paths)
    ]


def write(path: Path, payload: dict) -> None:
    path.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def main() -> None:
    artifact = {
        "schema": "extractor-a26-artifact-manifest-v1",
        "authority_executable_sha256": "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad",
        "files": rows(ARTIFACT_PATHS),
    }
    artifact_path = HERE / "artifact_manifest.json"
    write(artifact_path, artifact)

    readme_path = HERE / "README.md"
    marker = "Manifest SHA-256: `"
    readme = readme_path.read_text(encoding="utf-8")
    prefix, rest = readme.split(marker, 1)
    _, suffix = rest.split("`", 1)
    readme_path.write_text(prefix + marker + digest(artifact_path) + "`" + suffix, encoding="utf-8")

    full_paths = ARTIFACT_PATHS + [
        "extractor/native/lanes/resolved_tooltip_instances_live/README.md",
        "extractor/native/lanes/resolved_tooltip_instances_live/artifact_manifest.json",
        "extractor/native/lanes/resolved_tooltip_instances_live/build_manifest.py",
    ]
    write(
        HERE / "manifest.json",
        {
            "schema": "extractor-a26-full-manifest-v1",
            "artifact_manifest_sha256": digest(artifact_path),
            "files": rows(full_paths),
        },
    )
    print(digest(artifact_path))


if __name__ == "__main__":
    main()

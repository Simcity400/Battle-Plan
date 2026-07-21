from __future__ import annotations

import hashlib
import json
import os
import re
import tempfile
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


JOB_RE = re.compile(r"^\d{8}T\d{6}Z-[a-z0-9-]{1,48}-[0-9a-f]{8}$")
SHA256_RE = re.compile(r"^[0-9a-f]{64}$")
HEX_RE = re.compile(r"^(?:0x)?[0-9a-fA-F]+$")


def utc_now() -> str:
    return datetime.now(timezone.utc).isoformat().replace("+00:00", "Z")


def slug(value: str, fallback: str = "job", maximum: int = 48) -> str:
    result = re.sub(r"[^a-z0-9]+", "-", str(value).lower()).strip("-")[:maximum]
    return result or fallback


def sha256_file(path: Path, chunk_size: int = 1 << 20) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(chunk_size), b""):
            digest.update(block)
    return digest.hexdigest()


def sha256_bytes(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def atomic_json(path: Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    encoded = json.dumps(value, indent=2, sort_keys=True, ensure_ascii=False) + "\n"
    fd, temporary = tempfile.mkstemp(prefix=path.name + ".", suffix=".tmp", dir=path.parent)
    try:
        with os.fdopen(fd, "w", encoding="utf-8", newline="\n") as handle:
            handle.write(encoded)
            handle.flush()
            os.fsync(handle.fileno())
        os.replace(temporary, path)
    except BaseException:
        try:
            os.unlink(temporary)
        except OSError:
            pass
        raise


def parse_int(value: Any, field: str) -> int:
    if isinstance(value, bool):
        raise ValueError(f"{field} must be an integer or hexadecimal string")
    if isinstance(value, int):
        return value
    if isinstance(value, str) and HEX_RE.fullmatch(value.strip()):
        raw = value.strip()
        base = 16 if raw.lower().startswith("0x") or any(char in "abcdefABCDEF" for char in raw) else 10
        return int(raw, base)
    raise ValueError(f"{field} must be an integer or hexadecimal string")


def ensure_relative_artifact(job_dir: Path, relative: str) -> Path:
    if not isinstance(relative, str) or not relative or Path(relative).is_absolute():
        raise ValueError("artifact path must be a non-empty relative path")
    resolved_job = job_dir.resolve()
    resolved = (job_dir / relative).resolve()
    if resolved != resolved_job and resolved_job not in resolved.parents:
        raise ValueError("artifact path escapes the job directory")
    return resolved

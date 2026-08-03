from __future__ import annotations

import hashlib
import json
import re
import sqlite3
import unicodedata
from contextlib import contextmanager
from pathlib import Path
from typing import Any, Iterable, Iterator

from common import utc_now


SCHEMA_VERSION = 1
GHIDRA_JOB_ID_RE = re.compile(
    r"^[0-9]{8}T[0-9]{6}Z-[a-z0-9-]{1,40}-[0-9a-f]{8}$"
)


SCHEMA = """
CREATE TABLE IF NOT EXISTS meta (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS builds (
    build_id TEXT PRIMARY KEY,
    binary_id TEXT NOT NULL,
    sha256 TEXT NOT NULL UNIQUE,
    image_base TEXT NOT NULL,
    exe_path TEXT,
    version_label TEXT,
    registered_utc TEXT NOT NULL,
    inventory_status TEXT NOT NULL DEFAULT 'not_indexed',
    inventory_sha256 TEXT
);
CREATE TABLE IF NOT EXISTS re_jobs (
    job_id TEXT PRIMARY KEY,
    operation TEXT NOT NULL,
    status TEXT NOT NULL,
    created_utc TEXT NOT NULL,
    completed_utc TEXT,
    request_json TEXT NOT NULL,
    result_json TEXT,
    error TEXT
);
CREATE TABLE IF NOT EXISTS artifacts (
    artifact_id INTEGER PRIMARY KEY AUTOINCREMENT,
    job_id TEXT NOT NULL,
    build_id TEXT,
    path TEXT NOT NULL,
    sha256 TEXT NOT NULL,
    bytes INTEGER NOT NULL,
    kind TEXT NOT NULL,
    UNIQUE(job_id, path)
);
CREATE TABLE IF NOT EXISTS functions (
    build_id TEXT NOT NULL,
    rva INTEGER NOT NULL,
    name TEXT NOT NULL,
    body_min INTEGER NOT NULL,
    body_max INTEGER NOT NULL,
    instruction_count INTEGER NOT NULL,
    call_count INTEGER NOT NULL,
    byte_sha256 TEXT NOT NULL,
    shape_sha256 TEXT NOT NULL,
    shape_text TEXT NOT NULL,
    source_job_id TEXT,
    source_artifact TEXT,
    source_artifact_sha256 TEXT,
    PRIMARY KEY(build_id, rva),
    FOREIGN KEY(build_id) REFERENCES builds(build_id)
);
CREATE INDEX IF NOT EXISTS functions_byte_idx ON functions(build_id, byte_sha256);
CREATE INDEX IF NOT EXISTS functions_shape_idx ON functions(build_id, shape_sha256);
CREATE TABLE IF NOT EXISTS ghidra_manifests (
    manifest_path TEXT PRIMARY KEY,
    manifest_sha256 TEXT NOT NULL,
    build_id TEXT NOT NULL,
    source_job_id TEXT NOT NULL,
    function_count INTEGER NOT NULL,
    ingested_utc TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS logical_symbols (
    symbol_id TEXT PRIMARY KEY,
    canonical_name TEXT NOT NULL,
    created_utc TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS symbol_instances (
    symbol_id TEXT NOT NULL,
    build_id TEXT NOT NULL,
    rva INTEGER NOT NULL,
    status TEXT NOT NULL,
    source_match_id TEXT,
    PRIMARY KEY(build_id, rva),
    FOREIGN KEY(symbol_id) REFERENCES logical_symbols(symbol_id),
    FOREIGN KEY(build_id, rva) REFERENCES functions(build_id, rva)
);
CREATE INDEX IF NOT EXISTS symbol_instances_symbol_idx ON symbol_instances(symbol_id, build_id);
CREATE TABLE IF NOT EXISTS build_matches (
    match_id TEXT PRIMARY KEY,
    source_build_id TEXT NOT NULL,
    target_build_id TEXT NOT NULL,
    source_rva INTEGER NOT NULL,
    target_rva INTEGER,
    method TEXT NOT NULL,
    score REAL NOT NULL,
    disposition TEXT NOT NULL,
    reasons_json TEXT NOT NULL,
    job_id TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS build_matches_pair_idx ON build_matches(source_build_id, target_build_id);
CREATE TABLE IF NOT EXISTS claims (
    claim_id TEXT PRIMARY KEY,
    subject TEXT NOT NULL,
    predicate TEXT NOT NULL,
    value_json TEXT NOT NULL,
    status TEXT NOT NULL,
    build_id TEXT,
    evidence_job_ids_json TEXT NOT NULL,
    created_utc TEXT NOT NULL,
    supersedes_claim_id TEXT
);
CREATE TABLE IF NOT EXISTS memory_paths (
    path_id TEXT PRIMARY KEY,
    build_id TEXT NOT NULL,
    base_rva INTEGER NOT NULL,
    offsets_json TEXT NOT NULL,
    deref_base INTEGER NOT NULL,
    value_type TEXT NOT NULL,
    max_bytes INTEGER NOT NULL,
    description TEXT NOT NULL,
    created_utc TEXT NOT NULL,
    FOREIGN KEY(build_id) REFERENCES builds(build_id)
);
"""


def make_build_id(binary_id: str, sha256: str) -> str:
    label = "-".join(part for part in "".join(
        char.lower() if char.isalnum() else " " for char in binary_id
    ).split())
    return f"{label[:48]}-{sha256[:12]}"


def native_symbol_id(build_id: str, rva: int) -> str:
    value = f"{build_id}:{rva:x}".encode("utf-8")
    return "sym_" + hashlib.sha256(value).hexdigest()[:20]


class Registry:
    def __init__(self, path: Path, read_only: bool = False):
        self.path = path
        self.read_only = read_only
        if read_only:
            if not self.path.is_file():
                raise FileNotFoundError(f"registry is missing: {self.path}")
            with self.connect() as connection:
                current = connection.execute(
                    "SELECT value FROM meta WHERE key='schema_version'"
                ).fetchone()
                if current is None or int(current[0]) != SCHEMA_VERSION:
                    value = None if current is None else current[0]
                    raise RuntimeError(f"unsupported registry schema version {value}")
            return
        self.path.parent.mkdir(parents=True, exist_ok=True)
        with self.connect() as connection:
            connection.executescript(SCHEMA)
            current = connection.execute("SELECT value FROM meta WHERE key='schema_version'").fetchone()
            if current is None:
                connection.execute(
                    "INSERT INTO meta(key, value) VALUES('schema_version', ?)",
                    (str(SCHEMA_VERSION),),
                )
            elif int(current[0]) != SCHEMA_VERSION:
                raise RuntimeError(f"unsupported registry schema version {current[0]}")

    @contextmanager
    def connect(self) -> Iterator[sqlite3.Connection]:
        if self.read_only:
            uri = self.path.resolve().as_uri() + "?mode=ro"
            connection = sqlite3.connect(uri, uri=True, timeout=30.0)
        else:
            connection = sqlite3.connect(self.path, timeout=30.0)
        try:
            connection.row_factory = sqlite3.Row
            if self.read_only:
                connection.execute("PRAGMA query_only=ON")
            else:
                connection.execute("PRAGMA journal_mode=WAL")
                connection.execute("PRAGMA foreign_keys=ON")
            connection.execute("PRAGMA busy_timeout=30000")
            yield connection
            if not self.read_only:
                connection.commit()
        except BaseException:
            if not self.read_only:
                connection.rollback()
            raise
        finally:
            connection.close()

    def upsert_build(
        self,
        binary_id: str,
        sha256: str,
        image_base: str,
        exe_path: str | None = None,
        version_label: str | None = None,
    ) -> str:
        build_id = make_build_id(binary_id, sha256)
        with self.connect() as connection:
            connection.execute(
                """
                INSERT INTO builds(build_id,binary_id,sha256,image_base,exe_path,version_label,registered_utc)
                VALUES(?,?,?,?,?,?,?)
                ON CONFLICT(sha256) DO UPDATE SET
                  binary_id=excluded.binary_id,
                  image_base=excluded.image_base,
                  exe_path=COALESCE(excluded.exe_path,builds.exe_path),
                  version_label=COALESCE(excluded.version_label,builds.version_label)
                """,
                (build_id, binary_id, sha256, image_base, exe_path, version_label, utc_now()),
            )
            row = connection.execute("SELECT build_id FROM builds WHERE sha256=?", (sha256,)).fetchone()
        assert row is not None
        return str(row[0])

    def get_build(self, identifier: str) -> dict[str, Any]:
        with self.connect() as connection:
            row = connection.execute(
                "SELECT * FROM builds WHERE build_id=? OR sha256=?",
                (identifier, identifier.lower()),
            ).fetchone()
        if row is None:
            raise ValueError(f"unknown build: {identifier}")
        return dict(row)

    def record_job(self, job_id: str, operation: str, request: dict[str, Any]) -> None:
        with self.connect() as connection:
            connection.execute(
                "INSERT INTO re_jobs(job_id,operation,status,created_utc,request_json) VALUES(?,?,?,?,?)",
                (job_id, operation, "running", utc_now(), json.dumps(request, sort_keys=True)),
            )

    def finish_job(self, job_id: str, result: dict[str, Any]) -> None:
        with self.connect() as connection:
            connection.execute(
                "UPDATE re_jobs SET status='complete',completed_utc=?,result_json=? WHERE job_id=?",
                (utc_now(), json.dumps(result, sort_keys=True), job_id),
            )

    def fail_job(self, job_id: str, error: str) -> None:
        with self.connect() as connection:
            connection.execute(
                "UPDATE re_jobs SET status='failed',completed_utc=?,error=? WHERE job_id=?",
                (utc_now(), error, job_id),
            )

    def add_artifact(
        self, job_id: str, path: str, sha256: str, byte_count: int,
        kind: str, build_id: str | None = None,
    ) -> None:
        with self.connect() as connection:
            connection.execute(
                """
                INSERT INTO artifacts(job_id,build_id,path,sha256,bytes,kind)
                VALUES(?,?,?,?,?,?)
                ON CONFLICT(job_id,path) DO UPDATE SET
                  build_id=excluded.build_id,sha256=excluded.sha256,
                  bytes=excluded.bytes,kind=excluded.kind
                """,
                (job_id, build_id, path, sha256, byte_count, kind),
            )

    def upsert_functions(self, rows: Iterable[dict[str, Any]]) -> int:
        count = 0
        with self.connect() as connection:
            connection.execute("BEGIN IMMEDIATE")
            for row in rows:
                connection.execute(
                    """
                    INSERT INTO functions(
                      build_id,rva,name,body_min,body_max,instruction_count,call_count,
                      byte_sha256,shape_sha256,shape_text,source_job_id,source_artifact,
                      source_artifact_sha256
                    ) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?)
                    ON CONFLICT(build_id,rva) DO UPDATE SET
                      name=excluded.name,body_min=excluded.body_min,body_max=excluded.body_max,
                      instruction_count=excluded.instruction_count,call_count=excluded.call_count,
                      byte_sha256=excluded.byte_sha256,shape_sha256=excluded.shape_sha256,
                      shape_text=excluded.shape_text,source_job_id=excluded.source_job_id,
                      source_artifact=excluded.source_artifact,
                      source_artifact_sha256=excluded.source_artifact_sha256
                    """,
                    (
                        row["build_id"], row["rva"], row["name"], row["body_min"], row["body_max"],
                        row["instruction_count"], row["call_count"], row["byte_sha256"],
                        row["shape_sha256"], row["shape_text"], row.get("source_job_id"),
                        row.get("source_artifact"), row.get("source_artifact_sha256"),
                    ),
                )
                symbol_id = native_symbol_id(row["build_id"], row["rva"])
                connection.execute(
                    "INSERT OR IGNORE INTO logical_symbols(symbol_id,canonical_name,created_utc) VALUES(?,?,?)",
                    (symbol_id, row["name"], utc_now()),
                )
                connection.execute(
                    """
                    INSERT OR IGNORE INTO symbol_instances(symbol_id,build_id,rva,status)
                    VALUES(?,?,?,'native')
                    """,
                    (symbol_id, row["build_id"], row["rva"]),
                )
                count += 1
        return count

    def functions_for_build(self, build_id: str) -> list[dict[str, Any]]:
        with self.connect() as connection:
            rows = connection.execute(
                "SELECT * FROM functions WHERE build_id=? ORDER BY rva", (build_id,)
            ).fetchall()
        return [dict(row) for row in rows]

    def mark_inventory(self, build_id: str, status: str, digest: str | None = None) -> None:
        with self.connect() as connection:
            connection.execute(
                "UPDATE builds SET inventory_status=?,inventory_sha256=? WHERE build_id=?",
                (status, digest, build_id),
            )

    def current_manifest(self, manifest_path: str, manifest_sha256: str) -> dict[str, Any] | None:
        with self.connect() as connection:
            row = connection.execute(
                "SELECT * FROM ghidra_manifests WHERE manifest_path=? AND manifest_sha256=?",
                (manifest_path, manifest_sha256),
            ).fetchone()
        return None if row is None else dict(row)

    def record_manifest(
        self, manifest_path: str, manifest_sha256: str, build_id: str,
        source_job_id: str, function_count: int,
    ) -> None:
        with self.connect() as connection:
            connection.execute(
                """
                INSERT INTO ghidra_manifests(
                  manifest_path,manifest_sha256,build_id,source_job_id,function_count,ingested_utc
                ) VALUES(?,?,?,?,?,?)
                ON CONFLICT(manifest_path) DO UPDATE SET
                  manifest_sha256=excluded.manifest_sha256,build_id=excluded.build_id,
                  source_job_id=excluded.source_job_id,function_count=excluded.function_count,
                  ingested_utc=excluded.ingested_utc
                """,
                (manifest_path, manifest_sha256, build_id, source_job_id, function_count, utc_now()),
            )

    def source_symbol(self, build_id: str, rva: int) -> str:
        with self.connect() as connection:
            row = connection.execute(
                "SELECT symbol_id FROM symbol_instances WHERE build_id=? AND rva=?",
                (build_id, rva),
            ).fetchone()
        if row is None:
            raise ValueError(f"function has no logical symbol: {build_id}+0x{rva:x}")
        return str(row[0])

    def source_symbols(self, build_id: str) -> dict[int, str]:
        with self.connect() as connection:
            rows = connection.execute(
                "SELECT rva,symbol_id FROM symbol_instances WHERE build_id=?", (build_id,)
            ).fetchall()
        return {int(row["rva"]): str(row["symbol_id"]) for row in rows}

    def bind_matches(self, rows: Iterable[dict[str, Any]]) -> None:
        with self.connect() as connection:
            connection.execute("BEGIN IMMEDIATE")
            for row in rows:
                connection.execute(
                    """
                    INSERT OR REPLACE INTO build_matches(
                      match_id,source_build_id,target_build_id,source_rva,target_rva,
                      method,score,disposition,reasons_json,job_id
                    ) VALUES(?,?,?,?,?,?,?,?,?,?)
                    """,
                    (
                        row["match_id"], row["source_build_id"], row["target_build_id"],
                        row["source_rva"], row["target_rva"], row["method"], row["score"],
                        row["disposition"], json.dumps(row["reasons"], sort_keys=True), row["job_id"],
                    ),
                )
                if row["target_rva"] is not None and row["disposition"] in {"exact", "high_confidence"}:
                    connection.execute(
                        """
                        UPDATE symbol_instances SET symbol_id=?,status='ported',source_match_id=?
                        WHERE build_id=? AND rva=?
                        """,
                        (
                            row["symbol_id"], row["match_id"], row["target_build_id"],
                            row["target_rva"],
                        ),
                    )

    def bind_match(
        self, match_id: str, source_build_id: str, target_build_id: str,
        source_rva: int, target_rva: int | None, method: str, score: float,
        disposition: str, reasons: dict[str, Any], job_id: str,
    ) -> None:
        self.bind_matches([{
            "match_id": match_id, "source_build_id": source_build_id,
            "target_build_id": target_build_id, "source_rva": source_rva,
            "target_rva": target_rva, "method": method, "score": score,
            "disposition": disposition, "reasons": reasons, "job_id": job_id,
            "symbol_id": self.source_symbol(source_build_id, source_rva),
        }])

    def register_memory_path(
        self, path_id: str, build_id: str, base_rva: int, offsets: list[int],
        deref_base: bool, value_type: str, max_bytes: int, description: str,
    ) -> None:
        with self.connect() as connection:
            connection.execute(
                """
                INSERT INTO memory_paths(
                  path_id,build_id,base_rva,offsets_json,deref_base,value_type,
                  max_bytes,description,created_utc
                ) VALUES(?,?,?,?,?,?,?,?,?)
                ON CONFLICT(path_id) DO UPDATE SET
                  build_id=excluded.build_id,base_rva=excluded.base_rva,
                  offsets_json=excluded.offsets_json,deref_base=excluded.deref_base,
                  value_type=excluded.value_type,max_bytes=excluded.max_bytes,
                  description=excluded.description
                """,
                (
                    path_id, build_id, base_rva, json.dumps(offsets), int(deref_base),
                    value_type, max_bytes, description, utc_now(),
                ),
            )

    def memory_path(self, path_id: str) -> dict[str, Any]:
        with self.connect() as connection:
            row = connection.execute("SELECT * FROM memory_paths WHERE path_id=?", (path_id,)).fetchone()
        if row is None:
            raise ValueError(f"unknown registered memory path: {path_id}")
        value = dict(row)
        value["offsets"] = json.loads(value.pop("offsets_json"))
        value["deref_base"] = bool(value["deref_base"])
        return value

    def resolve_symbol(self, symbol_id: str, build_id: str) -> dict[str, Any]:
        with self.connect() as connection:
            row = connection.execute(
                """
                SELECT s.symbol_id,s.canonical_name,i.build_id,i.rva,i.status,
                       f.name,f.body_min,f.body_max,f.instruction_count,f.call_count
                FROM logical_symbols s
                JOIN symbol_instances i ON i.symbol_id=s.symbol_id
                JOIN functions f ON f.build_id=i.build_id AND f.rva=i.rva
                WHERE s.symbol_id=? AND i.build_id=?
                """,
                (symbol_id, build_id),
            ).fetchone()
        if row is None:
            raise ValueError(f"symbol {symbol_id} has no instance in build {build_id}")
        return dict(row)

    def record_claim(self, value: dict[str, Any]) -> str:
        claim_id = value.get("claim_id") or "claim_" + hashlib.sha256(
            json.dumps(value, sort_keys=True).encode("utf-8")
        ).hexdigest()[:20]
        with self.connect() as connection:
            connection.execute(
                """
                INSERT INTO claims(
                  claim_id,subject,predicate,value_json,status,build_id,
                  evidence_job_ids_json,created_utc,supersedes_claim_id
                ) VALUES(?,?,?,?,?,?,?,?,?)
                """,
                (
                    claim_id, value["subject"], value["predicate"],
                    json.dumps(value["value"], sort_keys=True), value["status"],
                    value.get("build_id"), json.dumps(value.get("evidence_job_ids", [])),
                    utc_now(), value.get("supersedes_claim_id"),
                ),
            )
        return claim_id

    def claims_for_function(
        self,
        address: int,
        image_base: int,
        executable_sha256: str,
        function_name: str | None = None,
    ) -> list[dict[str, Any]]:
        """Return existing claims that name an exact VA, RVA, symbol, or function."""
        rva = address - image_base
        with self.connect() as connection:
            build = connection.execute(
                "SELECT build_id FROM builds WHERE sha256=?", (executable_sha256.lower(),)
            ).fetchone()
            build_id = None if build is None else str(build[0])
            symbol_ids: list[str] = []
            if build_id is not None and rva >= 0:
                symbol_ids = [
                    str(row[0])
                    for row in connection.execute(
                        "SELECT symbol_id FROM symbol_instances WHERE build_id=? AND rva=?",
                        (build_id, rva),
                    )
                ]
            # Orphan build ids (rows whose build no longer exists in `builds`) are
            # legacy claims, not claims about some other binary: dropping them hides
            # real findings that name this exact address. They are surfaced with
            # build_attribution='orphan' so callers can weigh them accordingly.
            orphan = (
                "(build_id IS NOT NULL AND "
                "build_id NOT IN (SELECT build_id FROM builds))"
            )
            if build_id is None:
                rows = connection.execute(
                    f"SELECT * FROM claims WHERE build_id IS NULL OR {orphan} "
                    "ORDER BY created_utc DESC,claim_id"
                ).fetchall()
            else:
                rows = connection.execute(
                    f"""
                    SELECT * FROM claims
                    WHERE build_id IS NULL OR build_id=? OR {orphan}
                    ORDER BY created_utc DESC,claim_id
                    """,
                    (build_id,),
                ).fetchall()
            known_builds = {
                str(row[0]) for row in connection.execute("SELECT build_id FROM builds")
            }

        # Boundaries reject both hex continuation (0x...920 must not match 0x...9200)
        # and surrounding identifier characters (prefix0x...suffix is a different
        # token, not a citation of this address).
        expressions = [
            re.compile(rf"(?<![0-9a-z_])0x{address:x}(?![0-9a-z_])"),
            re.compile(rf"(?<![a-z0-9_])fun_{address:x}(?![0-9a-z_])"),
            re.compile(rf"(?<![a-z0-9_])func_{address:x}(?![0-9a-z_])"),
        ]
        if rva >= 0:
            expressions.extend(
                [
                    re.compile(rf"(?<![a-z0-9_])rva:0x0*{rva:x}(?![0-9a-z_])"),
                    re.compile(rf"(?<![a-z0-9_])\+0x0*{rva:x}(?![0-9a-z_])"),
                ]
            )
        exact_tokens = {value.lower() for value in symbol_ids}
        if function_name:
            exact_tokens.add(function_name.lower())

        results = []
        for row in rows:
            haystack = "\n".join(
                str(row[key]).lower()
                for key in ("subject", "predicate", "value_json")
                if row[key] is not None
            )
            exact_match = any(
                re.search(rf"(?<![a-z0-9_]){re.escape(token)}(?![a-z0-9_])", haystack)
                for token in exact_tokens if token
            )
            if not exact_match and not any(expression.search(haystack) for expression in expressions):
                continue
            value = dict(row)
            value["value"] = json.loads(value.pop("value_json"))
            value["evidence_job_ids"] = json.loads(value.pop("evidence_job_ids_json"))
            row_build = value.get("build_id")
            if row_build is None:
                value["build_attribution"] = "build_agnostic"
            elif build_id is not None and str(row_build) == build_id:
                value["build_attribution"] = "exact"
            elif str(row_build) not in known_builds:
                value["build_attribution"] = "orphan"
            else:
                value["build_attribution"] = "other_build"
            results.append(value)
        return results

    def search_function_proposed_names(
        self,
        query: str,
        executable_sha256: str,
    ) -> list[dict[str, Any]]:
        """Find build-relevant annotation claims by their proposed function name."""
        with self.connect() as connection:
            build = connection.execute(
                "SELECT build_id FROM builds WHERE sha256=?", (executable_sha256.lower(),)
            ).fetchone()
            build_id = None if build is None else str(build[0])
            orphan = (
                "(build_id IS NOT NULL AND "
                "build_id NOT IN (SELECT build_id FROM builds))"
            )
            if build_id is None:
                rows = connection.execute(
                    f"""
                    SELECT * FROM claims
                    WHERE predicate='annotation'
                      AND (build_id IS NULL OR {orphan})
                    ORDER BY created_utc DESC,claim_id
                    """
                ).fetchall()
            else:
                rows = connection.execute(
                    f"""
                    SELECT * FROM claims
                    WHERE predicate='annotation'
                      AND (build_id IS NULL OR build_id=? OR {orphan})
                    ORDER BY created_utc DESC,claim_id
                    """,
                    (build_id,),
                ).fetchall()

        needle = query.casefold()
        results = []
        for row in rows:
            value = json.loads(row["value_json"])
            proposed_name = value.get("proposed_name")
            if not isinstance(proposed_name, str) or needle not in proposed_name.casefold():
                continue
            raw_address = value.get("address")
            if not isinstance(raw_address, str) or re.fullmatch(
                r"0x[0-9a-fA-F]{1,16}", raw_address
            ) is None:
                continue
            results.append(
                {
                    "address": int(raw_address, 16),
                    "proposed_name": proposed_name,
                    "claim_id": str(row["claim_id"]),
                    "status": str(row["status"]),
                    "evidence_refs": value.get("evidence_refs", []),
                }
            )
        return results

    def record_function_annotation(
        self,
        address: int,
        image_base: int,
        executable_sha256: str,
        note: str,
        evidence_refs: list[Any],
        evidence_root: Path | None = None,
        name: str | None = None,
    ) -> dict[str, Any]:
        """Append a proposed function finding to the existing WAL claims journal."""
        if not isinstance(note, str) or not note.strip():
            raise ValueError("note must be a non-empty string")
        proposed_name = None
        if name is not None:
            if not isinstance(name, str):
                raise ValueError("name must be a string")
            if any(unicodedata.category(character) == "Cc" for character in name):
                raise ValueError("name must not contain control characters")
            proposed_name = name.strip()
            if not proposed_name:
                raise ValueError("name must not be empty")
            if len(proposed_name) > 256:
                raise ValueError("name must be at most 256 characters")
        if not isinstance(evidence_refs, list) or not evidence_refs:
            raise ValueError("evidence_refs must contain at least one artifact citation")
        if evidence_root is None:
            raise ValueError("evidence_root is required to verify annotation citations")
        evidence_root = evidence_root.resolve()
        validated_refs = []
        for reference in evidence_refs:
            if not isinstance(reference, dict):
                raise ValueError("each evidence reference must be an object")
            job_id = reference.get("job_id")
            relative = reference.get("path")
            digest = reference.get("sha256")
            if not isinstance(job_id, str) or GHIDRA_JOB_ID_RE.fullmatch(job_id) is None:
                raise ValueError("evidence reference job_id is invalid")
            if not isinstance(relative, str) or not relative or Path(relative).is_absolute():
                raise ValueError("evidence reference path must be job-relative")
            job_dir = (evidence_root / job_id).resolve()
            artifact = (job_dir / relative).resolve()
            if job_dir not in artifact.parents:
                raise ValueError("evidence reference path escapes its job")
            job_path = job_dir / "job.json"
            if not job_path.is_file():
                raise ValueError(f"evidence reference job does not exist: {job_id}")
            receipt = json.loads(job_path.read_text(encoding="utf-8"))
            if receipt.get("job_id") != job_id or receipt.get("status") != "complete":
                raise ValueError(f"evidence reference job is not complete: {job_id}")
            if not artifact.is_file():
                raise ValueError("evidence reference artifact does not exist")
            hasher = hashlib.sha256()
            with artifact.open("rb") as handle:
                for chunk in iter(lambda: handle.read(1024 * 1024), b""):
                    hasher.update(chunk)
            actual_sha = hasher.hexdigest()
            if not isinstance(digest, str) or digest.lower() != actual_sha:
                raise ValueError("evidence reference SHA-256 disagrees with artifact")
            validated_refs.append(
                {"job_id": job_id, "path": artifact.relative_to(job_dir).as_posix(),
                 "sha256": actual_sha, "bytes": artifact.stat().st_size}
            )
        # Fail before opening a write transaction if a reference cannot be journaled.
        json.dumps(validated_refs, sort_keys=True)
        with self.connect() as connection:
            build = connection.execute(
                "SELECT build_id FROM builds WHERE sha256=?", (executable_sha256.lower(),)
            ).fetchone()
        build_id = None if build is None else str(build[0])
        created = utc_now()
        evidence_job_ids = []
        for reference in validated_refs:
            evidence_job_ids.append(reference["job_id"])
        payload = {
            "address": f"0x{address:x}",
            "rva": f"0x{address - image_base:x}" if address >= image_base else None,
            "note": note.strip(),
            "evidence_refs": validated_refs,
            "annotation_created_utc": created,
        }
        if proposed_name is not None:
            payload["proposed_name"] = proposed_name
        claim_seed = json.dumps(payload, sort_keys=True).encode("utf-8")
        claim_id = "claim_" + hashlib.sha256(claim_seed).hexdigest()[:20]
        recorded = {
            "claim_id": claim_id,
            "subject": f"a729:function:0x{address:x}",
            "predicate": "annotation",
            "value": payload,
            "status": "proposed",
            "build_id": build_id,
            "evidence_job_ids": list(dict.fromkeys(evidence_job_ids)),
        }
        self.record_claim(recorded)
        return {
            "claim_id": claim_id,
            "status": "proposed",
            "build_id": build_id,
            "created_utc": created,
            "proposed_name": proposed_name,
        }

    def query(self, kind: str, arguments: dict[str, Any]) -> list[dict[str, Any]]:
        limit = arguments.get("limit", 100)
        if not isinstance(limit, int) or not 1 <= limit <= 500:
            raise ValueError("limit must be an integer from 1 through 500")
        with self.connect() as connection:
            if kind == "builds":
                rows = connection.execute("SELECT * FROM builds ORDER BY registered_utc DESC LIMIT ?", (limit,)).fetchall()
            elif kind == "jobs":
                rows = connection.execute("SELECT * FROM re_jobs ORDER BY created_utc DESC LIMIT ?", (limit,)).fetchall()
            elif kind == "symbols":
                needle = f"%{arguments.get('name', '')}%"
                build_id = arguments.get("build_id")
                sql = """
                    SELECT s.symbol_id,s.canonical_name,i.build_id,i.rva,i.status,f.name
                    FROM logical_symbols s JOIN symbol_instances i ON i.symbol_id=s.symbol_id
                    JOIN functions f ON f.build_id=i.build_id AND f.rva=i.rva
                    WHERE (s.canonical_name LIKE ? OR f.name LIKE ?)
                """
                params: list[Any] = [needle, needle]
                if build_id:
                    sql += " AND i.build_id=?"
                    params.append(build_id)
                sql += " ORDER BY s.canonical_name,i.build_id LIMIT ?"
                params.append(limit)
                rows = connection.execute(sql, params).fetchall()
            elif kind == "matches":
                rows = connection.execute(
                    "SELECT * FROM build_matches ORDER BY rowid DESC LIMIT ?", (limit,)
                ).fetchall()
            elif kind == "claims":
                rows = connection.execute("SELECT * FROM claims ORDER BY created_utc DESC LIMIT ?", (limit,)).fetchall()
            elif kind == "memory_paths":
                rows = connection.execute("SELECT * FROM memory_paths ORDER BY path_id LIMIT ?", (limit,)).fetchall()
            elif kind == "coverage":
                rows = connection.execute(
                    """
                    SELECT b.build_id,b.binary_id,b.sha256,b.inventory_status,
                           COUNT(f.rva) AS function_count,
                           COUNT(DISTINCT i.symbol_id) AS symbol_count
                    FROM builds b LEFT JOIN functions f ON f.build_id=b.build_id
                    LEFT JOIN symbol_instances i ON i.build_id=f.build_id AND i.rva=f.rva
                    GROUP BY b.build_id ORDER BY b.registered_utc DESC LIMIT ?
                    """,
                    (limit,),
                ).fetchall()
            else:
                raise ValueError(f"unsupported query kind: {kind}")
        return [dict(row) for row in rows]

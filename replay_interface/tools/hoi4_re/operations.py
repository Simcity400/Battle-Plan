from __future__ import annotations

import importlib.util
import json
import os
import shutil
import subprocess
import traceback
import uuid
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Callable

from common import atomic_json, ensure_relative_artifact, parse_int, sha256_file, slug, utc_now
from correlation import correlate
from diffing import diff_builds
from emulation import emulate_function
from evidence import ingest_ghidra_jobs, seed_manifest_receipts
from offline_analysis import (
    candidate_census, cross_analyze, inspect_pe, published_index_status, toolchain_status,
)
from registry import Registry
from runtime_ops import observe_runtime, runtime_status, trace_calls
from save_ops import load_hoi4_save


@dataclass(frozen=True)
class Config:
    workspace: Path
    state_root: Path
    ghidra_output_root: Path
    ghidra_install_dir: Path
    ghidra_script_dir: Path
    ghidra_timeout_seconds: int = 7200
    rakaly_path: Path = Path(
        r"C:\Users\Example\Downloads\rakaly_cli\rakaly-0.8.17-x86_64-pc-windows-msvc\rakaly.exe"
    )
    save_root: Path = Path(
        r"C:\Users\Example\OneDrive\Documents\Paradox Interactive\Hearts of Iron IV\save games"
    )
    angr_python: Path = Path(r"C:\hoi4_evidence\toolchain\py312\Scripts\python.exe")
    rizin_path: Path = Path(r"C:\Program Files\Rizin\bin\rizin.exe")
    dynamorio_root: Path = Path(
        r"C:\Users\Example\AppData\Local\Microsoft\WinGet\Packages\DynamoRIO.DynamoRIO_Microsoft.Winget.Source_8wekyb3d8bbwe\DynamoRIO-Windows-11.91.20536"
    )
    windbg_ttd_path: Path = Path(
        r"C:\Program Files\WindowsApps\Microsoft.WinDbg_1.2606.22001.0_x64__8wekyb3d8bbwe\amd64\ttd\TTD.exe"
    )
    function_index_path: Path = Path(r"C:\hoi4_evidence\agent_re\function_index.sqlite3")

    @classmethod
    def from_env(cls) -> "Config":
        workspace = Path(os.environ.get("BATTLE_PLAN_RE_WORKSPACE", os.environ.get("HOI4_RE_WORKSPACE", r"")))
        state_root = Path(os.environ.get("BATTLE_PLAN_RE_STATE_ROOT", os.environ.get("HOI4_RE_STATE_ROOT", r"C:\hoi4_evidence\agent_re")))
        return cls(
            workspace=workspace,
            state_root=state_root,
            ghidra_output_root=Path(os.environ.get(
                "BATTLE_PLAN_RE_GHIDRA_OUTPUT_ROOT", r"C:\hoi4_evidence\ghidra_agent_exports"
            )),
            ghidra_install_dir=Path(os.environ.get(
                "BATTLE_PLAN_RE_GHIDRA_INSTALL_DIR",
                r"C:\Users\Example\Downloads\ghidra_12.1.2_PUBLIC_20260605\ghidra_12.1.2_PUBLIC",
            )),
            ghidra_script_dir=Path(os.environ.get(
                "BATTLE_PLAN_RE_GHIDRA_SCRIPT_DIR",
                str(workspace / "replay_interface" / "tools" / "hoi4_re" / "ghidra_scripts"),
            )),
            ghidra_timeout_seconds=int(os.environ.get("BATTLE_PLAN_RE_GHIDRA_TIMEOUT_SECONDS", os.environ.get("HOI4_RE_GHIDRA_TIMEOUT_SECONDS", "7200"))),
            rakaly_path=Path(os.environ.get(
                "BATTLE_PLAN_RE_RAKALY_PATH",
                r"C:\Users\Example\Downloads\rakaly_cli\rakaly-0.8.17-x86_64-pc-windows-msvc\rakaly.exe",
            )),
            save_root=Path(os.environ.get(
                "BATTLE_PLAN_RE_SAVE_ROOT",
                r"C:\Users\Example\OneDrive\Documents\Paradox Interactive\Hearts of Iron IV\save games",
            )),
            angr_python=Path(os.environ.get(
                "BATTLE_PLAN_RE_ANGR_PYTHON", r"C:\hoi4_evidence\toolchain\py312\Scripts\python.exe",
            )),
            rizin_path=Path(os.environ.get(
                "BATTLE_PLAN_RE_RIZIN_PATH", r"C:\Program Files\Rizin\bin\rizin.exe",
            )),
            dynamorio_root=Path(os.environ.get(
                "BATTLE_PLAN_RE_DYNAMORIO_ROOT",
                r"C:\Users\Example\AppData\Local\Microsoft\WinGet\Packages\DynamoRIO.DynamoRIO_Microsoft.Winget.Source_8wekyb3d8bbwe\DynamoRIO-Windows-11.91.20536",
            )),
            windbg_ttd_path=Path(os.environ.get(
                "BATTLE_PLAN_RE_WINDBG_TTD_PATH",
                r"C:\Program Files\WindowsApps\Microsoft.WinDbg_1.2606.22001.0_x64__8wekyb3d8bbwe\amd64\ttd\TTD.exe",
            )),
            function_index_path=Path(os.environ.get(
                "BATTLE_PLAN_RE_FUNCTION_INDEX", r"C:\hoi4_evidence\agent_re\function_index.sqlite3",
            )),
        )

    @property
    def jobs_root(self) -> Path:
        return self.state_root / "jobs"

    @property
    def registry_path(self) -> Path:
        return self.state_root / "registry.sqlite3"

    @property
    def runtime_lease(self) -> Path:
        return self.state_root / "locks" / "live-runtime.lock"

    @property
    def analyze_headless(self) -> Path:
        return self.ghidra_install_dir / "support" / "analyzeHeadless.bat"

    @property
    def drrun_path(self) -> Path:
        return self.dynamorio_root / "bin64" / "drrun.exe"

    @property
    def drcov_path(self) -> Path:
        return self.dynamorio_root / "tools" / "lib64" / "release" / "drcov.dll"


def _pe_identity(path: Path) -> tuple[str, str]:
    import pefile

    pe = pefile.PE(str(path), fast_load=True)
    image_base = f"0x{int(pe.OPTIONAL_HEADER.ImageBase):x}"
    version = None
    fixed = getattr(pe, "VS_FIXEDFILEINFO", None)
    if fixed:
        info = fixed[0]
        version = ".".join(str(value) for value in (
            info.FileVersionMS >> 16, info.FileVersionMS & 0xFFFF,
            info.FileVersionLS >> 16, info.FileVersionLS & 0xFFFF,
        ))
    pe.close()
    return (f"{path.name} {version}" if version else path.name, image_base)


def _load_inventory(path: Path, build_id: str, source_job_id: str) -> list[dict[str, Any]]:
    digest = sha256_file(path)
    rows = []
    for number, raw in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        if not raw.strip():
            continue
        value = json.loads(raw)
        required = {
            "rva", "name", "body_min", "body_max", "instruction_count", "call_count",
            "byte_sha256", "shape_sha256", "shape_text",
        }
        missing = sorted(required - set(value))
        if missing:
            raise ValueError(f"inventory line {number} is missing {missing}")
        rows.append({
            "build_id": build_id,
            "rva": int(str(value["rva"]), 0),
            "name": value["name"],
            "body_min": int(str(value["body_min"]), 0),
            "body_max": int(str(value["body_max"]), 0),
            "instruction_count": int(value["instruction_count"]),
            "call_count": int(value["call_count"]),
            "byte_sha256": value["byte_sha256"],
            "shape_sha256": value["shape_sha256"],
            "shape_text": value["shape_text"],
            "source_job_id": source_job_id,
            "source_artifact": str(path),
            "source_artifact_sha256": digest,
        })
    return rows


class Broker:
    def __init__(self, config: Config):
        self.config = config
        self.config.state_root.mkdir(parents=True, exist_ok=True)
        self.config.jobs_root.mkdir(parents=True, exist_ok=True)
        self.registry = Registry(self.config.registry_path)

    def status(self) -> dict[str, Any]:
        packages = {
            name: importlib.util.find_spec(name) is not None
            for name in ("pefile", "frida", "unicorn")
        }
        builds = self.registry.query("builds", {"limit": 500})
        index_publication = published_index_status(
            self.config.function_index_path, self.config.ghidra_output_root, builds,
        )
        checks = {
            "workspace": self.config.workspace.is_dir(),
            "state_root": self.config.state_root.is_dir(),
            "ghidra_output_root": self.config.ghidra_output_root.is_dir(),
            "analyze_headless": self.config.analyze_headless.is_file(),
            "build_inventory_script": (self.config.ghidra_script_dir / "BuildInventory.java").is_file(),
            "rakaly": self.config.rakaly_path.is_file(),
            "save_root": self.config.save_root.is_dir(),
            "function_index": index_publication["available"],
            **{f"python_{name}": present for name, present in packages.items()},
        }
        coverage = self.registry.query("coverage", {"limit": 500})
        toolchain = toolchain_status(self.config)
        return {
            "ready": all(checks.values()) and toolchain["ready"],
            "checks": checks,
            "toolchain": toolchain,
            "function_index": index_publication,
            "mode": "agent-native durable jobs; static offline by default; live operations explicit",
            "paths": {
                "workspace": str(self.config.workspace),
                "state_root": str(self.config.state_root),
                "registry": str(self.config.registry_path),
                "jobs": str(self.config.jobs_root),
                "ghidra_output_root": str(self.config.ghidra_output_root),
                "save_root": str(self.config.save_root),
                "function_index": str(self.config.function_index_path),
            },
            "build_coverage": coverage,
            "runtime": runtime_status(self.config.workspace),
            "operations": [
                "register_build", "inspect_build", "candidate_census", "cross_analyze_function",
                "ingest_ghidra_jobs", "inventory_build", "diff_builds", "migrate_build",
                "register_memory_path", "observe_runtime", "trace_calls", "emulate_function",
                "correlate_save_memory", "record_claim",
            ],
            "safety": {
                "current_ghidra_broker_modified": False,
                "new_build_analysis_projects": "isolated writable projects under state_root",
                "runtime_reads": "registered paths and exact SHA-256 only",
                "frida": "bounded hook-based observation with exclusive lease and no force bypass",
                "emulation": "leaf functions only",
                "promotion": "never automatic",
            },
        }

    def _new_job(self, operation: str, inputs: dict[str, Any]) -> tuple[str, Path, dict[str, Any]]:
        stamp = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")
        job_id = f"{stamp}-{slug(operation)}-{uuid.uuid4().hex[:8]}"
        job_dir = self.config.jobs_root / job_id
        job_dir.mkdir(parents=False, exist_ok=False)
        request = {"schema": "hoi4-re-job-v1", "job_id": job_id, "operation": operation, "inputs": inputs}
        record = {**request, "status": "running", "created_utc": utc_now()}
        atomic_json(job_dir / "job.json", record)
        self.registry.record_job(job_id, operation, request)
        return job_id, job_dir, record

    def _compact(self, result: dict[str, Any]) -> dict[str, Any]:
        large = {
            "matches", "unresolved", "messages", "save_deltas", "memory_deltas", "candidates",
            "observations", "compatibility_pack", "diff", "invalid_details",
        }
        compact = {key: value for key, value in result.items() if key not in large}
        for key in large:
            if isinstance(result.get(key), list):
                compact[f"{key}_count"] = len(result[key])
        return compact

    def _index_job_artifacts(self, job_id: str, job_dir: Path, build_id: str | None = None) -> list[dict[str, Any]]:
        indexed = []
        for path in sorted(job_dir.rglob("*")):
            if not path.is_file() or path.name == "job.json":
                continue
            relative = path.relative_to(job_dir).as_posix()
            digest = sha256_file(path)
            kind = "json" if path.suffix.lower() == ".json" else "log" if path.suffix.lower() == ".log" else "artifact"
            self.registry.add_artifact(job_id, relative, digest, path.stat().st_size, kind, build_id)
            indexed.append({"path": relative, "sha256": digest, "bytes": path.stat().st_size, "kind": kind})
        return indexed

    def run(self, operation: str, inputs: dict[str, Any]) -> dict[str, Any]:
        if not isinstance(inputs, dict):
            raise ValueError("inputs must be an object")
        handlers: dict[str, Callable[[dict[str, Any], str, Path], dict[str, Any]]] = {
            "register_build": self._register_build,
            "inspect_build": self._inspect_build,
            "candidate_census": self._candidate_census,
            "cross_analyze_function": self._cross_analyze_function,
            "ingest_ghidra_jobs": self._ingest_ghidra_jobs,
            "inventory_build": self._inventory_build,
            "diff_builds": self._diff_builds,
            "migrate_build": self._migrate_build,
            "register_memory_path": self._register_memory_path,
            "observe_runtime": self._observe_runtime,
            "trace_calls": self._trace_calls,
            "emulate_function": self._emulate_function,
            "correlate_save_memory": self._correlate_save_memory,
            "record_claim": self._record_claim,
        }
        if operation not in handlers:
            raise ValueError(f"unsupported operation: {operation}")
        job_id, job_dir, record = self._new_job(operation, inputs)
        try:
            result = handlers[operation](inputs, job_id, job_dir)
            atomic_json(job_dir / "result.json", result)
            artifacts = self._index_job_artifacts(job_id, job_dir, result.get("build_id"))
            compact = self._compact(result)
            self.registry.finish_job(job_id, compact)
            record.update(status="complete", completed_utc=utc_now(), summary=compact, artifacts=artifacts)
            atomic_json(job_dir / "job.json", record)
            return {"job_id": job_id, "job_dir": str(job_dir), "summary": compact, "artifacts": artifacts}
        except Exception as error:
            message = f"{type(error).__name__}: {error}"
            self.registry.fail_job(job_id, message)
            (job_dir / "failure.traceback.log").write_text(traceback.format_exc(), encoding="utf-8")
            try:
                artifacts = self._index_job_artifacts(job_id, job_dir)
            except Exception as artifact_error:
                artifacts = []
                record["artifact_index_error"] = f"{type(artifact_error).__name__}: {artifact_error}"
            record.update(
                status="failed", completed_utc=utc_now(), error=message, artifacts=artifacts,
            )
            atomic_json(job_dir / "job.json", record)
            raise

    def _register_build(self, inputs: dict[str, Any], _job_id: str, _job_dir: Path) -> dict[str, Any]:
        path = Path(inputs["exe_path"]).resolve()
        if not path.is_file() or path.name.lower() != "hoi4.exe":
            raise ValueError("exe_path must identify an existing hoi4.exe")
        digest = sha256_file(path)
        snapshot = self._materialize_snapshot(path, digest)
        if sha256_file(path) != digest:
            raise RuntimeError("executable changed while it was being registered")
        derived_binary_id, image_base = _pe_identity(snapshot)
        try:
            existing = self.registry.get_build(digest)
        except ValueError:
            existing = None
        version_label = inputs.get("version_label") or (existing or {}).get("version_label")
        if inputs.get("binary_id"):
            binary_id = str(inputs["binary_id"])
        elif existing and existing.get("binary_id") != path.name:
            binary_id = str(existing["binary_id"])
        elif version_label and derived_binary_id == path.name:
            binary_id = f"{path.name} {version_label}"
        else:
            binary_id = derived_binary_id
        build_id = self.registry.upsert_build(
            binary_id, digest, image_base, str(path), version_label
        )
        build = self.registry.get_build(build_id)
        return {
            "schema": "hoi4-re-build-registration-v1", "build_id": build_id,
            "binary_id": binary_id, "sha256": digest, "image_base": image_base,
            "exe_path": str(path), "snapshot_path": str(snapshot),
            "inventory_status": build["inventory_status"],
        }

    def _materialize_snapshot(self, source: Path | None, expected_sha256: str) -> Path:
        snapshot = self.config.state_root / "build_snapshots" / expected_sha256 / "hoi4.exe"
        snapshot.parent.mkdir(parents=True, exist_ok=True)
        if snapshot.exists():
            if snapshot.is_symlink() or not snapshot.is_file():
                raise RuntimeError("content-addressed executable snapshot is not a regular file")
            if sha256_file(snapshot) != expected_sha256:
                raise RuntimeError("content-addressed executable snapshot has the wrong SHA-256")
            return snapshot
        if source is None or not source.is_file():
            raise RuntimeError("registered executable is missing and no exact snapshot exists")
        temporary = snapshot.with_name(f"hoi4.{uuid.uuid4().hex}.tmp")
        try:
            shutil.copyfile(source, temporary)
            if sha256_file(temporary) != expected_sha256:
                raise RuntimeError("executable changed while the exact snapshot was being created")
            if snapshot.exists():
                if sha256_file(snapshot) != expected_sha256:
                    raise RuntimeError("a concurrent executable snapshot has the wrong SHA-256")
            else:
                os.replace(temporary, snapshot)
            if sha256_file(snapshot) != expected_sha256:
                raise RuntimeError("published executable snapshot has the wrong SHA-256")
            return snapshot
        finally:
            try:
                temporary.unlink()
            except FileNotFoundError:
                pass

    def _verified_exe(self, build_id: str) -> tuple[dict[str, Any], Path]:
        build = self.registry.get_build(build_id)
        source = Path(build["exe_path"]) if build.get("exe_path") else None
        return build, self._materialize_snapshot(source, build["sha256"])

    def _inspect_build(self, inputs: dict[str, Any], _job_id: str, _job_dir: Path) -> dict[str, Any]:
        build, exe = self._verified_exe(inputs["build_id"])
        result = inspect_pe(exe)
        result["build_id"] = build["build_id"]
        return result

    def _candidate_census(self, inputs: dict[str, Any], _job_id: str, job_dir: Path) -> dict[str, Any]:
        build, exe = self._verified_exe(inputs["build_id"])
        result = candidate_census(
            exe, self.config.function_index_path, job_dir / "candidate_census.json",
            self.config.ghidra_output_root,
        )
        result["build_id"] = build["build_id"]
        return result

    def _cross_analyze_function(self, inputs: dict[str, Any], _job_id: str, job_dir: Path) -> dict[str, Any]:
        build, exe = self._verified_exe(inputs["build_id"])
        if inputs.get("symbol_id"):
            instance = self.registry.resolve_symbol(inputs["symbol_id"], build["build_id"])
            rva = int(instance["rva"])
        elif "rva" in inputs:
            rva = parse_int(inputs["rva"], "rva")
        else:
            raise ValueError("cross_analyze_function requires symbol_id or rva")
        identity = inspect_pe(exe)
        if rva < 0 or rva >= int(identity["size_of_image"]):
            raise ValueError("rva is outside the registered image")
        timeout_s = int(inputs.get("timeout_s", 180))
        max_bytes = int(inputs.get("max_bytes", 4096))
        if not 10 <= timeout_s <= 900:
            raise ValueError("timeout_s must be from 10 through 900")
        if not 16 <= max_bytes <= 65536:
            raise ValueError("max_bytes must be from 16 through 65536")
        window_end = rva + max_bytes
        if not any(
            int(section["rva_start"]) <= rva
            and window_end <= int(section["rva_end"])
            for section in identity["executable_ranges"]
        ):
            raise ValueError("the requested byte window is not inside one executable section")
        result = cross_analyze(
            exe, int(build["image_base"], 16) + rva, job_dir, self.config,
            list(inputs.get("engines", ["angr", "rizin"])), timeout_s, max_bytes,
            build["sha256"],
        )
        result.update(build_id=build["build_id"], rva=hex(rva), exe_sha256=build["sha256"])
        return result

    def _ingest_ghidra_jobs(self, inputs: dict[str, Any], _job_id: str, _job_dir: Path) -> dict[str, Any]:
        if inputs.get("seed_receipts_from_job"):
            source_job_id = str(inputs["seed_receipts_from_job"])
            source_dir = ensure_relative_artifact(self.config.jobs_root, source_job_id)
            record = json.loads((source_dir / "job.json").read_text(encoding="utf-8"))
            if record.get("status") != "complete":
                raise ValueError("source ingest job is not complete")
            verified_result = json.loads((source_dir / "result.json").read_text(encoding="utf-8"))
            return seed_manifest_receipts(
                self.registry, self.config.ghidra_output_root, verified_result, source_job_id,
            )
        return ingest_ghidra_jobs(self.registry, self.config.ghidra_output_root)

    def _inventory_build(self, inputs: dict[str, Any], job_id: str, job_dir: Path) -> dict[str, Any]:
        build, exe = self._verified_exe(inputs["build_id"])
        projects = self.config.state_root / "ghidra_projects"
        projects.mkdir(parents=True, exist_ok=True)
        project_name = "build_" + build["sha256"][:12]
        project_file = projects / f"{project_name}.gpr"
        inventory = job_dir / "function_inventory.jsonl"
        if project_file.is_file():
            action = ["-process", exe.name, "-noanalysis", "-readOnly"]
        else:
            action = ["-import", str(exe), "-analysisTimeoutPerFile", str(self.config.ghidra_timeout_seconds)]
        command = [
            str(self.config.analyze_headless), str(projects), project_name, *action,
            "-scriptPath", str(self.config.ghidra_script_dir),
            "-postScript", "BuildInventory.java", str(inventory), build["sha256"],
            "-log", str(job_dir / "ghidra.log"),
        ]
        completed = subprocess.run(
            command, cwd=str(job_dir), stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT, text=True, encoding="utf-8", errors="replace",
            timeout=self.config.ghidra_timeout_seconds + 300, check=False,
            creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0),
        )
        (job_dir / "ghidra.console.log").write_text(completed.stdout, encoding="utf-8")
        if completed.returncode != 0 or not inventory.is_file():
            raise RuntimeError(f"Ghidra inventory failed with exit code {completed.returncode}")
        rows = _load_inventory(inventory, build["build_id"], job_id)
        count = self.registry.upsert_functions(rows)
        digest = sha256_file(inventory)
        self.registry.mark_inventory(build["build_id"], "full_inventory", digest)
        return {
            "schema": "hoi4-re-build-inventory-v1", "build_id": build["build_id"],
            "sha256": build["sha256"], "function_count": count,
            "inventory_sha256": digest, "project": str(project_file),
        }

    def _diff_builds(self, inputs: dict[str, Any], job_id: str, job_dir: Path) -> dict[str, Any]:
        source = self.registry.get_build(inputs["source_build"])
        target = self.registry.get_build(inputs["target_build"])
        result = diff_builds(self.registry, source["build_id"], target["build_id"], job_id)
        atomic_json(job_dir / "compatibility_pack.draft.json", result["compatibility_pack"])
        return result

    def _migrate_build(self, inputs: dict[str, Any], job_id: str, job_dir: Path) -> dict[str, Any]:
        source = self.registry.get_build(inputs["source_build"])
        registration = None
        if inputs.get("target_build"):
            target = self.registry.get_build(inputs["target_build"])
        elif inputs.get("target_exe_path"):
            registration = self._register_build({
                "exe_path": inputs["target_exe_path"],
                "binary_id": inputs.get("target_binary_id"),
                "version_label": inputs.get("version_label"),
            }, job_id, job_dir)
            target = self.registry.get_build(registration["build_id"])
        else:
            raise ValueError("migrate_build requires target_build or target_exe_path")
        if source["sha256"] == target["sha256"]:
            raise ValueError("source and target builds have the same SHA-256")

        inventory = self._inventory_build({"build_id": target["build_id"]}, job_id, job_dir)
        migration_diff = diff_builds(
            self.registry, source["build_id"], target["build_id"], job_id,
        )
        atomic_json(job_dir / "migration_diff.json", migration_diff)
        atomic_json(job_dir / "compatibility_pack.draft.json", migration_diff["compatibility_pack"])
        return {
            "schema": "hoi4-re-build-migration-v1",
            "source_build_id": source["build_id"],
            "target_build_id": target["build_id"],
            "build_id": target["build_id"],
            "registration": registration,
            "inventory": inventory,
            "coverage": migration_diff["coverage"],
            "matched_function_count": migration_diff["matched_function_count"],
            "dispositions": migration_diff["dispositions"],
            "diff": migration_diff,
            "compatibility_pack": migration_diff["compatibility_pack"],
            "promotion_allowed": False,
        }

    def _register_memory_path(self, inputs: dict[str, Any], _job_id: str, _job_dir: Path) -> dict[str, Any]:
        build = self.registry.get_build(inputs["build_id"])
        path_id = slug(inputs["path_id"], maximum=48)
        offsets = [parse_int(value, "offset") for value in inputs.get("offsets", [])]
        if len(offsets) > 16:
            raise ValueError("memory paths may contain at most 16 offsets")
        value_type = inputs["value_type"]
        if value_type not in {"u8", "i8", "u16", "i16", "u32", "i32", "u64", "i64", "f32", "f64", "ptr", "cstr", "bytes"}:
            raise ValueError("unsupported value_type")
        max_bytes = int(inputs.get("max_bytes", 64))
        if not 1 <= max_bytes <= 4096:
            raise ValueError("max_bytes must be from 1 through 4096")
        self.registry.register_memory_path(
            path_id, build["build_id"], parse_int(inputs["base_rva"], "base_rva"),
            offsets, bool(inputs.get("deref_base", True)), value_type, max_bytes,
            str(inputs.get("description", "")),
        )
        return {"schema": "hoi4-re-memory-path-v1", "path_id": path_id, "build_id": build["build_id"]}

    def _observe_runtime(self, inputs: dict[str, Any], _job_id: str, _job_dir: Path) -> dict[str, Any]:
        return observe_runtime(
            self.registry, self.config.workspace, inputs["build_id"], inputs["path_ids"],
            self.config.runtime_lease,
        )

    def _trace_calls(self, inputs: dict[str, Any], _job_id: str, job_dir: Path) -> dict[str, Any]:
        return trace_calls(
            self.registry, self.config.workspace, inputs["build_id"], inputs["symbol_ids"],
            inputs.get("duration_s", 10.0), inputs.get("max_hits", 8),
            self.config.runtime_lease, job_dir / "partial_trace.json",
            inputs.get("receiver_vtable_rvas"),
            inputs.get("receiver_qword_offsets"),
            inputs.get("argument_pointer_arrays"),
            inputs.get("receiver_pointee_qword_offsets"),
            inputs.get("receiver_pointer_chains"),
            inputs.get("skip_hits"),
            inputs.get("capture_return_address"),
        )

    def _emulate_function(self, inputs: dict[str, Any], _job_id: str, _job_dir: Path) -> dict[str, Any]:
        build, exe = self._verified_exe(inputs["build_id"])
        instance = self.registry.resolve_symbol(inputs["symbol_id"], build["build_id"])
        result = emulate_function(
            exe, int(build["image_base"], 16), int(instance["rva"]), int(instance["call_count"]),
            inputs.get("fixture", {}),
        )
        result.update(build_id=build["build_id"], symbol_id=inputs["symbol_id"], exe_sha256=build["sha256"])
        return result

    def _json_ref(self, value: Any) -> tuple[Any, dict[str, Any]]:
        if isinstance(value, dict) and "job_id" in value:
            job_dir = self.config.jobs_root / str(value["job_id"])
            path = ensure_relative_artifact(job_dir, value["path"])
        elif isinstance(value, str):
            candidate = Path(value)
            path = candidate.resolve() if candidate.is_absolute() else (self.config.workspace / candidate).resolve()
            roots = (
                self.config.workspace.resolve(), self.config.state_root.resolve(),
                self.config.save_root.resolve(),
            )
            if not any(path == root or root in path.parents for root in roots):
                raise ValueError("input is outside the workspace, RE state root, and configured save root")
        else:
            return value, {"kind": "inline"}
        if path.suffix.lower() == ".hoi4":
            return load_hoi4_save(path, self.config.rakaly_path, self.config.state_root / "temporary")
        return json.loads(path.read_text(encoding="utf-8")), {
            "kind": "json", "path": str(path), "sha256": sha256_file(path),
        }

    def _correlate_save_memory(self, inputs: dict[str, Any], _job_id: str, _job_dir: Path) -> dict[str, Any]:
        save_before, save_before_source = self._json_ref(inputs["save_before"])
        save_after, save_after_source = self._json_ref(inputs["save_after"])
        memory_before, memory_before_source = self._json_ref(inputs["memory_before"])
        memory_after, memory_after_source = self._json_ref(inputs["memory_after"])
        result = correlate(save_before, save_after, memory_before, memory_after)
        result["sources"] = {
            "save_before": save_before_source, "save_after": save_after_source,
            "memory_before": memory_before_source, "memory_after": memory_after_source,
        }
        return result

    def _record_claim(self, inputs: dict[str, Any], _job_id: str, _job_dir: Path) -> dict[str, Any]:
        if inputs.get("status") not in {"proposed", "verified", "refuted", "superseded"}:
            raise ValueError("claim status must be proposed, verified, refuted, or superseded")
        evidence = inputs.get("evidence_job_ids", [])
        if not isinstance(evidence, list):
            raise ValueError("evidence_job_ids must be a list")
        claim_id = self.registry.record_claim(inputs)
        return {"schema": "hoi4-re-claim-v1", "claim_id": claim_id, "status": inputs["status"]}

    def query(self, kind: str, arguments: dict[str, Any]) -> dict[str, Any]:
        return {"kind": kind, "rows": self.registry.query(kind, arguments)}

    def list_jobs(self, limit: int = 50) -> dict[str, Any]:
        return self.query("jobs", {"limit": limit})

    def read_artifact(self, job_id: str, relative: str, max_chars: int = 200_000) -> dict[str, Any]:
        job_dir = self.config.jobs_root / job_id
        path = ensure_relative_artifact(job_dir, relative)
        if not path.is_file():
            raise FileNotFoundError(relative)
        if not isinstance(max_chars, int) or not 1 <= max_chars <= 2_000_000:
            raise ValueError("max_chars must be from 1 through 2000000")
        text = path.read_text(encoding="utf-8", errors="replace")
        return {
            "job_id": job_id, "path": relative, "sha256": sha256_file(path),
            "bytes": path.stat().st_size, "truncated": len(text) > max_chars,
            "text": text[:max_chars],
        }

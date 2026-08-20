from __future__ import annotations

import contextlib
import json
import msvcrt
import sqlite3
import struct
import subprocess
import time
from pathlib import Path
from typing import Any, Callable, Iterator

from common import atomic_json, sha256_file


OFFLINE_SLOTS = 4
MACHINE_LOCK_ROOT = Path(r"C:\hoi4_evidence\agent_re\machine-locks")


def _probe(command: list[str], timeout: int = 15) -> dict[str, Any]:
    try:
        completed = subprocess.run(
            command, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
            text=True, encoding="utf-8", errors="replace", timeout=timeout, check=False,
            creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0),
        )
        return {
            "available": completed.returncode == 0,
            "exit_code": completed.returncode,
            "version": completed.stdout.strip().splitlines()[-1][:300] if completed.stdout.strip() else None,
        }
    except Exception as error:
        return {"available": False, "error": f"{type(error).__name__}: {error}"}


def _file_status(path: Path) -> dict[str, Any]:
    try:
        available = path.is_file()
        return {"available": available, "path": str(path), **({} if available else {"error": "missing"})}
    except OSError as error:
        return {"available": False, "path": str(path), "error": f"{type(error).__name__}: {error}"}


def toolchain_status(config: Any) -> dict[str, Any]:
    angr_file = _file_status(config.angr_python)
    rizin_file = _file_status(config.rizin_path)
    checks = {
        "angr": _probe([str(config.angr_python), "-c", "import angr; print(angr.__version__)"])
        if angr_file["available"] else angr_file,
        "rizin": _probe([str(config.rizin_path), "-v"])
        if rizin_file["available"] else rizin_file,
        "dynamorio": _file_status(config.drrun_path),
        "drcov": _file_status(config.drcov_path),
        "windbg_ttd": _file_status(config.windbg_ttd_path),
    }
    result = {
        "ready": all(value["available"] for value in checks.values()),
        "offline_parallel_slots": OFFLINE_SLOTS,
        "offline_lock_root": str(MACHINE_LOCK_ROOT),
        "live_parallel_slots": 1,
        "checks": checks,
    }
    return result


@contextlib.contextmanager
def offline_slot(lock_root: Path, slots: int, timeout_s: float = 600.0) -> Iterator[int]:
    lock_root.mkdir(parents=True, exist_ok=True)
    deadline = time.monotonic() + timeout_s
    try:
        while time.monotonic() < deadline:
            for slot in range(max(1, slots)):
                handle = (lock_root / f"offline-{slot + 1:02d}.lock").open("a+b")
                if handle.tell() == 0:
                    handle.write(b"0")
                    handle.flush()
                handle.seek(0)
                try:
                    msvcrt.locking(handle.fileno(), msvcrt.LK_NBLCK, 1)
                    yield slot + 1
                    return
                except OSError:
                    handle.close()
            time.sleep(0.1)
        raise TimeoutError(f"all {slots} offline analysis slots stayed busy for {timeout_s:g}s")
    finally:
        if "handle" in locals() and not handle.closed:
            try:
                handle.seek(0)
                msvcrt.locking(handle.fileno(), msvcrt.LK_UNLCK, 1)
                handle.close()
            except OSError:
                pass


def inspect_pe(path: Path, include_pdata: bool = False) -> dict[str, Any]:
    import pefile

    pe = pefile.PE(str(path), fast_load=False)
    image_base = int(pe.OPTIONAL_HEADER.ImageBase)
    executable_ranges = []
    for section in pe.sections:
        if int(section.Characteristics) & 0x20000000:
            start = int(section.VirtualAddress)
            size = max(int(section.Misc_VirtualSize), int(section.SizeOfRawData))
            executable_ranges.append({
                "name": section.Name.rstrip(b"\0").decode("ascii", errors="replace"),
                "rva_start": start, "rva_end": start + size,
            })
    pdata = []
    for entry in getattr(pe, "DIRECTORY_ENTRY_EXCEPTION", []):
        begin = int(entry.struct.BeginAddress)
        if begin:
            pdata.append(begin)

    pdb = None
    for entry in getattr(pe, "DIRECTORY_ENTRY_DEBUG", []):
        pointer = int(entry.struct.PointerToRawData)
        size = int(entry.struct.SizeOfData)
        data = pe.__data__[pointer:pointer + size]
        if data.startswith(b"RSDS") and len(data) >= 25:
            raw = data[4:20]
            data1, data2, data3 = struct.unpack("<IHH", raw[:8])
            data4 = raw[8:]
            guid = (
                f"{data1:08x}-{data2:04x}-{data3:04x}-"
                f"{data4[:2].hex()}-{data4[2:].hex()}"
            )
            age = struct.unpack("<I", data[20:24])[0]
            name = data[24:].split(b"\0", 1)[0].decode("utf-8", errors="replace")
            pdb = {
                "guid": guid,
                "age": age,
                "symbol_key": guid.replace("-", "").upper() + str(age),
                "embedded_path": name,
            }
            break
    result = {
        "schema": "hoi4-re-pe-identity-v1",
        "path": str(path),
        "sha256": sha256_file(path),
        "image_base": hex(image_base),
        "size_of_image": int(pe.OPTIONAL_HEADER.SizeOfImage),
        "executable_ranges": executable_ranges,
        "pdata_function_count": len(set(pdata)),
        "pdb": pdb,
    }
    if include_pdata:
        result["pdata_rvas"] = sorted(set(pdata))
    pe.close()
    return result


def _validate_index_certificate(
    meta: dict[str, str], executable_sha256: str, image_base: str,
    expected_store_root: Path | None,
) -> None:
    required = {
        "schema_version": "3",
        "generation_complete": "1",
        "full_store_scan_complete": "1",
        "executable_sha256": executable_sha256.lower(),
        "image_base": image_base.lower(),
    }
    mismatched = {
        key: {"expected": value, "actual": meta.get(key)}
        for key, value in required.items() if meta.get(key, "").lower() != value
    }
    if mismatched or not meta.get("generation_id"):
        raise ValueError(
            "function index is unpublished or has the wrong executable identity: "
            + json.dumps(mismatched or {"generation_id": "missing"}, sort_keys=True)
        )
    if expected_store_root is not None:
        store = meta.get("store_root")
        if not store or Path(store).resolve() != expected_store_root.resolve():
            raise ValueError("function index belongs to a different Ghidra export store")


def _read_function_index_snapshot(
    function_index: Path, executable_sha256: str, image_base: str,
    expected_store_root: Path | None, after_metadata: Callable[[], None] | None = None,
) -> tuple[dict[str, str], set[int]]:
    if not function_index.is_file():
        raise FileNotFoundError(f"published function index is missing: {function_index}")
    uri = function_index.resolve().as_uri() + "?mode=ro"
    with contextlib.closing(sqlite3.connect(uri, uri=True, timeout=30.0)) as connection:
        connection.execute("PRAGMA query_only=ON")
        connection.execute("BEGIN")
        meta = {str(row[0]): str(row[1]) for row in connection.execute("SELECT key,value FROM meta")}
        _validate_index_certificate(meta, executable_sha256, image_base, expected_store_root)
        if after_metadata is not None:
            after_metadata()
        addresses = {int(row[0]) for row in connection.execute("SELECT address FROM functions")}
    return meta, addresses


def published_index_status(
    function_index: Path, expected_store_root: Path, builds: list[dict[str, Any]],
) -> dict[str, Any]:
    try:
        if not function_index.is_file():
            raise FileNotFoundError(f"published function index is missing: {function_index}")
        uri = function_index.resolve().as_uri() + "?mode=ro"
        with contextlib.closing(sqlite3.connect(uri, uri=True, timeout=30.0)) as connection:
            connection.execute("PRAGMA query_only=ON")
            connection.execute("BEGIN")
            meta = {
                str(row[0]): str(row[1])
                for row in connection.execute("SELECT key,value FROM meta")
            }
        matching = [row for row in builds if row.get("sha256") == meta.get("executable_sha256")]
        if len(matching) != 1:
            raise ValueError("function index identity does not match one registered build")
        _validate_index_certificate(
            meta, matching[0]["sha256"], matching[0]["image_base"], expected_store_root,
        )
        return {
            "available": True, "path": str(function_index),
            "generation_id": meta["generation_id"],
            "executable_sha256": meta["executable_sha256"],
            "image_base": meta["image_base"], "store_root": meta["store_root"],
        }
    except Exception as error:
        return {
            "available": False, "path": str(function_index),
            "error": f"{type(error).__name__}: {error}",
        }


def candidate_census(
    exe: Path, function_index: Path, output: Path, expected_store_root: Path | None = None,
) -> dict[str, Any]:
    identity = inspect_pe(exe, include_pdata=True)
    image_base = int(identity["image_base"], 0)
    meta, addresses = _read_function_index_snapshot(
        function_index, identity["sha256"], identity["image_base"], expected_store_root,
    )
    indexed = {address - image_base for address in addresses}
    pdata = set(identity.pop("pdata_rvas"))
    missing = sorted(pdata - indexed)
    candidates_path = output.with_name("missing_candidate_rvas.txt")
    candidates_path.write_text(
        "".join(f"0x{rva:x}\n" for rva in missing), encoding="ascii", newline="\n"
    )
    artifact = {
        "schema": "hoi4-re-candidate-census-v1",
        "identity": identity,
        "function_index": str(function_index),
        "function_index_generation": meta["generation_id"],
        "function_index_schema": int(meta["schema_version"]),
        "function_index_executable_sha256": meta["executable_sha256"],
        "function_index_image_base": meta["image_base"],
        "function_index_store_root": meta.get("store_root"),
        "indexed_function_count": len(indexed),
        "pdata_function_count": len(pdata),
        "pdata_index_overlap": len(pdata & indexed),
        "pdata_not_indexed_count": len(missing),
        "missing_candidate_rvas_artifact": candidates_path.name,
        "interpretation": "Candidate accounting only; coverage is not proof of semantic reconstruction.",
    }
    atomic_json(output, artifact)
    return artifact | {"census_artifact": output.name}


def _decoded(value: str | bytes | None) -> str:
    if value is None:
        return ""
    return value.decode("utf-8", errors="replace") if isinstance(value, bytes) else value


def _run_command(command: list[str], output: Path, timeout_s: int) -> dict[str, Any]:
    timed_out = False
    try:
        completed = subprocess.run(
            command, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
            text=True, encoding="utf-8", errors="replace", timeout=timeout_s, check=False,
            creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0),
        )
        stdout, stderr, exit_code = completed.stdout, completed.stderr, completed.returncode
    except subprocess.TimeoutExpired as error:
        timed_out = True
        stdout, stderr, exit_code = _decoded(error.stdout), _decoded(error.stderr), None
        stderr += f"\nTimeoutExpired: exceeded {timeout_s}s\n"
    output.write_text(stdout, encoding="utf-8", newline="\n")
    stderr_path = output.with_suffix(output.suffix + ".stderr.log")
    if stderr:
        stderr_path.write_text(stderr, encoding="utf-8", newline="\n")
    return {
        "status": "timed_out" if timed_out else "complete" if exit_code == 0 else "failed",
        "exit_code": exit_code, "output": output.name, "bytes": output.stat().st_size,
        "stderr": stderr_path.name if stderr else None,
    }


def _load_json(path: Path, label: str) -> Any:
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except (OSError, ValueError) as error:
        raise RuntimeError(f"{label} did not produce valid JSON: {error}") from error


def _require_success(result: dict[str, Any], label: str) -> None:
    if result["status"] != "complete":
        raise RuntimeError(f"{label} {result['status']} (exit_code={result['exit_code']})")


def _require_snapshot(exe: Path, expected_sha256: str, label: str) -> None:
    if not exe.is_file() or sha256_file(exe) != expected_sha256:
        raise RuntimeError(f"content-addressed executable snapshot changed during {label}")


def cross_analyze(
    exe: Path, address: int, job_dir: Path, config: Any, engines: list[str],
    timeout_s: int, max_bytes: int, expected_sha256: str,
) -> dict[str, Any]:
    unknown = sorted(set(engines) - {"angr", "rizin"})
    if unknown:
        raise ValueError(f"unsupported engines: {unknown}")
    if not engines:
        raise ValueError("engines must contain angr and/or rizin")
    results: dict[str, Any] = {}
    failure: Exception | None = None
    with offline_slot(MACHINE_LOCK_ROOT, OFFLINE_SLOTS, timeout_s + 30) as slot:
        try:
            _require_snapshot(exe, expected_sha256, "analysis start")
            if "angr" in engines:
                output = job_dir / "angr.json"
                results["angr"] = _run_command([
                    str(config.angr_python), str(Path(__file__).with_name("angr_worker.py")),
                    "--exe", str(exe), "--address", hex(address), "--max-bytes", str(max_bytes),
                    "--output", str(output),
                ], job_dir / "angr.console.log", timeout_s)
                _require_success(results["angr"], "angr")
                angr_result = _load_json(output, "angr")
                if angr_result.get("status") != "complete":
                    raise RuntimeError(f"angr worker failed: {angr_result.get('error', 'unknown error')}")
                results["angr"]["result"] = output.name
                _require_snapshot(exe, expected_sha256, "angr")
            if "rizin" in engines:
                upper = address + max_bytes
                common = [
                    str(config.rizin_path), "-q", "-e", "io.cache=true",
                    "-c", f"s {address:#x}", "-c", "af",
                ]
                results["rizin_disassembly"] = _run_command(
                    [*common, "-c", f"pDj {max_bytes}", str(exe)],
                    job_dir / "rizin_disassembly.json", timeout_s,
                )
                _require_success(results["rizin_disassembly"], "Rizin disassembly")
                operations = _load_json(job_dir / "rizin_disassembly.json", "Rizin disassembly")
                if not isinstance(operations, list) or not operations or any(
                    int(row.get("size", 0)) <= 0
                    or not address <= int(row.get("offset", -1))
                    or int(row.get("offset", -1)) + int(row.get("size", 0)) > upper
                    for row in operations
                ):
                    raise RuntimeError("Rizin disassembly escaped the requested byte range")
                if not any(int(row.get("offset", -1)) == address for row in operations):
                    raise RuntimeError("Rizin disassembly did not recover the requested entry")
                _require_snapshot(exe, expected_sha256, "Rizin disassembly")
                results["rizin_cfg"] = _run_command(
                    [*common, "-c", "afbj", str(exe)], job_dir / "rizin_cfg.json", timeout_s,
                )
                _require_success(results["rizin_cfg"], "Rizin CFG")
                blocks = _load_json(job_dir / "rizin_cfg.json", "Rizin CFG")
                if not isinstance(blocks, list) or not blocks or any(
                    int(row.get("size", 0)) <= 0
                    or not address <= int(row.get("addr", -1))
                    or int(row.get("addr", -1)) + int(row.get("size", 0)) > upper
                    for row in blocks
                ):
                    raise RuntimeError("Rizin CFG escaped the requested byte range")
                if not any(int(row.get("addr", -1)) == address for row in blocks):
                    raise RuntimeError("Rizin CFG did not recover an entry block")
                _require_snapshot(exe, expected_sha256, "Rizin CFG")
        except Exception as error:
            failure = error
    summary = {
        "schema": "hoi4-re-cross-analysis-v1", "address": hex(address),
        "status": "failed" if failure else "complete",
        "offline_slot": slot, "engines": engines, "max_bytes": max_bytes, "results": results,
        "promotion_allowed": False,
    }
    if failure is not None:
        summary["error"] = f"{type(failure).__name__}: {failure}"
        atomic_json(job_dir / "cross_analysis.partial.json", summary)
        raise RuntimeError(summary["error"])
    return summary

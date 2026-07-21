from __future__ import annotations

import json
import os
import time
from pathlib import Path
from typing import Any

from common import atomic_json
from registry import Registry


def _load_winmem(workspace: Path):
    import sys

    memory_reader = workspace / "extractor" / "memory_reader"
    if str(memory_reader) not in sys.path:
        sys.path.insert(0, str(memory_reader))
    import winmem

    return winmem


class RuntimeLease:
    def __init__(self, path: Path):
        self.path = path
        self.handle = None

    def __enter__(self):
        import msvcrt

        self.path.parent.mkdir(parents=True, exist_ok=True)
        self.handle = self.path.open("a+b")
        self.handle.seek(0, os.SEEK_END)
        if self.handle.tell() == 0:
            self.handle.write(b"0")
            self.handle.flush()
        self.handle.seek(0)
        try:
            msvcrt.locking(self.handle.fileno(), msvcrt.LK_NBLCK, 1)
        except OSError as error:
            self.handle.close()
            self.handle = None
            raise RuntimeError("the live the game process is already leased by another RE job") from error
        return self

    def __exit__(self, *_: object) -> None:
        import msvcrt

        if self.handle is not None:
            self.handle.seek(0)
            try:
                msvcrt.locking(self.handle.fileno(), msvcrt.LK_UNLCK, 1)
            finally:
                self.handle.close()
                self.handle = None


def runtime_status(workspace: Path) -> dict[str, Any]:
    winmem = _load_winmem(workspace)
    pid = winmem.find_pid("hoi4.exe")
    if pid is None:
        return {"running": False, "process": "hoi4.exe"}
    with winmem.Process(pid, "hoi4.exe") as process:
        fingerprint = process.fingerprint()
    fingerprint["base"] = f"0x{int(fingerprint['base']):x}"
    fingerprint["size"] = f"0x{int(fingerprint['size']):x}"
    return {"running": True, "process": fingerprint}


def _read_value(process: Any, address: int, value_type: str, max_bytes: int) -> Any:
    readers = {
        "u8": process.u8, "i8": process.i8, "u16": process.u16, "i16": process.i16,
        "u32": process.u32, "i32": process.i32, "u64": process.u64, "i64": process.i64,
        "f32": process.f32, "f64": process.f64, "ptr": process.ptr,
    }
    if value_type in readers:
        return readers[value_type](address)
    if value_type == "cstr":
        return process.cstr(address, max_bytes)
    if value_type == "bytes":
        return process.read(address, max_bytes).hex()
    raise ValueError(f"unsupported memory path value_type: {value_type}")


def observe_runtime(
    registry: Registry, workspace: Path, build_identifier: str,
    path_ids: list[str], lease_path: Path,
) -> dict[str, Any]:
    if not isinstance(path_ids, list) or not path_ids or len(path_ids) > 64:
        raise ValueError("path_ids must contain 1..64 registered path IDs")
    build = registry.get_build(build_identifier)
    winmem = _load_winmem(workspace)
    with RuntimeLease(lease_path):
        process = winmem.open_process("hoi4.exe", "hoi4.exe")
        if process is None:
            raise RuntimeError("hoi4.exe is not running")
        with process:
            fingerprint = process.fingerprint()
            if str(fingerprint.get("sha256", "")).lower() != build["sha256"]:
                raise RuntimeError(
                    f"live executable SHA-256 {fingerprint.get('sha256')} does not match build {build['sha256']}"
                )
            observations = []
            for path_id in path_ids:
                spec = registry.memory_path(path_id)
                if spec["build_id"] != build["build_id"]:
                    raise RuntimeError(f"memory path {path_id} belongs to a different build")
                address = process.module_base + int(spec["base_rva"])
                offsets = [int(value) for value in spec["offsets"]]
                if offsets:
                    address = process.chain(address, offsets, deref_base=spec["deref_base"])
                value = _read_value(process, address, spec["value_type"], int(spec["max_bytes"]))
                observations.append({
                    "path_id": path_id,
                    "value_type": spec["value_type"],
                    "value": value,
                    "resolved_address": f"0x{address:x}",
                    "resolved_module_rva": f"0x{address - process.module_base:x}" if process.in_module(address) else None,
                })
            return {
                "schema": "hoi4-re-runtime-observation-v1",
                "mode": "read-only ReadProcessMemory",
                "build_id": build["build_id"],
                "exe_sha256": fingerprint["sha256"],
                "pid": process.pid,
                "module_base": f"0x{process.module_base:x}",
                "observations": observations,
            }


def _trace_script(targets: list[dict[str, Any]], max_hits: int) -> str:
    return f"""
'use strict';
const module = Process.getModuleByName('hoi4.exe');
const targets = {json.dumps(targets)};
const maxHits = {max_hits};
const counts = {{}};
function hx(value) {{ try {{ return value.toString(); }} catch (_) {{ return null; }} }}
for (const target of targets) {{
  const address = module.base.add(ptr(target.rva));
  counts[target.symbol_id] = 0;
  Interceptor.attach(address, {{
    onEnter(args) {{
      const seen = counts[target.symbol_id];
      if (seen >= maxHits) {{ this.capture = false; return; }}
      counts[target.symbol_id] = seen + 1;
      this.capture = true;
      this.hit = seen + 1;
      send({{
        type: 'enter', symbol_id: target.symbol_id, rva: target.rva,
        hit: this.hit, thread_id: this.threadId,
        registers: {{ rip: hx(this.context.rip), rsp: hx(this.context.rsp),
          rcx: hx(this.context.rcx), rdx: hx(this.context.rdx),
          r8: hx(this.context.r8), r9: hx(this.context.r9) }}
      }});
    }},
    onLeave(retval) {{
      if (!this.capture) return;
      send({{type: 'leave', symbol_id: target.symbol_id, rva: target.rva,
        hit: this.hit, thread_id: this.threadId, retval: hx(retval)}});
    }}
  }});
}}
send({{type: 'ready', targets: targets.length}});
"""


def trace_calls(
    registry: Registry, workspace: Path, build_identifier: str,
    symbol_ids: list[str], duration_s: float, max_hits: int,
    lease_path: Path, partial_path: Path,
) -> dict[str, Any]:
    if not isinstance(symbol_ids, list) or not symbol_ids or len(symbol_ids) > 32:
        raise ValueError("symbol_ids must contain 1..32 logical symbol IDs")
    if not isinstance(duration_s, (int, float)) or not 0.1 <= float(duration_s) <= 30.0:
        raise ValueError("duration_s must be from 0.1 through 30.0")
    if not isinstance(max_hits, int) or not 1 <= max_hits <= 64:
        raise ValueError("max_hits must be from 1 through 64")
    import frida

    build = registry.get_build(build_identifier)
    targets = []
    for symbol_id in symbol_ids:
        instance = registry.resolve_symbol(symbol_id, build["build_id"])
        targets.append({"symbol_id": symbol_id, "rva": f"0x{int(instance['rva']):x}"})
    winmem = _load_winmem(workspace)
    with RuntimeLease(lease_path):
        process = winmem.open_process("hoi4.exe", "hoi4.exe")
        if process is None:
            raise RuntimeError("hoi4.exe is not running")
        with process:
            fingerprint = process.fingerprint()
            pid = process.pid
        if str(fingerprint.get("sha256", "")).lower() != build["sha256"]:
            raise RuntimeError("live executable does not match the requested build; no force override is available")
        messages: list[dict[str, Any]] = []
        errors: list[dict[str, Any]] = []

        def on_message(message: dict[str, Any], data: bytes | None) -> None:
            if message.get("type") == "send" and isinstance(message.get("payload"), dict):
                messages.append(message["payload"])
            else:
                errors.append({"message": message, "data_bytes": 0 if data is None else len(data)})

        session = frida.attach(pid)
        script = None
        cleanup_errors: list[str] = []
        try:
            script = session.create_script(_trace_script(targets, max_hits))
            script.on("message", on_message)
            script.load()
            time.sleep(float(duration_s))
        finally:
            if script is not None:
                try:
                    script.unload()
                except Exception as error:
                    cleanup_errors.append(f"script.unload: {type(error).__name__}: {error}")
            try:
                session.detach()
            except Exception as error:
                cleanup_errors.append(f"session.detach: {type(error).__name__}: {error}")
        result = {
            "schema": "hoi4-re-frida-call-trace-v1",
            "mode": "hook-based observational instrumentation",
            "build_id": build["build_id"],
            "exe_sha256": build["sha256"],
            "pid": pid,
            "duration_s": float(duration_s),
            "max_hits": max_hits,
            "targets": targets,
            "messages": messages,
            "errors": errors,
            "cleanup_errors": cleanup_errors,
            "promotion_allowed": False,
        }
        atomic_json(partial_path, result)
        if cleanup_errors:
            raise RuntimeError("Frida cleanup failed; the runtime session is tainted: " + "; ".join(cleanup_errors))
        return result

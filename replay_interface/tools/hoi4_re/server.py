#!/usr/bin/env python3
from __future__ import annotations

import json
import sys
import traceback
from pathlib import Path
from typing import Any

SCRIPT_DIR = Path(__file__).resolve().parent
if str(SCRIPT_DIR) not in sys.path:
    sys.path.insert(0, str(SCRIPT_DIR))

from operations import Broker, Config


TOOLS = [
    {
        "name": "re_status",
        "description": "Check the unified the game RE registry, offline engines, Ghidra inventory path, and optional live process. Run first.",
        "inputSchema": {"type": "object", "properties": {}, "additionalProperties": False},
    },
    {
        "name": "re_query",
        "description": "Query builds, symbols, matches, claims, memory paths, jobs, or coverage from the shared evidence registry.",
        "inputSchema": {
            "type": "object", "required": ["kind"],
            "properties": {
                "kind": {"type": "string", "enum": ["builds", "symbols", "matches", "claims", "memory_paths", "jobs", "coverage"]},
                "name": {"type": "string"}, "build_id": {"type": "string"},
                "limit": {"type": "integer", "minimum": 1, "maximum": 500},
            }, "additionalProperties": False,
        },
    },
    {
        "name": "re_run",
        "description": "Run one durable RE job. Operations cover build registration/inventory/diff, Ghidra evidence ingestion, registered live reads, bounded call tracing, leaf emulation, save correlation, and claims.",
        "inputSchema": {
            "type": "object", "required": ["operation", "inputs"],
            "properties": {
                "operation": {"type": "string", "enum": [
                    "register_build", "ingest_ghidra_jobs", "inventory_build", "diff_builds", "migrate_build",
                    "register_memory_path", "observe_runtime", "trace_calls", "emulate_function",
                    "correlate_save_memory", "record_claim",
                ]},
                "inputs": {"type": "object"},
            }, "additionalProperties": False,
        },
    },
    {
        "name": "re_list_jobs",
        "description": "List recent durable unified-RE jobs and their status.",
        "inputSchema": {
            "type": "object", "properties": {"limit": {"type": "integer", "minimum": 1, "maximum": 500}},
            "additionalProperties": False,
        },
    },
    {
        "name": "re_read_artifact",
        "description": "Read a UTF-8 artifact from a unified-RE job without allowing path traversal.",
        "inputSchema": {
            "type": "object", "required": ["job_id", "path"],
            "properties": {
                "job_id": {"type": "string"}, "path": {"type": "string"},
                "max_chars": {"type": "integer", "minimum": 1, "maximum": 2000000},
            }, "additionalProperties": False,
        },
    },
]


def tool_result(value: Any, is_error: bool = False) -> dict[str, Any]:
    return {
        "content": [{"type": "text", "text": json.dumps(value, indent=2, sort_keys=True)}],
        "isError": is_error,
    }


def dispatch(broker: Broker, method: str, params: dict[str, Any]) -> dict[str, Any]:
    if method == "initialize":
        return {
            "protocolVersion": params.get("protocolVersion", "2024-11-05"),
            "capabilities": {"tools": {"listChanged": False}},
            "serverInfo": {"name": "hoi4-re", "version": "0.1.0"},
            "instructions": (
                "Run re_status first. Use build IDs, symbol IDs, path IDs, and job IDs instead of raw paths or addresses. "
                "The existing exact-a729 Ghidra broker remains the static authority. New-build analysis is isolated. "
                "Live reads are SHA-gated; Frida calls are hook-based, exclusive, bounded, and never imply promotion."
            ),
        }
    if method == "ping":
        return {}
    if method == "tools/list":
        return {"tools": TOOLS}
    if method == "tools/call":
        name = params.get("name")
        arguments = params.get("arguments") or {}
        try:
            if name == "re_status":
                value = broker.status()
            elif name == "re_query":
                value = broker.query(arguments["kind"], arguments)
            elif name == "re_run":
                value = broker.run(arguments["operation"], arguments["inputs"])
            elif name == "re_list_jobs":
                value = broker.list_jobs(arguments.get("limit", 50))
            elif name == "re_read_artifact":
                value = broker.read_artifact(
                    arguments["job_id"], arguments["path"], arguments.get("max_chars", 200_000)
                )
            else:
                return tool_result({"error": f"unknown tool: {name}"}, True)
            return tool_result(value)
        except Exception as error:
            return tool_result({"error": f"{type(error).__name__}: {error}"}, True)
    raise KeyError(method)


def main() -> int:
    try:
        broker = Broker(Config.from_env())
    except Exception as error:
        print(f"hoi4-re configuration error: {error}", file=sys.stderr, flush=True)
        return 2
    for raw in sys.stdin:
        try:
            request = json.loads(raw)
            if not isinstance(request, dict):
                raise ValueError("request must be an object")
            request_id = request.get("id")
            method = request.get("method")
            if method and method.startswith("notifications/"):
                continue
            if request_id is None:
                continue
            try:
                result = dispatch(broker, method, request.get("params") or {})
                response = {"jsonrpc": "2.0", "id": request_id, "result": result}
            except KeyError:
                response = {"jsonrpc": "2.0", "id": request_id, "error": {"code": -32601, "message": f"method not found: {method}"}}
            print(json.dumps(response, separators=(",", ":")), flush=True)
        except Exception as error:
            print(json.dumps({"jsonrpc": "2.0", "id": None, "error": {"code": -32603, "message": str(error)}}), flush=True)
            traceback.print_exc(file=sys.stderr)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

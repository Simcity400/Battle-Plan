#!/usr/bin/env python3
"""Isolated angr worker used by the stdlib-based hoi4_re MCP server."""

from __future__ import annotations

import argparse
import json
import traceback
from pathlib import Path


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--exe", required=True)
    parser.add_argument("--address", required=True)
    parser.add_argument("--max-bytes", type=int, default=4096)
    parser.add_argument("--output", required=True)
    args = parser.parse_args()

    import angr

    address = int(args.address, 0)
    maximum = max(16, min(args.max_bytes, 65536))
    result: dict[str, object] = {
        "schema": "hoi4-re-angr-function-v1",
        "engine": "angr",
        "engine_version": angr.__version__,
        "address": hex(address),
        "max_bytes": maximum,
    }
    try:
        project = angr.Project(args.exe, auto_load_libs=False)
        region = (address, address + maximum)
        cfg = project.analyses.CFGFast(
            function_starts=[address],
            regions=[region],
            normalize=True,
            data_references=False,
            force_complete_scan=False,
        )
        function = cfg.kb.functions.get(address)
        if function is None:
            raise RuntimeError(f"angr did not recover a function at {address:#x}")
        decompiler = project.analyses.Decompiler(function, cfg=cfg.model)
        code = None
        if decompiler.codegen is not None:
            code = decompiler.codegen.text
        result.update({
            "status": "complete",
            "name": function.name,
            "blocks": len(function.block_addrs_set),
            "size": int(function.size),
            "returning": function.returning,
            "decompilation": code,
        })
    except Exception as error:
        result.update({
            "status": "failed",
            "error": f"{type(error).__name__}: {error}",
            "traceback": traceback.format_exc(),
        })
    Path(args.output).write_text(
        json.dumps(result, indent=2, sort_keys=True) + "\n", encoding="utf-8", newline="\n"
    )
    return 0 if result["status"] == "complete" else 2


if __name__ == "__main__":
    raise SystemExit(main())

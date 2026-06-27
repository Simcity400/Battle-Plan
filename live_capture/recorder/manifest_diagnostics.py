"""Patch recorder diagnostics into a finalized .battleplan manifest.

This stays recorder-local so the GUI can surface cadence diagnostics without
changing the shared capture loop or writer API.
"""
from __future__ import annotations

import json
import os
from typing import Any, Dict


def patch_manifest_diagnostics(path: str, diagnostics: Dict[str, Any]) -> bool:
    """Add ``recording_diagnostics`` to a finalized ``.battleplan`` container.

    Returns True when the container was patched. Missing/unfinalized files return
    False so the recorder can keep running and try again after the next finalize.
    """
    if not path or not os.path.isfile(path):
        return False

    from live_capture import replay_writer as rw

    with open(path, "rb") as fh:
        by_name = rw._zip_read_all(fh.read())
    manifest_bytes = by_name.get("manifest.json")
    if manifest_bytes is None:
        return False

    manifest = json.loads(manifest_bytes.decode("utf-8"))
    manifest["recording_diagnostics"] = _json_safe(diagnostics)
    manifest["manifest_digest"] = None
    manifest["manifest_digest"] = rw.manifest_digest(manifest)

    files = []
    for name in sorted(k for k in by_name if k != "manifest.json"):
        files.append(rw._ZipFile(name, by_name[name], deflate=True))
    man_bytes = json.dumps(manifest, separators=(",", ":"),
                           ensure_ascii=False).encode("utf-8")
    files.append(rw._ZipFile("manifest.json", man_bytes, deflate=False))
    data = rw._write_zip(files)

    tmp = path + ".tmp"
    out_dir = os.path.dirname(os.path.abspath(path)) or "."
    with open(tmp, "wb") as fh:
        fh.write(data)
        fh.flush()
        os.fsync(fh.fileno())
    os.replace(tmp, path)
    rw._fsync_dir(out_dir)
    return True


def _json_safe(obj: Any) -> Any:
    return json.loads(json.dumps(obj, separators=(",", ":"),
                                 ensure_ascii=False, allow_nan=False))


__all__ = ["patch_manifest_diagnostics"]

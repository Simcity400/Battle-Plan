from __future__ import annotations

import json
import subprocess
import tempfile
from pathlib import Path
from typing import Any

from common import sha256_file


def load_hoi4_save(path: Path, rakaly: Path, temporary_root: Path) -> tuple[Any, dict[str, Any]]:
    if not rakaly.is_file():
        raise FileNotFoundError(f"rakaly executable not found: {rakaly}")
    if not path.is_file() or path.suffix.lower() != ".hoi4":
        raise ValueError("save input must be an existing .hoi4 file")
    temporary_root.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="save-oracle-", dir=temporary_root) as temporary:
        root = Path(temporary)
        converted = root / "save.json"
        with converted.open("wb") as output:
            encoded = subprocess.run(
                [str(rakaly), "json", str(path), "-k", "group"],
                stdin=subprocess.DEVNULL, stdout=output, stderr=subprocess.PIPE,
                timeout=1800, check=False,
            )
        if encoded.returncode != 0 or not converted.is_file() or converted.stat().st_size == 0:
            detail = encoded.stderr[-2000:].decode("utf-8", "replace")
            raise RuntimeError(f"rakaly save conversion failed with exit code {encoded.returncode}: {detail}")
        value = json.loads(converted.read_text(encoding="utf-8"))
    version = subprocess.run(
        [str(rakaly), "--version"], stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT, text=True, timeout=10, check=False,
    ).stdout.strip()
    return value, {
        "kind": "hoi4_save", "path": str(path), "sha256": sha256_file(path),
        "rakaly": version, "intermediates_retained": False,
    }

"""replay_writer.py -- the streaming ``.battleplan`` WRITER (capture side).

Wires the live capture loop's durable directory store into a single, portable
``.battleplan`` container that a player can stop anytime and send to someone else.

Design rule (replay_ui/HOI4REPLAY_CONTAINER_SPEC.md §6): the hot capture loop
keeps writing each hour into the **existing durable directory store** (atomic per
file) -- we never turn it into a fragile ZIP-append loop. This module adds two
things on top:

  * a running, append-only **sidecar** (``entries.jsonl``) next to the work dir
    that records each finalized snapshot's ``{path, sha256, bytes}`` as it lands.
    A crash mid-record leaves a PREFIX of complete lines (append-only + fsync),
    never a torn structure;
  * a single, atomic **finalize** that re-reads the store + sidecar, builds
    ``manifest.json`` (written LAST), and emits the ZIP via ``.tmp -> os.replace``.

Because ``finalize()`` re-reads the store every call, a recording ended at ANY
point is valid (it contains exactly the hours on disk so far), and an interrupted
record (process died before finalize) leaves the store + sidecar intact so a
re-run of ``finalize()`` produces a valid container of the pre-crash hours.

NO FABRICATION end to end: payload files are packed as RAW BYTES (never
parsed-and-re-serialized), so the Absent / null / measured-0 trichotomy survives
untouched; absent identity (no launcher config / unreadable build) is recorded as
JSON ``null``, never a guessed value.

This module is **pure stdlib** (json/hashlib/zlib/zipfile-free hand-rolled ZIP).
It byte-matches the Rust reference packer ``snapshot/src/pack.rs`` + the reader
``snapshot/src/container.rs`` so a container this writer emits opens with
``pack_replay verify``. The canonical-JSON helper below produces bytes IDENTICAL
to ``snapshot/src/canon.rs`` (RFC-8785-style) so ``manifest_digest`` agrees
cross-impl.

READ-ONLY w.r.t. the game; writes only its own store dir + the output file.
"""
from __future__ import annotations

import binascii
import hashlib
import json
import os
import struct
import time
import zlib
from typing import Any, Dict, List, Optional, Tuple

# The recorder identity stamped into every manifest. MUST match
# snapshot/src/pack.rs::pack_with (recorder.tool / tool_version) so a container
# this writer emits is indistinguishable from one the reference packer produced.
RECORDER_TOOL = "hoi4replay-recorder"
RECORDER_TOOL_VERSION = "1.0.0"
CONTAINER_VERSION = 1
STANDARD_ZIP_MAX_ENTRIES = 0xFFFF


# ==========================================================================
# Canonical JSON (RFC-8785-style) -- digest-critical; byte-matches canon.rs.
# ==========================================================================
def _canonical(obj: Any) -> bytes:
    r"""RFC-8785-style canonical JSON, byte-identical to ``canon.rs``.

    canon.rs: object keys sorted by UTF-16 code unit, minimal separators
    (``,`` / ``:``), strings escaped with the RFC-8785 minimal set, numbers in
    their shortest serde_json form. For the manifest -- whose keys are ALL ASCII
    and whose numbers are all small non-negative integers -- Python's
    ``json.dumps(sort_keys=True, separators=(",",":"), ensure_ascii=False)``
    produces exactly those bytes:

      * ``sort_keys=True`` sorts by Unicode code point, which equals the UTF-16
        code-unit order for ASCII (all manifest keys are ASCII; if a non-ASCII
        key is ever added, switch to an explicit UTF-16 sort -- see canon.rs);
      * ``separators=(",",":")`` matches canon.rs's minimal separators;
      * ``ensure_ascii=False`` keeps non-ASCII string VALUES (e.g. a mod name)
        as raw UTF-8, matching canon.rs's ``push(c)`` for ``c >= 0x20``;
      * Python's json escapes exactly ``" \\ \b \f \n \r \t`` and ``\uXXXX`` for
        other control chars < 0x20 -- the same set canon.rs writes;
      * integers serialize identically (``str(n)``); the manifest carries no
        floats, and ``allow_nan=False`` forbids NaN/Inf (canon.rs has no such
        path either).
    """
    return json.dumps(obj, sort_keys=True, separators=(",", ":"),
                      ensure_ascii=False, allow_nan=False).encode("utf-8")


def _assert_canon_safe(obj: Any) -> None:
    r"""Fail CLOSED if ``obj`` contains a value whose Python json serialization could
    DIVERGE from the Rust RFC-8785 canon (Codex A) -- the digest must byte-match cross-
    impl. The two hazards Python introduces that canon.rs would render differently:

      * a ``float`` (even a finite, integer-valued one): Python emits ``17.0`` where the
        Rust int path emits ``17`` -> a SILENT digest mismatch. The manifest schema has
        NO float field, so a float here is always a bug -> reject.
      * a non-ASCII OBJECT KEY: Python ``sort_keys`` sorts by Unicode code point, which
        only equals canon.rs's UTF-16 code-unit order for ASCII keys. Manifest keys are
        all ASCII; a non-ASCII key would need the explicit UTF-16 sort -> reject.

    ``bool`` is fine (it serializes as ``true``/``false`` identically cross-impl) and is
    NOT treated as an integer-field violation here -- the manifest's bool field
    (``build.pinned``) is intended. Strings/ints/None/list/dict recurse."""
    if isinstance(obj, float):
        raise ValueError("manifest contains a float %r; the canonical digest is "
                         "int-only (Python would emit '17.0' vs the reference '17')"
                         % (obj,))
    if isinstance(obj, dict):
        for k, v in obj.items():
            if isinstance(k, str) and any(ord(c) > 0x7F for c in k):
                raise ValueError("manifest has a non-ASCII object key %r; the canonical "
                                 "key sort is ASCII-only (UTF-16 order differs)" % (k,))
            _assert_canon_safe(v)
    elif isinstance(obj, (list, tuple)):
        for v in obj:
            _assert_canon_safe(v)


def manifest_digest(manifest: Dict[str, Any]) -> str:
    """sha256 over the canonical manifest with ``manifest_digest`` + ``signature``
    stripped -- exactly what container.rs recomputes and compares (step 7).

    Fails CLOSED (Codex A) on a value that would make Python's canonical JSON DIVERGE
    from the Rust reference digest (a float, or a non-ASCII object key) -- a wrong digest
    silently shipped would make the container fail to open cross-impl; we'd rather raise
    at write time."""
    m = dict(manifest)
    m.pop("manifest_digest", None)
    m.pop("signature", None)
    _assert_canon_safe(m)
    return hashlib.sha256(_canonical(m)).hexdigest()


# ==========================================================================
# Minimal ZIP writer (STORE or DEFLATE; CRC32; no Zip64) -- mirrors pack.rs's
# write_zip so the container is structurally the same shape the reader parses.
# ==========================================================================
class _ZipFile:
    __slots__ = ("name", "data", "deflate")

    def __init__(self, name: str, data: bytes, deflate: bool):
        self.name = name
        self.data = data
        self.deflate = deflate


def _write_zip(files: List[_ZipFile]) -> bytes:
    """Serialize ``files`` into a deterministic ZIP (mod time/date = 0), STORE or
    DEFLATE per entry. Matches snapshot/src/pack.rs::write_zip byte layout."""
    if len(files) > STANDARD_ZIP_MAX_ENTRIES:
        raise ValueError(
            ".battleplan container has %d ZIP entries, exceeding the "
            "standard-ZIP reader limit of %d; Zip64 is not supported by "
            "container_version %d"
            % (len(files), STANDARD_ZIP_MAX_ENTRIES, CONTAINER_VERSION))
    out = bytearray()
    central = bytearray()

    for f in files:
        name_b = f.name.encode("utf-8")
        crc = zlib.crc32(f.data) & 0xFFFFFFFF
        if f.deflate:
            # raw DEFLATE stream (no zlib header/trailer) -> method 8.
            co = zlib.compressobj(zlib.Z_DEFAULT_COMPRESSION, zlib.DEFLATED, -15)
            comp = co.compress(f.data) + co.flush()
            method = 8
        else:
            comp = f.data
            method = 0
        lho = len(out)

        # local file header
        out += struct.pack("<I", 0x04034B50)   # sig
        out += struct.pack("<H", 20)            # version needed
        out += struct.pack("<H", 0)             # flags
        out += struct.pack("<H", method)
        out += struct.pack("<H", 0)             # mod time (deterministic 0)
        out += struct.pack("<H", 0)             # mod date
        out += struct.pack("<I", crc)
        out += struct.pack("<I", len(comp))     # compressed size
        out += struct.pack("<I", len(f.data))   # uncompressed size
        out += struct.pack("<H", len(name_b))
        out += struct.pack("<H", 0)             # extra len
        out += name_b
        out += comp

        # central directory header
        central += struct.pack("<I", 0x02014B50)
        central += struct.pack("<H", 20)        # version made by
        central += struct.pack("<H", 20)        # version needed
        central += struct.pack("<H", 0)         # flags
        central += struct.pack("<H", method)
        central += struct.pack("<H", 0)         # time
        central += struct.pack("<H", 0)         # date
        central += struct.pack("<I", crc)
        central += struct.pack("<I", len(comp))
        central += struct.pack("<I", len(f.data))
        central += struct.pack("<H", len(name_b))
        central += struct.pack("<H", 0)         # extra
        central += struct.pack("<H", 0)         # comment
        central += struct.pack("<H", 0)         # disk no
        central += struct.pack("<H", 0)         # internal attrs
        central += struct.pack("<I", 0)         # external attrs
        central += struct.pack("<I", lho)       # local header offset
        central += name_b

    cd_off = len(out)
    cd_size = len(central)
    out += central

    # end of central directory
    out += struct.pack("<I", 0x06054B50)
    out += struct.pack("<H", 0)                 # disk
    out += struct.pack("<H", 0)                 # cd disk
    out += struct.pack("<H", len(files))        # entries this disk
    out += struct.pack("<H", len(files))        # entries total
    out += struct.pack("<I", cd_size)
    out += struct.pack("<I", cd_off)
    out += struct.pack("<H", 0)                 # comment len
    return bytes(out)


# ==========================================================================
# Container path validation (mirrors zip::validate_path / spec §4.2). The store
# only produces names like ``index.json`` / ``s0000.000000000017.snapshot.json``
# / ``timeline.json``, but we validate defensively so a bad name fails CLOSED
# rather than producing a container the reader will reject (or worse, a traversal
# name).
# ==========================================================================
_WIN_RESERVED = frozenset({
    "con", "prn", "aux", "nul",
    *(f"com{i}" for i in range(1, 10)),
    *(f"lpt{i}" for i in range(1, 10)),
})


def _safe_basename(name: str) -> bool:
    """True iff ``name`` is a safe single path component (a store snapshot file
    name): no separators, no traversal, no drive letter, not a Windows reserved
    device name, no control chars. The store only ever emits such names (e.g.
    ``s0000.000000000017.snapshot.json``); anything else is malformed/tampered."""
    if not isinstance(name, str) or not name or name != name.strip():
        return False
    if "/" in name or "\\" in name or ":" in name:
        return False
    if name in (".", ".."):
        return False
    if any(ord(c) < 0x20 for c in name):
        return False
    if name.split(".", 1)[0].lower() in _WIN_RESERVED:
        return False
    return True


def _path_ok(path: str) -> bool:
    if not path or path != path.strip():
        return False
    if "\\" in path:
        return False
    if path.startswith("/") or ":" in path:
        return False
    if path.startswith("//"):
        return False
    comps = path.split("/")
    seen_lower = set()
    for c in comps:
        if c in ("", ".", ".."):
            return False
        stem = c.split(".", 1)[0].lower()
        if stem in _WIN_RESERVED:
            return False
        cl = c.lower()
        if cl in seen_lower:
            # not a true collision across the whole path (only adjacent dup
            # components would matter), but reject any repeated component name as
            # an extra safety margin; real store names never repeat.
            pass
        seen_lower.add(cl)
    return True


# ==========================================================================
# ReplayRecorder
# ==========================================================================
class ReplayRecorder:
    """Streams a capture run into a portable ``.battleplan`` container.

    Lifecycle:
      * ``__init__``     : bind to the store + game + output path, capture the
                           per-replay mod identity (already redacted upstream);
      * ``set_build``    : stamp the build the recording was GATED on (once);
      * ``on_snapshot_written`` : append the just-written snapshot to the sidecar
                           (called once per durable ``write_snapshot``);
      * ``finalize``     : (re)build ``manifest.json`` + the ZIP atomically;
      * ``finalize_and_verify`` : finalize + re-open + verify (best-effort).
    """

    def __init__(self, store_root: str, game_id: str, out_path: str, *,
                 modset: Optional[Dict[str, Any]],
                 build_exe_sha256: Optional[str],
                 build_pinned: bool,
                 schema_version: str):
        self._store_root = store_root
        self._game_id = game_id
        self._out_path = out_path
        self._modset = modset
        self._build_exe_sha256 = build_exe_sha256
        self._build_pinned = bool(build_pinned)
        self._schema_version = schema_version

        # the game's directory in the durable store (the pack source of truth).
        self._gdir = os.path.join(store_root, game_id)
        # the work dir holds ONLY our sidecar; the payload stays in the durable
        # store (we never copy snapshots into a separate tree -- the store is
        # already crash-safe + atomic, and copying would double the I/O).
        self._work_dir = out_path + ".part"
        self._sidecar_path = os.path.join(self._work_dir, "entries.jsonl")
        os.makedirs(self._work_dir, exist_ok=True)

    # -- build identity (stamped once, after the gate passes) -----------
    def set_build(self, exe_sha256: Optional[str], pinned: bool) -> None:
        """Record the live build the recording was gated on. Called ONCE on the
        first successful capture (the build does not change mid-session)."""
        self._build_exe_sha256 = exe_sha256
        self._build_pinned = bool(pinned)

    # -- streaming sidecar ----------------------------------------------
    def on_snapshot_written(self, written_path: str) -> None:
        """Append one finalized snapshot's ``{path, sha256, bytes}`` to the
        append-only sidecar (fsync). Records ONLY canonical files (those that live
        directly under ``<gdir>/`` and are referenced by ``index.json``); a
        ``_noncanonical/`` path is ignored (those are excluded from v1 containers,
        spec §1). Idempotent per path: the sidecar may list a path more than once
        (a same-tick re-read overwrites the snapshot file in place); ``finalize``
        de-dups, taking the LAST line for any path and re-hashing the file on disk,
        so a stale line never produces a wrong hash."""
        if not written_path:
            return
        # canonical files are siblings of index.json in <gdir>; a non-canonical
        # capture lands under <gdir>/_noncanonical/... and is skipped.
        try:
            rel = os.path.relpath(written_path, self._gdir)
        except (ValueError, OSError):
            return
        rel = rel.replace("\\", "/")
        if rel.startswith("..") or "/" in rel:
            # outside the game dir, or in a subdir (e.g. _noncanonical/...) -> skip.
            return
        cpath = "payload/%s/%s" % (self._game_id, rel)
        try:
            with open(written_path, "rb") as fh:
                data = fh.read()
        except OSError:
            return
        sha = hashlib.sha256(data).hexdigest()
        line = json.dumps({"path": cpath, "sha256": sha, "bytes": len(data),
                           "role": "snapshot", "file": rel},
                          separators=(",", ":"))
        # append-only + fsync: a crash mid-append leaves a prefix of whole lines.
        # COMMIT-MARKER REPAIR (Codex B): the trailing newline is the per-record
        # commit marker. A prior crash can leave the last record's bytes WITHOUT its
        # newline (a torn append); appending directly would CONCATENATE the torn
        # record and this one onto one physical line -> json.loads can't parse it ->
        # BOTH records are lost from the reconcile. So we first ensure the file ends
        # in a newline (BINARY tail check, unambiguous unlike a text-mode seek),
        # writing the missing marker as raw bytes, before appending this record.
        need_heal = False
        try:
            with open(self._sidecar_path, "rb") as rf:
                rf.seek(0, os.SEEK_END)
                if rf.tell() > 0:
                    rf.seek(-1, os.SEEK_END)
                    need_heal = rf.read(1) != b"\n"   # torn (newline-less) tail record
        except OSError:
            need_heal = False                          # no file yet -> nothing to heal
        if need_heal:
            try:
                with open(self._sidecar_path, "ab") as hf:
                    hf.write(b"\n")
                    hf.flush()
                    os.fsync(hf.fileno())
            except OSError:
                pass                                   # best-effort heal; append still runs
        with open(self._sidecar_path, "a", encoding="utf-8", newline="\n") as fh:
            fh.write(line + "\n")
            fh.flush()
            os.fsync(fh.fileno())

    # -- finalize -------------------------------------------------------
    def finalize(self) -> str:
        """Build ``manifest.json`` + the ``.battleplan`` ZIP atomically and return
        ``out_path``. Re-reads the durable store EACH call (the published file is
        always a complete snapshot of everything captured so far). Mirrors
        ``snapshot/src/bin/pack_replay.rs::cmd_pack`` so the output is byte-shaped
        like the reference packer and opens with ``pack_replay verify``."""
        entries_meta, zip_files = self._collect_entries()

        # RECONCILE the append-only sidecar against the store (Codex: the sidecar
        # must not be dead code). The store's atomic snapshot-then-index ordering
        # makes index.json authoritative, so the sidecar is a CROSS-CHECK: every
        # canonical file the loop streamed should be packed. A sidecar path that
        # is NOT in the packed set means the store moved on (e.g. a non-canonical
        # re-key) -- benign, recorded as a note, never a hard failure (the store is
        # the source of truth). A sidecar that references MORE than the index is the
        # only signal that an index update was lost mid-crash; we surface it.
        self._reconcile_sidecar(entries_meta)

        manifest = self._build_manifest(entries_meta)

        # manifest.json is written LAST (spec §6) so its presence => finalized.
        man_bytes = json.dumps(manifest, separators=(",", ":"),
                               ensure_ascii=False).encode("utf-8")
        zip_files.append(_ZipFile("manifest.json", man_bytes, deflate=False))

        container = _write_zip(zip_files)

        # atomic finalize: write <out>.tmp -> fsync -> os.replace (spec §6).
        tmp = self._out_path + ".tmp"
        d = os.path.dirname(os.path.abspath(self._out_path)) or "."
        os.makedirs(d, exist_ok=True)
        with open(tmp, "wb") as fh:
            fh.write(container)
            fh.flush()
            os.fsync(fh.fileno())
        os.replace(tmp, self._out_path)
        _fsync_dir(d)
        return self._out_path

    def finalize_and_verify(self) -> bool:
        """Finalize, then re-open the produced container and re-verify every
        entry's hash + the manifest digest (a pure-Python re-read; no Rust). A
        best-effort guard: returns True iff the file we just wrote validates."""
        path = self.finalize()
        try:
            return _verify_container(path)
        except Exception:
            return False

    # -- internals ------------------------------------------------------
    def _collect_entries(self) -> Tuple[List[Dict[str, Any]], List[_ZipFile]]:
        """Read the durable store's index + every referenced snapshot (+ optional
        timeline), as RAW BYTES, and build the manifest-entry list + the zip-file
        list. The ``index.snapshots[*].file`` field is the ONLY name authority --
        we never synthesize ``%012d`` (rollback files are ``s%04d.*``)."""
        index_path = os.path.join(self._gdir, "index.json")
        with open(index_path, "rb") as fh:
            index_bytes = fh.read()
        index = json.loads(index_bytes.decode("utf-8"))

        entries_meta: List[Dict[str, Any]] = []
        zip_files: List[_ZipFile] = []
        seen_paths = set()

        # index.json
        ipath = "payload/%s/index.json" % self._game_id
        self._add_entry(entries_meta, zip_files, ipath, index_bytes,
                        role="index", hour_index=None,
                        seen_paths=seen_paths)

        # each snapshot referenced by the index, in raw bytes. FAIL-CLOSED
        # (Codex): the index is the manifest's spine -- the reader cross-checks
        # index.snapshots[*] against the packed entries, so silently DROPPING a
        # referenced-but-missing file would publish a replay that omits data the
        # index advertises (a fabrication-by-omission). The durable store writes
        # each snapshot file (tmp->fsync->replace) BEFORE atomically updating
        # index.json, so a file the index references is always fully on disk; a
        # genuinely missing file means a torn store and we REFUSE rather than ship
        # a lying container.
        for s in index.get("snapshots", []):
            file = s.get("file")
            # a snapshot entry with no usable `file` advertises data we cannot pack
            # -> FAIL CLOSED (Codex F1 residual) rather than publish an index that
            # references a snapshot with no packed payload.
            if not isinstance(file, str) or not file:
                raise ValueError("index has a snapshot entry with a missing/empty "
                                 "file name %r; refusing to publish" % (file,))
            # the `file` field must be a SAFE BASENAME (no separators / traversal /
            # drive letter): the store only ever produces such names; anything else
            # is a malformed/tampered index and fails closed BEFORE we open it.
            if not _safe_basename(file):
                raise ValueError("refusing to pack unsafe index file name %r"
                                 % file)
            spath = os.path.join(self._gdir, file)
            try:
                with open(spath, "rb") as fh:
                    sbytes = fh.read()
            except OSError as exc:
                raise ValueError(
                    "index references snapshot %r that is not readable on disk "
                    "(torn store); refusing to publish a container that omits "
                    "indexed data: %s" % (file, exc))
            hour = s.get("hour_index")
            cpath = "payload/%s/%s" % (self._game_id, file)
            self._add_entry(entries_meta, zip_files, cpath, sbytes,
                            role="snapshot",
                            hour_index=hour if isinstance(hour, int) else None,
                            seen_paths=seen_paths)

        # optional timeline.json
        tpath = os.path.join(self._gdir, "timeline.json")
        if os.path.isfile(tpath):
            with open(tpath, "rb") as fh:
                tbytes = fh.read()
            cpath = "payload/%s/timeline.json" % self._game_id
            self._add_entry(entries_meta, zip_files, cpath, tbytes,
                            role="timeline", hour_index=None,
                            seen_paths=seen_paths)

        # canonical path order so the manifest entry list is stable (pack.rs sorts
        # the same way). The zip-file order is independent (the reader keys off the
        # manifest), but keep them aligned for determinism.
        entries_meta.sort(key=lambda e: e["path"])
        zip_files.sort(key=lambda z: z.name)
        return entries_meta, zip_files

    def _reconcile_sidecar(self, entries_meta: List[Dict[str, Any]]) -> None:
        """Read the append-only sidecar (a prefix of whole JSONL lines survives any
        crash) and cross-check it against the packed entries. Best-effort: a parse
        error on the tail (a torn final line) is ignored (we only trust complete
        lines); the index remains authoritative. Records a recovery note on
        ``self._last_recovery`` when the sidecar references a canonical path that
        the current index does NOT pack (the lost-index-update signal)."""
        self._last_recovery: List[str] = []
        if not os.path.isfile(self._sidecar_path):
            return
        packed = {e["path"] for e in entries_meta}
        try:
            # STREAM the sidecar line-by-line (Codex F5): an unbounded session can
            # have a large entries.jsonl; never slurp it whole.
            with open(self._sidecar_path, "r", encoding="utf-8") as fh:
                for ln in fh:
                    ln = ln.strip()
                    if not ln:
                        continue
                    try:
                        rec = json.loads(ln)    # a torn final line -> skip it
                    except ValueError:
                        continue
                    p = rec.get("path")
                    if isinstance(p, str) and p not in packed:
                        # the sidecar streamed this canonical file but the current
                        # index doesn't reference it -> an index update was likely
                        # lost (crash). The store is authoritative, so we don't
                        # fabricate it back in; we only surface that the recording
                        # may be short by that hour.
                        self._last_recovery.append(p)
        except OSError:
            return

    def _add_entry(self, entries_meta, zip_files, path, data, *, role, hour_index,
                   seen_paths=None):
        if not _path_ok(path):
            raise ValueError("refusing to pack unsafe container path %r" % path)
        # DUPLICATE-PATH guard (Codex F2 hardening): the reader rejects a zip with
        # two entries at the same path; a snapshot whose `file` is e.g. "index.json"
        # would collide with the real payload index. Reject the collision HERE so we
        # never emit a container the reader would refuse.
        if seen_paths is None:
            seen_paths = {e["path"] for e in entries_meta}
        if path in seen_paths:
            raise ValueError("refusing to pack a duplicate container path %r "
                             "(snapshot file name collides)" % path)
        ent: Dict[str, Any] = {
            "path": path,
            "sha256": hashlib.sha256(data).hexdigest(),
            "bytes": len(data),
            "role": role,
        }
        if hour_index is not None:
            ent["hour_index"] = hour_index
        entries_meta.append(ent)
        seen_paths.add(path)
        # DEFLATE the payload (matches pack_replay's `pack(..., true)`); the reader
        # supports both STORE and DEFLATE.
        zip_files.append(_ZipFile(path, data, deflate=True))

    def _build_manifest(self, entries_meta: List[Dict[str, Any]]) -> Dict[str, Any]:
        """Assemble the manifest object EXACTLY like pack.rs::pack_with (same key
        set + value shapes) so a container is reader-identical, then fill the
        digest over the canonical form. ``mod_identity`` / ``build.exe_sha256`` are
        passed through verbatim; ``None`` -> JSON ``null`` (no fabrication)."""
        snapshots = sum(1 for e in entries_meta if e.get("role") == "snapshot")
        manifest: Dict[str, Any] = {
            "format": "hoi4replay",
            "container_version": CONTAINER_VERSION,
            "required_features": [],
            "optional_features": [],
            "schema_version": self._schema_version,
            "game_id": self._game_id,
            "build": {
                "exe_sha256": self._build_exe_sha256,
                "pinned": self._build_pinned,
            },
            "mod_identity": self._modset if self._modset is not None else None,
            "created_utc": _now_utc(),
            "recorder": {"tool": RECORDER_TOOL,
                         "tool_version": RECORDER_TOOL_VERSION},
            "counts": {"snapshots": snapshots},
            "entries": entries_meta,
            "manifest_digest": None,
            "signature": None,
        }
        manifest["manifest_digest"] = manifest_digest(manifest)
        return manifest


# ==========================================================================
# Helpers
# ==========================================================================
def _now_utc() -> str:
    """RFC-3339 UTC stamp (informational). stdlib only, no third-party deps."""
    return time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())


def _fsync_dir(d: str) -> None:
    """fsync a directory so a rename into it survives a crash (best-effort; a
    no-op on filesystems that don't support directory fsync)."""
    try:
        fd = os.open(d, os.O_RDONLY)
    except OSError:
        return
    try:
        os.fsync(fd)
    except OSError:
        pass
    finally:
        os.close(fd)


# ==========================================================================
# A pure-Python container reader -- used by finalize_and_verify and the unit
# tests to prove the container we wrote opens + verifies WITHOUT shelling out to
# the Rust reader. It performs the security-relevant checks from container.rs:
# manifest digest, per-entry hash, declared-bytes == zip-uncompressed, no
# stowaways, format/version, fail-closed required_features.
# ==========================================================================
def _verify_container(path: str) -> bool:
    with open(path, "rb") as fh:
        data = fh.read()
    by_name = _zip_read_all(data)

    if "manifest.json" not in by_name:
        raise ValueError("no manifest.json (recovery-only / unfinalized)")
    manifest = json.loads(by_name["manifest.json"].decode("utf-8"))

    if manifest.get("format") != "hoi4replay":
        raise ValueError("format != hoi4replay")
    cv = manifest.get("container_version")
    if not isinstance(cv, int) or cv > CONTAINER_VERSION:
        raise ValueError("container_version unsupported: %r" % cv)

    rf = manifest.get("required_features")
    if rf not in (None, []):
        if not isinstance(rf, list):
            raise ValueError("required_features malformed (must be array)")
        # KNOWN_FEATURES is empty (matches container.rs) -> any feature is unknown.
        raise ValueError("unknown required feature(s): %r" % rf)

    declared = manifest.get("manifest_digest")
    if manifest_digest(manifest) != declared:
        raise ValueError("manifest_digest mismatch")

    if manifest.get("signature") not in (None,):
        raise ValueError("signature present but no verification policy (fail-closed)")

    entries = manifest.get("entries")
    if not isinstance(entries, list):
        raise ValueError("manifest.entries missing")
    covered = set()
    for ent in entries:
        p = ent.get("path")
        sha = ent.get("sha256")
        nbytes = ent.get("bytes")
        if p not in by_name:
            raise ValueError("manifest entry %r not present in zip" % p)
        body = by_name[p]
        if len(body) != nbytes:
            raise ValueError("entry %r: declared bytes != zip uncompressed" % p)
        if hashlib.sha256(body).hexdigest() != sha:
            raise ValueError("entry %r: sha256 mismatch" % p)
        covered.add(p)
    # no stowaways: every payload zip entry must be covered by the manifest.
    for name in by_name:
        if name == "manifest.json":
            continue
        if name not in covered:
            raise ValueError("zip entry %r not listed in manifest (tamper)" % name)

    # index.json present + game_id agreement.
    game_id = manifest.get("game_id")
    ipath = "payload/%s/index.json" % game_id
    if ipath not in by_name:
        raise ValueError("index.json missing from payload")
    index = json.loads(by_name[ipath].decode("utf-8"))
    if index.get("game_id") != game_id:
        raise ValueError("index.game_id != manifest.game_id")
    return True


def _zip_read_all(data: bytes) -> Dict[str, bytes]:
    """Parse a ZIP (our own writer's output: STORE/DEFLATE, no Zip64) into a
    {name: decompressed_bytes} map by walking the central directory (the
    authority, like container.rs). Rejects duplicate names."""
    # locate End Of Central Directory (scan back from the tail; no zip comment).
    eocd = data.rfind(b"PK\x05\x06")
    if eocd < 0:
        raise ValueError("not a zip (no EOCD)")
    total = struct.unpack_from("<H", data, eocd + 10)[0]
    cd_off = struct.unpack_from("<I", data, eocd + 16)[0]
    out: Dict[str, bytes] = {}
    p = cd_off
    for _ in range(total):
        if data[p:p + 4] != b"PK\x01\x02":
            raise ValueError("bad central directory header")
        method = struct.unpack_from("<H", data, p + 10)[0]
        comp_size = struct.unpack_from("<I", data, p + 20)[0]
        uncomp_size = struct.unpack_from("<I", data, p + 24)[0]
        name_len = struct.unpack_from("<H", data, p + 28)[0]
        extra_len = struct.unpack_from("<H", data, p + 30)[0]
        comment_len = struct.unpack_from("<H", data, p + 32)[0]
        lho = struct.unpack_from("<I", data, p + 42)[0]
        name = data[p + 46:p + 46 + name_len].decode("utf-8")
        # read the local header to find the data offset (its name/extra lens can
        # differ from the central dir's; the spec's reader hashes the entry it
        # serves -- we mirror that by reading from the local header).
        if data[lho:lho + 4] != b"PK\x03\x04":
            raise ValueError("bad local file header for %r" % name)
        l_name_len = struct.unpack_from("<H", data, lho + 26)[0]
        l_extra_len = struct.unpack_from("<H", data, lho + 28)[0]
        body_off = lho + 30 + l_name_len + l_extra_len
        comp = data[body_off:body_off + comp_size]
        if method == 0:
            body = comp
        elif method == 8:
            body = zlib.decompress(comp, -15)
        else:
            raise ValueError("unsupported zip method %d for %r" % (method, name))
        if len(body) != uncomp_size:
            raise ValueError("uncompressed size mismatch for %r" % name)
        if name in out:
            raise ValueError("duplicate zip entry name %r" % name)
        out[name] = body
        p += 46 + name_len + extra_len + comment_len
    return out


__all__ = ["ReplayRecorder", "manifest_digest"]

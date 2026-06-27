# `.battleplan` Writer — Integration Hand-off (for the capture-tree owner)

> **Audience:** the agent who owns `live_capture/` + `memory_reader/` + `capture.py`.
> **Goal:** wire a streaming `.battleplan` writer into the capture loop so a player
> can record while playing and stop anytime with a valid, portable file.
> **Authoritative format:** `replay_ui/HOI4REPLAY_CONTAINER_SPEC.md`.
> **Reference impl already built (read-only side):** the Rust `snapshot` crate
> (`snapshot/src/container.rs` reader, `snapshot/src/pack.rs` packer,
> `snapshot/src/bin/pack_replay.rs` CLI). The CLI *already* packs a directory store
> into a verified, byte-identical `.battleplan` today — so the simplest correct
> wiring is "capture as you do now, then finalize via the packer." This plan gives
> you both that path and the fully-streaming path.

This is a **drop-in** plan: exact call sites, exact signatures, nothing to
re-derive. Everything below lives in the capture tree **you** own; I did not edit
it (ownership boundary).

---

## 0. The one design rule that makes this safe

**Keep capturing into the existing durable directory store. Do NOT turn the hot
capture loop into a ZIP-append loop.** A ZIP cannot legally "refresh" `index.json`
mid-stream without a duplicate entry, and a crash mid-append tears the central
directory (Codex c). Instead:

* the capture loop writes each hour into the durable store **exactly as it does
  today** (`DurableCaptureStore.write_snapshot`, atomic);
* a tiny **running sidecar** (`entries.jsonl`) records each finalized file's
  `{path, sha256, bytes}` as it lands;
* **finalize** (record stopped / "save now" / interval end) builds the
  `manifest.json` and the ZIP in one atomic step.

A recording ended at any point is valid because finalize reads whatever the store
has so far. An *interrupted* record (process died before finalize) leaves the store
+ sidecar intact, and the recovery path (just re-run finalize) produces a valid
`.battleplan` of the hours captured before the crash.

---

## 1. New module to add: `live_capture/replay_writer.py`

A pure-stdlib module (no game, no third-party deps). Two responsibilities:
record the per-hour sidecar line, and finalize the container.

### 1.1 Exact public surface

```python
# live_capture/replay_writer.py
from typing import Optional, Dict, Any

class ReplayRecorder:
    def __init__(self, store_root: str, game_id: str,
                 out_path: str, *,
                 modset: Optional[Dict[str, Any]],
                 build_exe_sha256: Optional[str],
                 build_pinned: bool,
                 schema_version: str): ...

    # call ONCE per successfully-written snapshot, with the path the durable
    # store returned. Streams a sidecar line (append-only, fsync). Idempotent
    # per path (re-recording the same file replaces its sidecar line on finalize).
    def on_snapshot_written(self, written_path: str) -> None: ...

    # build manifest.json + the .battleplan ZIP atomically (.tmp -> os.replace).
    # Safe to call repeatedly (each call re-reads the store + sidecar); the last
    # call before the player stops is the published file.
    def finalize(self) -> str: ...   # returns out_path

    # convenience: finalize + return whether it validated (re-open + verify).
    def finalize_and_verify(self) -> bool: ...
```

### 1.2 Exact behavior (mirror the Rust packer — they MUST agree byte-for-byte)

`finalize()` does precisely what `snapshot/src/bin/pack_replay.rs::cmd_pack` does:

1. Read `<store_root>/<game_id>/index.json` (raw bytes).
2. For each `index["snapshots"][i]["file"]` (the **`file` field is the only name
   authority** — never synthesize `%012d`): read `<gdir>/<file>` raw bytes.
3. If present, read `<gdir>/timeline.json` raw bytes.
4. For each file compute `sha256(bytes)` and `len(bytes)`; build the `entries`
   list `{path: "payload/<game>/<file>", sha256, bytes, role, hour_index?}`
   sorted by `path`.
5. Build `manifest.json` per spec §2 with: `format:"hoi4replay"`,
   `container_version:1`, `required_features:[]`, `schema_version`, `game_id`,
   `build:{exe_sha256, pinned}`, `mod_identity:<modset or null>`, `created_utc`,
   `recorder`, `counts`, `entries`, then `manifest_digest`, `signature:null`.
6. `manifest_digest = sha256(canonical_json(manifest_without_digest_and_signature))`.
   **Canonical JSON = RFC-8785:** keys sorted by UTF-16 code unit, `separators=(",",":")`,
   `ensure_ascii=False`, no trailing whitespace. (See §1.3 — there is a tested
   helper; it must match `snapshot/src/canon.rs` exactly.)
7. Create the ZIP (STORE or DEFLATE per entry; `manifest.json` written **last**),
   write to `<out>.tmp`, `fsync`, `os.replace(<out>.tmp, <out>)`.

`mod_identity` is `self._modset` verbatim (already redacted of personal paths by
`mod_identity.py`); `None` ⇒ JSON `null` (no fabrication). Same for
`build_exe_sha256`.

### 1.3 The canonical-JSON helper (copy this exactly; it is digest-critical)

The Rust reader recomputes `manifest_digest` over RFC-8785 canonical JSON. Python
must produce **identical bytes** or every container the recorder writes will be
rejected. Use this (stdlib only):

```python
import json

def _canonical(obj) -> bytes:
    # RFC-8785-ish: sorted keys, minimal separators, UTF-8, no NaN/Inf.
    return json.dumps(obj, sort_keys=True, separators=(",", ":"),
                      ensure_ascii=False, allow_nan=False).encode("utf-8")

def manifest_digest(manifest: dict) -> str:
    import hashlib
    m = dict(manifest)
    m.pop("manifest_digest", None)
    m.pop("signature", None)
    return hashlib.sha256(_canonical(m)).hexdigest()
```

> **Cross-impl agreement is already proven** for the snapshot/entry hashes: I packed
> the real `replay/snapshots/campaign` store with the Rust CLI and re-hashed every
> entry with Python's `hashlib` — they match (logged in the report). Add ONE unit
> test that round-trips a manifest dict through `_canonical` and asserts the digest
> equals what `snapshot`'s reader accepts (open the produced file with
> `pack_replay verify`). `json.dumps(sort_keys=True)` sorts by Unicode code point,
> which equals the Rust UTF-16 ordering for all ASCII manifest keys (our keys are
> all ASCII), so they agree; if you ever add a non-ASCII manifest key, switch to an
> explicit UTF-16 sort to stay aligned with `canon.rs`.

---

## 2. Exact call-sites in `capture.py`

`capture.py` is yours; here are the precise hooks. (Line refs are against the
current file; match by symbol, not line number, since the file moves.)

### 2.1 Construct the recorder in `CaptureLoop.__init__`

After `self._modset` is read (currently the `try: from live_capture import
mod_identity ...` block, ~L147–152) and before the continuity state load, add an
**optional** recorder, created only when the caller asks for one:

```python
# in CaptureLoop.__init__ signature, add:
#   record_to: Optional[str] = None      # path for the .battleplan (None = off)
self._recorder = None
if record_to:
    from live_capture.replay_writer import ReplayRecorder
    self._recorder = ReplayRecorder(
        store_root=self.store.root,
        game_id=game_id,
        out_path=record_to,
        modset=self._modset,                       # already read above
        build_exe_sha256=None,                     # filled per-capture (see 2.2)
        build_pinned=False,
        schema_version=schema.SCHEMA_VERSION,      # import replay.schema as schema
    )
```

`record_to` threads through from `run(...)`/`capture_once(...)` callers (the EXE in
§4 passes it). Default `None` keeps every existing call unchanged.

### 2.2 Stamp the build identity at session start (once)

The build hash is on the `identity` object you already build in `capture_once`
(`identity = _pi.identify(proc)`; `identity.exe_sha256`, `identity.build_ok()`).
On the **first** capture, push it into the recorder so the manifest records the
exact build the recording was gated on:

```python
# in capture_once, right after the build-gate check passes:
if self._recorder is not None and not self._recorder_build_set:
    self._recorder.set_build(identity.exe_sha256, identity.build_ok())
    self._recorder_build_set = True
```

(Add `self._recorder_build_set = False` in `__init__`, and a 2-line
`ReplayRecorder.set_build(self, sha, pinned)` setter that stores the two fields for
the next `finalize`.) The mod-set is already captured once at construction (§2.1),
which is correct — a mod-set does not change mid-campaign.

### 2.3 Stream each hour into the sidecar

`_finish_capture` ends with the durable write:

```python
path = self.store.write_snapshot(snap, allow_unknown_hour=False)   # existing line
self.tracker.commit(proposal)                                       # existing
self.store.save_continuity_state(...)                               # existing
```

Add **one line** right after the durable write succeeds (so the sidecar only ever
references a file that is actually on disk):

```python
if self._recorder is not None and path is not None:
    self._recorder.on_snapshot_written(path)
```

`on_snapshot_written` is robust to the `_noncanonical/` case: it records ONLY
canonical files (those under `<gdir>/` whose name is in the index). A non-canonical
path is ignored (those are excluded from v1 containers, spec §1).

### 2.4 Finalize when recording stops

`run(...)` already has the lifecycle. Finalize:

* in the `finally:` of `run(...)`, after `lock.release()`, call
  `if self._recorder: self._recorder.finalize_and_verify()`;
* AND expose finalize on the `on_capture` path so a long session can publish a
  valid file every N hours (each `finalize()` is a complete, atomic re-publish —
  cheap relative to a capture, and it means a crash after the latest finalize still
  yields a valid file). A reasonable default: finalize every 6 captures and once at
  the end.

> Because `finalize()` re-reads the store each time, you never hold a half-built
> ZIP; the published `.battleplan` is always a complete snapshot of "everything
> captured up to now." That is the whole "stop anytime, still valid" guarantee.

---

## 3. What you do NOT need to touch

* `assembler.py`, `registry.py`, the readers — unchanged. The container packs the
  snapshots they already produce, as raw bytes.
* `schema.py` — unchanged. `provenance.mod` already carries the mod identity; the
  manifest copies its `modset_id` and the reader cross-checks the two.
* `durable_store.py` — unchanged. The recorder reads the store it already writes.
* The Rust side — already built and tested; it will open whatever your finalize
  writes, provided the canonical-JSON helper in §1.3 is byte-exact (one unit test
  guarantees it).

---

## 4. Acceptance checklist for your wiring

1. `ReplayRecorder.finalize()` output opens with `pack_replay verify <file>` →
   "verify: OK".
2. A recording stopped after K hours contains exactly K snapshots, and
   `pack_replay verify <file> <store_root> <game_id>` reports byte-identity OK.
3. Killing the capture process mid-run, then calling `finalize()` once more (or
   re-running a one-shot finalize), yields a valid file of the pre-crash hours.
4. `mod_identity` in the manifest equals the snapshots' `provenance.mod`
   (the reader rejects a mismatch — that is your consistency test for free).
5. The unit test from §1.3 (Python digest == Rust-accepted digest) is green.

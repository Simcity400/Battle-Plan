# `.battleplan` Portable Recording Container — Format Specification v1

> A single, self-contained file that holds the **entire captured snapshot series**
> of one game plus **everything the replay surface needs to render it faithfully** —
> the mod identity, the build hash, the schema version, an integrity manifest.
> Produced on one machine while a player records, opened on another. It behaves
> exactly like a save game to the replay surface: open it, get the full replay.
>
> Status: **normative** for container_version `1`. Prior review findings are
> folded in below and tagged `[Codex …]`.
>
> **Payload flavors (2026-07-02 clarification).** Two payload shapes exist in
> the wild under this container format:
>
> 1. **Snapshot flavor** (this spec's §1 tree): hourly world snapshots +
>    populated `index.json`. This is the shape the replay layer consumes
>    (`replay_core`), and the proposed production contract — see
>    `EXTRACTION_INTERFACE_REQUESTS.md` R1.
> 2. **Compact-stream flavor** (current production recorder): `index.json`
>    with an empty `snapshots` list plus entries with roles
>    `collector_compact_stream` and `recorder_manifest`. Container mechanics
>    (§1.1–§4) apply unchanged; there is no consumable world state inside.
>    Replay tooling refuses these for playback with a precise error rather
>    than guessing compact record layouts.

---

## 0. Design goals (and the hard rules they obey)

| Goal | Rule |
|------|------|
| **Loss-LESS round-trip** | The store directory → pack → unpack yields **byte-identical** snapshot/index/timeline files. Payload files are copied as **raw bytes**; they are NEVER parsed-and-re-serialized. `[Codex a/e]` |
| **No fabrication preserved** | Because payload is byte-copied, the Absent / explicit-null / measured-`0` trichotomy survives untouched — no serializer can collapse `null`→absent or default-fill a `0`. The reader enforces it again at the API surface. `[Codex e]` |
| **Forward-compatible** | Typed header + `container_version`; unknown manifest fields are preserved; a `required_features` list lets an old reader **refuse** a file that needs semantics it lacks (silent ignore is unsafe). `[Codex b]` |
| **Integrity-checked** | Per-entry SHA-256 over raw decompressed bytes + a single top-level `manifest_digest` over the canonical manifest (covering ALL security metadata, not just file hashes). Optional detached Ed25519 signature slot for true tamper-PROOFING. `[Codex b]` |
| **Streamable / crash-safe** | A recording can be ended at any time and the finalized file is valid; an interrupted (never-finalized) file is recovery-only, never silently accepted. `[Codex c]` |
| **No personal paths** | Container paths are relative, `/`-separated, normalized; mod identity stores basenames/ids/versions only (already redacted upstream in `mod_identity.py`). |

---

## 1. Physical layout — a ZIP container

`.battleplan` is a standard ZIP archive (so any unzip tool can inspect it), with a
fixed internal tree. All internal paths are **UTF-8, `/`-separated, relative,
normalized** (no `.`/`..`/leading-slash/drive-letter/UNC/backslash component).

```
<name>.battleplan                         (ZIP)
├── manifest.json                         ← typed header + integrity (written LAST)
└── payload/
    └── <game_id>/
        ├── index.json                    ← byte-identical copy of the store index
        ├── <snapshot-file>.snapshot.json  ← byte-identical copies (one per hour)
        │   …                               filename is whatever index.file says
        ├── collector_raw/
        │   └── collector_frames.jsonl     ← optional full raw collector stream
        └── timeline.json                 ← byte-identical copy (if present)
```

Notes:

* **`<snapshot-file>` is taken verbatim from `index.snapshots[*].file`.** Both
  in-the-wild forms occur and BOTH are preserved exactly:
  `%012d.snapshot.json` (plain) and `s%04d.%012d.snapshot.json`
  (segment-prefixed rollback files). The packer NEVER synthesizes a name from
  `hour_index`. `[Codex a]`
* `timeline.json` is optional (older stores may lack it); its presence is recorded
  in the manifest.
* `collector_raw/collector_frames.jsonl` is optional. When present it is a
  manifest-covered raw JSONL stream of every validated collector frame, including
  sub-hour `domain_delta` / `gap` frames that are not folded into the hourly
  snapshot index. It is discoverable through `manifest.raw_streams` and
  `entries[*].role == "collector_raw_stream"`; it MUST NOT be put in
  `required_features` until native readers advertise support for that feature.
* An optional G2 engine capture lives at `g2/g2_engine_pixel_witness.json`
  and `g2/g2_engine_map_boundary.bmp`. It is declared only by the selected
  game's exact `index.json.g2_engine_capture` object and uses roles
  `g2_engine_witness` and `g2_engine_bgra_bmp`. The declaration binds both
  hashes and lengths, plus the immutable capture-time content digest and the
  finalized replay-content digest. Replay admission rejects undeclared roles,
  missing/duplicate entries, path/hash/length drift, malformed witness/BMP
  bytes, or recorder-authority join drift. Loose files are not replay authority.
* **Excluded by design** (machine-local / volatile, not portable): the capture
  `*.lock` files, `_continuity/continuity_state.json` (carries PIDs/runtime
  state). The continuity *information the reader needs* already travels inside each
  snapshot's `_capture` block, so dropping the runtime state file is loss-less for
  replay purposes. `[Codex a]`
* `_noncanonical/` snapshots (unknown-hour captures) are **not** part of the
  canonical series and are excluded from v1 (they have no `hour_index`, so the app
  cannot place them on the timeline). A future `container_version` may add an
  optional `extras/` tree for them; excluding them now is explicit, not a silent
  drop.

### 1.1 ZIP encoding constraints (deterministic, safe)

* Compression: `store` (0), `deflate` (8), or Zstandard (93) per entry. Zstandard
  decoding is constrained by the reader's declared maximum window log.
* Each entry's path validated against the rules in §4.2 **before** any extraction
  or hashing.
* **Duplicate entry names are rejected** (a ZIP may legally contain two
  `payload/<g>/index.json`; verifier and loader could then disagree on which one
  is real). `[Codex b/c]`
* The reader hashes the **decompressed bytes of the exact central-directory entry**
  it will serve; it never trusts a local-header name for path or content. `[Codex b]`
* No entry is ever extracted to disk by the reader. Path opens retain one pinned
  archive handle, parse central-directory records incrementally, and materialize
  only the requested decompressed entry, so zip-slip/symlink/device-name
  extraction attacks do not apply to the read path. (A future "unpack to dir"
  tool MUST apply §4.2.) `[Codex b]`

---

## 2. `manifest.json` (the typed header)

Written **last** during a streaming record (so its presence ⇒ a finalized, valid
file). One JSON object:

```jsonc
{
  "format": "hoi4replay",            // constant tag; reject if absent/wrong
  "container_version": 1,            // integer; reader refuses > its max known
  "required_features": [],           // features a reader MUST understand or REFUSE
  "optional_features": ["collector_raw_stream_v1"], // features a reader MAY ignore

  "schema_version": "1",             // replay snapshot SCHEMA_VERSION at capture
  "game_id": "campaign",             // the one game packed in payload/<game_id>/

  "build": {                         // the build the capture was GATED on
    "exe_sha256": "16022851…b120",   // hoi4.exe on-disk sha256 (process_identity)
    "pinned": true                   // was this == the reader-known pinned build?
  },

  "mod_identity": {                  // provenance.mod (mod_identity.py / schema)
    "modset_id": "…",                // order-sensitive digest, or "vanilla", or null
    "source": "dlc_load.json",       // where the launcher config came from, or null
    "load_order": ["test.mod"],      // descriptor basenames, in load order
    "mods": [ { "name": …, "version": …, "workshop_id": …, "descriptor": … } ]
  },                                 // whole object MAY be null = "mod-set not read"

  "created_utc": "2026-06-26T21:30:00Z",
  "recorder": { "tool": "hoi4replay-recorder", "tool_version": "1.0.0" },

  "counts": { "snapshots": 412, "countries": 143, "states": 1081,
              "raw_streams": 1, "raw_collector_frames": 4200 },

  "entries": [                       // EVERY payload file, in canonical path order
    { "path": "payload/campaign/index.json",
      "sha256": "…", "bytes": 1234, "role": "index" },
    { "path": "payload/campaign/000017000461.snapshot.json",
      "sha256": "…", "bytes": 998877, "role": "snapshot", "hour_index": 17000461 },
    { "path": "payload/campaign/collector_raw/collector_frames.jsonl",
      "sha256": "…", "bytes": 65536, "role": "collector_raw_stream",
      "format": "collector_frames_jsonl_v1", "frame_count": 4200,
      "frame_types": ["collector_started", "domain_delta", "gap"],
      "sequence_count": 4200, "domains": ["clock", "divisions"],
      "delta_domains": ["divisions"], "gap_domains": [],
      "first_sequence": 10, "last_sequence": 4209 },
    { "path": "payload/campaign/timeline.json",
      "sha256": "…", "bytes": 4096, "role": "timeline" }
    // roles: "index" | "snapshot" | "timeline" | "collector_raw_stream"
  ],

  "raw_streams": [
    { "path": "payload/campaign/collector_raw/collector_frames.jsonl",
      "role": "collector_raw_stream", "format": "collector_frames_jsonl_v1",
      "frame_count": 4200, "frame_types": ["collector_started", "domain_delta", "gap"],
      "sequence_count": 4200, "domains": ["clock", "divisions"],
      "delta_domains": ["divisions"], "gap_domains": [],
      "first_sequence": 10, "last_sequence": 4209 }
  ],

  "manifest_digest": "…",            // sha256 over the CANONICAL manifest (see §3)
  "signature": null                  // OPTIONAL detached Ed25519 over manifest_digest
}
```

* **`build` and `mod_identity` are authoritative at the manifest level**, and MUST
  be consistent with the snapshots: the reader cross-checks `mod_identity.modset_id`
  against a representative snapshot's `provenance.mod.modset_id` and the build hash
  against `save_version`/capture provenance where available; a disagreement is a
  rejection (a manifest must not be able to *lie* about identity). `[Codex b]`
* **`entries[*].bytes` is the decompressed size** and the reader enforces
  `bytes == ZIP-uncompressed-size` per entry — a primary zip-bomb guard. `[Codex b]`
* `mod_identity` / `build.exe_sha256` may legitimately be `null` (capture could not
  read them) — that is *measured absence*, distinct from a wrong value, and the
  reader surfaces it without fabricating.

---

## 3. Integrity model

Two independent layers, by intent:

1. **Corruption detection (always on):**
   * `entries[i].sha256` = SHA-256 of entry *i*'s raw decompressed bytes.
   * `manifest_digest` = SHA-256 of the **canonical serialization of the manifest
     with `manifest_digest` and `signature` removed**. The canonical form is
     **RFC-8785-style canonical JSON**: keys sorted lexicographically, minimal
     whitespace, no insignificant differences, duplicate keys rejected. This binds
     `schema_version`, `build`, `mod_identity`, `required_features`, every
     `entries[*]` triple (`path`,`sha256`,`bytes`) **and** the entry count — so a
     dropped/added/re-ordered entry, a flipped build flag, or a swapped mod id all
     change the digest. `[Codex b]`
   * Canonicalization uses the **parsed object**, not string concatenation, so a
     `/` or newline inside a path can never create an ambiguity that a
     delimiter-joined hash would. `[Codex b]`

2. **Tamper-PROOFING (optional, recommended for distribution):**
   * `signature` = detached **Ed25519** signature over the 32-byte
     `manifest_digest`, plus the signer's public key (or a key id resolvable out of
     band). Checksums alone only catch *accidental* corruption: an attacker can
     edit a snapshot, fix its entry hash, and recompute `manifest_digest`. A
     signature is what makes the file genuinely tamper-evident against a motivated
     party. v1 readers treat a present-and-valid signature as "verified-signed",
     a present-and-invalid signature as **rejection**, and an absent signature as
     "integrity-checked, unsigned" (allowed, surfaced to the UI). `[Codex b]`

> Honesty note (carried from the Codex round): without a signature this format is
> **checksum-protected, not tamper-proof**. The spec makes signing a first-class,
> drop-in slot so a distribution channel can require it; the baseline reader still
> guarantees that *accidental* corruption (bad sector, truncated download, partial
> write) is always caught.

---

## 4. Reader algorithm (normative)

Verification is **staged**: structural validation and an incremental CRC-32 pass
over every payload happen up front without archive-wide RAM residency; expensive
per-entry SHA-256 authentication happens lazily when bytes are requested.
`[Codex d]`

### 4.1 Open (up-front, O(total payload), bounded memory)

1. Open the ZIP through one pinned file handle; record the handle's observed
   length and reject a concurrent length change. Parse central-directory records
   incrementally, retaining only bounded entry metadata, and reject a
   malformed/truncated archive.
2. Reject **duplicate entry names**. `[Codex b]`
3. Validate **every** entry path against §4.2. Reject on any violation. `[Codex b]`
4. Before decoding or trusting the manifest, incrementally decompress every entry
   through its declared-size bound and re-derive CRC-32. Any CRC, decompressed-size,
   compression-stream, or data-descriptor mismatch is an open-time refusal.
5. Locate `manifest.json`. **If absent → this is an unfinalized/recovery-only file;
   the normal reader REFUSES it** (a separate recovery tool, §6, may rebuild a
   manifest and mark the result `recovered/unverified`). `[Codex c]`
6. Parse the manifest; reject if `format != "hoi4replay"` or
   `container_version >` reader max.
7. Reject if any `required_features[*]` is not in the reader's known set. `[Codex b]`
8. Recompute `manifest_digest` over the canonical manifest; reject on mismatch.
9. If `signature` present: verify Ed25519; reject on failure. If absent: continue,
   flag `unsigned`.
10. Enforce structural caps (zip-bomb): entry count ≤ cap, each
   `entries[*].bytes ≤ per-entry cap`, Σ`bytes ≤ total cap`, and each entry's
   declared `bytes` == the ZIP's uncompressed size for that entry. `[Codex b]`
11. Verify + parse `payload/<game>/index.json` **now** (hash it against its entry,
    then JSON-parse) — it is the spine the app reads first.
12. Verify + parse `timeline.json` now if present.
13. Cross-check identity consistency (§2): `mod_identity` vs a snapshot's
    `provenance.mod`; build hash vs provenance. Reject on contradiction. `[Codex b]`

### 4.2 Path rules (reject any of these)

* absolute path, drive letter (`C:`), UNC (`//`), leading `/`;
* any component equal to `.` or `..` (zip-slip); `[Codex b]`
* a backslash `\` anywhere (ambiguous on Windows); `[Codex b]`
* any path that is not already Unicode NFC;
* two entries whose NFC Unicode caseless collision keys are equal, including
  multi-codepoint/contextual folds such as `Straße`/`STRASSE` and final sigma
  (would collide when extracted on Windows/macOS); `[Codex b]`
* a Windows reserved device name component (`CON`, `NUL`, `COM1`…);
* a path not under `payload/` (except the single top-level `manifest.json`).

### 4.3 Per-snapshot read (lazy, O(one snapshot))

On a request for hour *h* (or file *f*):

1. Resolve the entry from the **index** (`file` is authority), NOT by synthesizing
   a name. `[Codex a]`
2. Read the entry's decompressed bytes (bounded by its declared `bytes`).
3. SHA-256 the bytes; compare to the entry's `sha256`. Mismatch → error (never
   expose unverified snapshot data). `[Codex d]`
4. Transition that exact manifest entry record from `Pending` to `Verified` and
   require `Verified` at the byte-exposure boundary. A mismatch leaves it
   `Pending`. Immutable byte-buffer archives may reuse that transition; a
   file-backed reader re-authenticates every exposure because a pinned handle
   guarantees file identity, not immutable contents.
5. JSON-parse the verified bytes into the existing `Snapshot` value (raw domains
   stay `serde_json::Value`; the Absent/null/0 trichotomy is preserved by the
   existing reader and asserted by tests).

A background task MAY full-verify all entries after open; it never gates first
paint.

---

## 5. Mapping to / from the store layout (loss-less)

| Store directory                                   | Container path                                  |
|---------------------------------------------------|-------------------------------------------------|
| `<root>/<game_id>/index.json`                     | `payload/<game_id>/index.json` (raw bytes)      |
| `<root>/<game_id>/<file>.snapshot.json`           | `payload/<game_id>/<file>.snapshot.json` (raw)  |
| `<root>/<game_id>/timeline.json`                  | `payload/<game_id>/timeline.json` (raw)         |
| `<session-run>/collector_frames.jsonl`            | `payload/<game_id>/collector_raw/collector_frames.jsonl` (raw, optional) |
| `<root>/<game_id>/_continuity/…`, `*.lock`        | **excluded** (machine-local; §1)                |
| `<root>/<game_id>/_noncanonical/…`                | **excluded** in v1 (no hour_index; §1)          |

**Pack** = for each file in `index.snapshots[*].file` (+ `index.json` + optional
`timeline.json` + optional `collector_frames.jsonl`): read its bytes, store them
at `payload/<game>/<same-name>` for store files or
`payload/<game>/collector_raw/collector_frames.jsonl` for the raw stream, record
`sha256`+`bytes`+`role` in `entries`, and add `raw_streams[]` discovery metadata
when the raw stream exists. Then build `manifest_digest`, write `manifest.json`
last.

**Unpack/serve** = §4. To materialize a directory store again (for tools that want
files on disk), write each entry's verified bytes to `<out>/<game>/<name>` — the
bytes are identical to the originals, so `index.json`, every snapshot, and
`timeline.json` are **byte-for-byte** what was captured. The trichotomy, the
rollback filenames, and compact-JSON formatting all survive because nothing was
re-encoded. `[Codex a/e]`

---

## 6. Streaming writer & partial recordings (design; built by the capture owner)

The recorder streams each hour into the file as the capture walk produces it, so a
recording can be ended anytime and the finalized result is valid. To stay safe
under a crash AND keep ZIP semantics legal (a ZIP cannot legally "refresh"
`index.json` without a duplicate entry), the writer uses a **work directory + a
running sidecar**, then a single atomic finalize: `[Codex c]`

```
<name>.battleplan.part/                  (work dir; never a half-written ZIP)
├── payload/<game>/<file>.snapshot.json   ← each hour: atomic-write one file
├── payload/<game>/index.json             ← rewritten atomically each hour
├── payload/<game>/timeline.json          ← rewritten atomically each hour
└── entries.jsonl                         ← append-only: one {path,sha256,bytes} per finalized file
```

* Each hour: atomic-write the snapshot file, atomic-rewrite `index.json` (and
  `timeline.json`), then **append** the snapshot's `{path,sha256,bytes}` line to
  `entries.jsonl` (fsync). The JSONL log is append-only, so a crash mid-record
  leaves a prefix of complete lines — never a torn structure. `[Codex c]`
* **Finalize** (record stopped, or "save now"): read `entries.jsonl` (+ the current
  `index.json`/`timeline.json`), build `manifest.json` with `manifest_digest`
  (sign if a key is configured), then create the ZIP into
  `<name>.battleplan.tmp`, fsync, and `os.replace` → `<name>.battleplan`.
  Writing the manifest LAST and renaming atomically means the published file either
  doesn't exist or is a fully valid, manifest-bearing container — never an
  in-between. `[Codex c]`
* **Interrupted record** (process died before finalize): the `.part/` work dir
  survives with a valid `entries.jsonl` prefix. The **recovery tool** (`§6` of the
  reader plan) finalizes it into a valid `.battleplan` that contains exactly the
  hours captured before the crash — so an early-ended recording is still a complete,
  valid replay of what was seen. A bare `.part/` is never opened as a replay. `[Codex c]`

This keeps capture itself writing to the **existing durable directory store** (no
new fragile ZIP-append path in the hot loop); the container is produced by a small
pack step that can run incrementally (rebuild the manifest from the sidecar) or
once at the end. `[Codex c]`

---

## 7. Versioning & forward compatibility

* `container_version` gates the *physical* format. A reader refuses a higher one.
* `required_features` / `optional_features` gate *semantics within* a version: a
  new capability that changes how the payload must be interpreted is listed in
  `required_features`, so an old reader fails closed instead of mis-rendering.
  A purely additive capability goes in `optional_features` and old readers ignore
  it. `[Codex b]`
* Unknown manifest keys are preserved (the reader keeps a raw copy) so a
  pack→unpack→pack pipeline never drops fields it didn't understand.
* `schema_version` is the snapshot payload schema; the reader passes it through to
  the bind layer (which already tolerates schema growth by keeping domains raw).

# R4 — Patch-day robustness runbook

**Purpose (plan §4 R4):** the extractor's offsets/vtables are derived against a specific
`hoi4.exe` build. A game patch can move them. This is the **fail-closed preparedness** checklist
to re-validate (and, where needed, re-derive) after a the game update. It is NOT a claim of
forward-compatibility over unseen versions — the first capture on a new build MUST pass this
runbook **before** that version is declared compatible. Every step is fail-closed: if a check
can't prove green, the version stays *unsupported*, not *assumed-good*.

**Completion status:** See `extractor/documents/MASTER_EXTRACTION_CONTRACT.md`; this runbook
defines the procedure and does not certify any build by itself.

**Preconditions:** Windows; a save loadable on the new build; the analyzed Ghidra substrate
available; `rakaly` for the melt oracle. Run from `extractor/native/`.

---

## Step 0 — record the new build identity
Capture the new `hoi4.exe` sha256 + version string and file it in the run log. The manifest's
`build_profile` id must change; a capture whose build id doesn't match the derived profile is
already fail-closed (readers refuse an unknown build).

## Step 1 — RTTI/vtable re-derivation (the structural half — already self-proven)
```
# re-derive against the NEW exe: prints the updated constants block + a diff report
python scripts/rtti_rederive.py --rederive "<new hoi4.exe>"
```
- Re-derives every RTTI-named vtable RVA from the new image by matching each class's decorated
  RTTI name (+ COL offset to disambiguate multiple-inheritance vtables). This half is
  *self-proven*: a moved-but-present class is re-anchored automatically; a **removed/renamed or
  ambiguous** class is listed **UNRESOLVED, never guessed** (fail-closed, named).
- The diff report names every RVA that moved → its dependent domain's absolute offsets are
  suspect → Step 2 must re-validate that domain. (Sanity self-proof against the CURRENT exe is
  `--verify`, which asserts byte-for-byte equality with `build_profile.rs` and exits 1 on any
  mismatch — run it first to confirm the baseline before pointing at the new build.)
- Any UNRESOLVED needle vtable = that domain fails closed (SOURCE_UNAVAILABLE); do not ship the
  version as compatible until the class is re-located and the manifest updated (`--derive-map`).

## Step 2 — offset sentinel checks (the field-offset half — needs a save)
For each domain whose vtable moved (or, for a major patch, ALL domains), run a short live
capture and check the per-domain sentinels:
```
# single window, all domains, on the new build
bash ~/.claude/hoi4_gate_prompts/run_one_window.sh patchcheck 120 -
```
- Green signal: per-domain `status_flags` has no unexpected `SOURCE_UNAVAILABLE`, row counts are
  plausible, and the `*_sample` attribution units (d47 char-id, d69 vec size@0x3AC, d53
  relation-container) read sane values. A field offset that moved reads garbage → its bound trips
  → fail-closed-with-sample (that's the sample naming the break).
- The **d69 vec header** (begin==manager, cap>=size, size<=256) and **d47 dedup** counters are
  fast structural canaries: if `active_walk_count` is absent (header unreadable) or d47
  `ambiguous_root_rows` spikes, an offset moved.

## Step 3 — d53 shadow-mode referee (offset-vs-scan cross-check)
The d53 relation-flag reader has an independent scan path used purely as a referee:
```
# add to the Step-2 capture command:
    --d53-relation-read-mode shadow
```
- `shadow` reads BOTH the offset path and the scan path and cross-checks them live. Green = the
  offset path agrees with the content scan → the d53 offsets survived the patch. A mismatch names
  the divergent relation → re-derive d53's `rel+0xD0` container offset before trusting d53 on the
  new build.

## Step 4 — bounds re-validation (R2 table)
Re-run the R2 sweep on the new build and confirm no *new* plausibility trips that aren't a bound
the patch legitimately raised (e.g. a patch adding countries past 2048 is the SAME class as the
mod-overhaul case in `R2_VALIDATION_BOUNDS_AUDIT.md` — the tag-ceiling rows). Any trip must be
either (a) a real content increase → raise the bound + keep the fail-sample, or (b) an offset
break → Step 2/3.

## Step 5 — melt-oracle spot check (definitive)
Melt the Step-2 save and cross-check 2–3 domains against ground truth (the d69/d53 precedent):
```
python ../save_oracle/parse_world.py "<save>.hoi4"          # tech-sharing groups + members
python ../save_oracle/parse_characters.py "<save>.hoi4"     # d47 characters
```
- Live domain counts/members == melt oracle → the offsets decode the RIGHT data on the new build.
  This is the same check that validated d69 on D04 (9 groups / 34 members). A count mismatch is a
  hard fail: an offset decodes plausible-but-wrong data.

## Step 6 — declare
Only when Steps 1–5 are green: bump the supported-build list and note the new build id in the
release doc. If ANY step is red, the version is **fail-closed unsupported**; captures from it
must not be admitted as supported production evidence. Any diagnostic attempt remains explicitly
failed/partial rather than being treated as valid capture data.

---

### One-shot command block (copy/paste for a routine patch)
```
cd extractor/native
python scripts/rtti_rederive.py --verify                       # baseline self-proof on current exe
python scripts/rtti_rederive.py --rederive "<new hoi4.exe>"    # prints updated block + which vtables moved
bash ~/.claude/hoi4_gate_prompts/run_one_window.sh patchcheck 120 - --d53-relation-read-mode shadow
python ../save_oracle/parse_world.py "<save>.hoi4"             # compare vs live d69 (groups/members)
# review: per-domain status, *_sample sanity, d53 shadow agreement, oracle counts
```
The vtable half is automatic and self-proven; Steps 2–5 are the human-reviewed gate. A major
patch requires full domain re-validation.

# XUI09 — research state and visible rendering

Disposition: **STATIC LAW + FAIL-CLOSED WIRING COMPLETE — PROMOTION BLOCKED
PENDING H-A29 MACHINE-LIVE RECEIPT AND ROOT CENTRAL VALIDATION**

This packet is bound to `hoi4.exe` 1.19.2.0.a729, image base
`0x140000000`, SHA-256
`870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`.

XUI09 closes the production join from UI10 typed research state into UI09's
opaque admitted technology/doctrine canvases. The join preserves exact authored
node order, completed and in-progress technology subsets, doctrine
completed/current subsets, active slot ordering, progress, ETA, localized title,
technology year, progressbar value, and ahead-of-time tooltip inputs.

Active-slot admission requires `in_progress_live[]`, ordered by measured
`slot_index`. Save-backed `in_progress[]` cannot authenticate A29's effective
producer substitutions and is not an active-slot fallback. Duplicate slots,
completed/current overlap, unknown definitions, missing authored nodes, invalid
progress or ETA, wrong canvas tokens, incomplete doctrine state, or missing,
cross-hour, or mismatched producer provenance reject the whole state. A measured
technology level is the completed-state witness; absent/unobtained levels are
not silently promoted.

The shipped `research_slot_entry` is exactly 500x100 and exposes `title`,
`year`, `eta`, and `research_progressbar`. Production binding uses the exact
prefixed widget names. The title uses XUI06-admitted resolved display text, never
the internal technology token. UI09 retains ownership of rectangles, order,
pan, clip, and pixel canvas admission.

The exact observed-producer authority is the slot-bound
`a729_observed_research_ahead_producer_v2`. Its source-record preimage binds
the research-slot ordinal as well as the technology definition, so a valid
producer receipt cannot be transplanted to another slot. Its branch 0 is a no-display
receipt: when exact days ahead is zero, the producer writes penalty zero and
emits no substitutions; the UI emits `NO_RESEARCH_PENALTY`. The portable row
then carries years `0`, penalty `0`, reduced penalty `0`, and the observed
nonnegative applied reduction. Ahead-of-time display uses branch 1 or 2 and
selects the shipped
`RESEARCH_YEAR_AHEAD_PENALTY` or
`RESEARCH_YEAR_AHEAD_PENALTY_RED` localization branch and passes exact
branch-shaped substitutions to XUI06: `YEARS` and `VALUE` for branch 1, plus
`REDUCED_YEARS` and `RED` for branch 2. It does not format or recompute them. Static
`BASE_YEAR_AHEAD_PENALTY_FACTOR = 2` and `BASE_TECH_COST = 110` are only
base inputs. They cannot reproduce focus reductions or the engine's effective
modifier aggregation.

Extractor row A29 is wired to supply the complete same-hour observed-producer
contract. The exact UI caller, numeric producer, and localization-substitution
consumer are joined so Domain 13 carries live slot identity, `tech_name`, moving
progress numerator, base progressbar denominator, the exact branch-shaped
producer result, and pointer-free callback provenance. Neither the extractor
nor replay may reconstruct that result from the base
factor, template year, or an aggregate memory field. `admit_research_state`
recomputes the pointer-free source-record digest and rejects legacy, save-only,
incomplete, cross-hour, hash-mismatched, or provenance-mismatched active rows.

This wiring does not itself prove a production observation. After integration,
H-A29 must open an ahead active technology's year tooltip during a pinned exact-a729
capture and retain a machine-accepted same-hour branch-1 or branch-2 producer
receipt. A branch-0 receipt alone cannot promote. Root-owned
central validation must then pass before promotion. This is external runtime
activation evidence, not human semantic or reverse-engineering input.

## Mechanical evidence

The following pre-Wave5 hashes and replay-verifier counts remain historical
baseline evidence only. They do not authenticate the new A29 producer hook,
portable provenance, or H-A29 receipt:

- XUI09 replay promotion contract SHA-256:
  `cda034e8b90c336e669b3948cc78f9cd86ef5d711a8635bc201e4e7d73233609`;
- UI09 promotion contract SHA-256:
  `7213d5ec1dfe44661994689984ab1be6d7aef3763e298b15ae89cfcf6930c2fe`;
- UI10 typed topology/state result SHA-256:
  `45353cd6b968ad27dc55494f5691b269087477f44a8f2230b65e7e59a06e811d`;
- XUI09 game-file census SHA-256:
  `a1f76d5a6f199735dd68e59eb852d3e5e507b6c433c26ebbbf7ab28ba701a15a`;
- A29 extractor dossier SHA-256:
  `b201f0fc16cb907a7ec91714128b7cba7686ea26ff19a793f45b96135730d8a8`;
- A29 RE report/evidence SHA-256:
  `1513f7e031604adb3bdf13b0a3474b20e54d5fa5eb1cc5a06b3a47457c711ce5` /
  `9f3e4acbd166fc6cf385e2dad4ec8257e01de7b32a87fe93020c099b277b14d1`;
- canonical replay verifier: 21/21 checks, including `game_ui` 8/8 and
  exact `rt_research_focus` 1/1;
- A29 is extractor-owned and explicitly `replay_success_counted=false`; the
  historical replay-verifier result does not execute or authenticate the new
  extractor producer hook, portable provenance, or H-A29 receipt;
- all 21 canonical-relative manifest bindings recursively validate;
- two byte-identical complete verifier results SHA-256:
  `1326b0e8435605712baa24c7dbfe6547b04d602f2903aa4ff75c0558edf94d11`.
- final mechanically verified XUI09 manifest SHA-256:
  `5ccb3799d8cf8292abcda73521fca30b1b625d9478594faeffd1bc463274f89b`.

Open renderer-law leaves: none. Production activation remains false until the
H-A29 machine-live receipt and root central validation pass. Human semantic
input required: none.

## Ghidra provenance

| Function address | Job ID | Artifact path | Manifest hash |
|---|---|---|---|
| `0x14021CFB0` | `20260717T101224Z-a29-research-ahead-effective-inputs-v1-f8473deb` | `C:\Users\Example\ghidra_agent_exports\20260717T101224Z-a29-research-ahead-effective-inputs-v1-f8473deb\string-xrefs.txt` | `cd33527ef2f8f5d73ac87272d13b3c6219927272875ffe379ec7f4e1f4f8f8fe` |
| `0x140ABE4C0`, `0x140ABEBF0`, `0x140ABEAA0`, `0x1409A7A80`, `0x140EC4C90`, `0x140EC5F90` | `20260717T101342Z-a29-effective-ahead-producer-candidates--f8665dfd` | `C:\Users\Example\ghidra_agent_exports\20260717T101342Z-a29-effective-ahead-producer-candidates--f8665dfd\evidence\_FUNCTION_EVIDENCE_MANIFEST.json` | `89137474708ad3c772421278a73bafdb91cd888f488356a1d3e9e7fd71a836d1` |
| `0x141BCEA60` | `20260717T101636Z-a29-ahead-display-caller-argument-source-82d4572d` | `C:\Users\Example\ghidra_agent_exports\20260717T101636Z-a29-ahead-display-caller-argument-source-82d4572d\evidence\func_141bcea60.decompile.c` | `69993109c2092a239ab9364963e16699676005055fb7b1cab11e77fbb5deeb2e` |
| `0x1424D8760` | `20260717T102238Z-a29-date-reduction-helper-v1-714418d7` | `C:\Users\Example\ghidra_agent_exports\20260717T102238Z-a29-date-reduction-helper-v1-714418d7\evidence\func_1424d8760.decompile.c` | `5f4965af25586d61f003db56663a96ee0ba6a1838eeba8d9abf03745381d36e8` |
| `0x141BCEA60`, `0x140ABEAA0`, `0x140ABE4C0`, `0x140ABEBF0`, `0x142230F20` | `20260720T124311Z-w5-a29-effective-producer-roots-e50d79d0` | `<local-path>` | `82884a9a122f95298d5068bdb68590806346c2c1ac274ca3b56889bb0e5abcc2` |

The full producer law, all artifact hashes, and incidental findings are frozen
in `extractor/documents/dossiers/A29_RE_REPORT.md` and
`A29_RE_EVIDENCE.json`.

The Wave5 read-only sentinel/string dump
`20260720T130641Z-w5-a29-hook-sentinels-and-substitution-i-9016d012`
has artifact SHA-256
`3fbca6726f09499e6cf1961969ae4154b9c265f8e5c688f8d939d8397387cee2`.
It binds the three hook sentinels and the reduced/unreduced localization keys and
substitution names; it is static authority, not a substitute for H-A29.

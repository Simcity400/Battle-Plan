# XUI09 — research state and visible rendering

Disposition: **PROVEN — READY TO PROMOTE (fail-closed producer admission)**

This packet is bound to `hoi4.exe` 1.19.2.0.a729, image base
`0x140000000`, SHA-256
`870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`.

XUI09 closes the production join from UI10 typed research state into UI09's
opaque admitted technology/doctrine canvases. The join preserves exact authored
node order, completed and in-progress technology subsets, doctrine
completed/current subsets, active slot ordering, progress, ETA, localized title,
technology year, progressbar value, and ahead-of-time tooltip inputs.

Live `in_progress_live[]` rows take precedence and are ordered by measured
`slot_index`; the save-backed `in_progress[]` fallback uses array ordinal.
Duplicate slots, completed/current overlap, unknown definitions, missing authored
nodes, invalid progress or ETA, wrong canvas tokens, incomplete doctrine state,
or missing provenance reject the whole state. A measured technology level is the
completed-state witness; absent/unobtained levels are not silently promoted.

The shipped `research_slot_entry` is exactly 500x100 and exposes `title`,
`year`, `eta`, and `research_progressbar`. Production binding uses the exact
prefixed widget names. The title uses XUI06-admitted resolved display text, never
the internal technology token. UI09 retains ownership of rectangles, order,
pan, clip, and pixel canvas admission.

Ahead-of-time display selects the shipped
`RESEARCH_YEAR_AHEAD_PENALTY` or
`RESEARCH_YEAR_AHEAD_PENALTY_RED` localization branch and passes exact
`YEARS`, `VALUE`, `REDUCED_YEARS`, and `RED` substitutions to XUI06.
It does not format or recompute them. Static
`BASE_YEAR_AHEAD_PENALTY_FACTOR = 2` and `BASE_TECH_COST = 110` are only
base inputs. They cannot reproduce focus reductions or the engine's effective
modifier aggregation.

Extractor row A29 now supplies the complete same-tick producer contract. Domain
13 carries exact live slot identity and `tech_name`, moving progress numerator,
base progressbar denominator, and the engine-derived four-value effective ahead
quartet. `admit_research_state` still rejects legacy or incomplete rows. No
capture is required.

## Mechanical evidence

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
- A29 is an extractor-owned, already-proven producer observation and is
  explicitly marked `replay_success_counted=false`; the replay verifier does
  not execute or score extractor tests;
- all 21 canonical-relative manifest bindings recursively validate;
- two byte-identical complete verifier results SHA-256:
  `1326b0e8435605712baa24c7dbfe6547b04d602f2903aa4ff75c0558edf94d11`.
- final mechanically verified XUI09 manifest SHA-256:
  `5ccb3799d8cf8292abcda73521fca30b1b625d9478594faeffd1bc463274f89b`.

Open renderer leaves: none. Human capture request: none.

## Ghidra provenance

| Function address | Job ID | Artifact path | Manifest hash |
|---|---|---|---|
| `0x14021CFB0` | `20260717T101224Z-a29-research-ahead-effective-inputs-v1-f8473deb` | `C:\Users\Example\ghidra_agent_exports\20260717T101224Z-a29-research-ahead-effective-inputs-v1-f8473deb\string-xrefs.txt` | `cd33527ef2f8f5d73ac87272d13b3c6219927272875ffe379ec7f4e1f4f8f8fe` |
| `0x140ABE4C0`, `0x140ABEBF0`, `0x140ABEAA0`, `0x1409A7A80`, `0x140EC4C90`, `0x140EC5F90` | `20260717T101342Z-a29-effective-ahead-producer-candidates--f8665dfd` | `C:\Users\Example\ghidra_agent_exports\20260717T101342Z-a29-effective-ahead-producer-candidates--f8665dfd\evidence\_FUNCTION_EVIDENCE_MANIFEST.json` | `89137474708ad3c772421278a73bafdb91cd888f488356a1d3e9e7fd71a836d1` |
| `0x141BCEA60` | `20260717T101636Z-a29-ahead-display-caller-argument-source-82d4572d` | `C:\Users\Example\ghidra_agent_exports\20260717T101636Z-a29-ahead-display-caller-argument-source-82d4572d\evidence\func_141bcea60.decompile.c` | `69993109c2092a239ab9364963e16699676005055fb7b1cab11e77fbb5deeb2e` |
| `0x1424D8760` | `20260717T102238Z-a29-date-reduction-helper-v1-714418d7` | `C:\Users\Example\ghidra_agent_exports\20260717T102238Z-a29-date-reduction-helper-v1-714418d7\evidence\func_1424d8760.decompile.c` | `5f4965af25586d61f003db56663a96ee0ba6a1838eeba8d9abf03745381d36e8` |

The full producer law, all artifact hashes, and incidental findings are frozen
in `extractor/documents/dossiers/A29_RE_REPORT.md` and
`A29_RE_EVIDENCE.json`.

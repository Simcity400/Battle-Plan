# XUI11 — politics detail data and tooltip-input law

Status: **REPLAY-OWNED PROVEN — promotion-ready fail-closed binding;
DATA_BLOCKED only by external resolved tooltip inputs**

This packet is `XUI11`, not `UI11`. The existing `UI11` packet remains the
frozen victory-point/capital map-furniture law and was not modified.

## Exact result

The politics-detail surface is a four-source composition with strict
precedence:

1. recorded live politics state;
2. save-backed state only where the live leaf is absent;
3. effective mod-over-vanilla idea, character, localisation, GUI and GFX
   definitions;
4. an unavailable badge when a runtime-computed input was not recorded.

Measured order is normative for active ideas, spirits and appointed advisors.
The replay must classify tokens only through the effective `common/ideas`
contract. `national_spirits` is the `category=country` subset;
`ideas_other` is the ordered non-law subset. The complete measured `laws`
object is preserved because current live keys (`economy`, `trade_laws`,
`mobilization_laws`) are not the same names as the save convenience aliases
(`economy`, `trade`, `conscription`). Party values are used verbatim—no
normalization, rebalancing or invented missing ideology row.

Appointed advisor identity is the recorded active advisor idea token. The
static join supplies exact `{idea_token, slot, ledger, character}` rows and the
recorded slot/ledger maps supply occupied grouping. GUI grid width is never
treated as runtime slot capacity, vacancy or eligibility.

## Effective static corpus

The current effective resolver output contains:

- 6,522 idea tokens in 24 categories from 222/222 files;
- 6,522/6,522 safe idea localisation joins, zero omitted;
- 4,298 ideas with non-empty modifier maps and 1,014 with research bonuses;
- 3,018 characters and 1,800 unique modern advisor tokens;
- six advisor slot families and four ledger families;
- 139,950 effective English localisation keys from 238/238 files across the
  vanilla and `AMSB - DEV` layers.

No mod override exists for `interface/countrypoliticsview.gui` or `.gfx`, so
the installed files are effective. Their SHA-256 values are respectively
`811051a83ef087b87be4de9b226fb831f6d6d8f20cd440371619501b5710c183`
and `83d3e5b5b88c373921c732d627fc6adce61f46e8ea1c3c866ef32a99bcc3932f`.
The verifier pins the politics view, party grid, national-spirit grids, idea
category/grid, political-ideas window, modifier container, and all eleven
declared advisor-slot sprites.

## Retained live evidence

The retained production native manifest contains 195 `politics_live` records,
1,559 politics-country rows and 6,236 popularity rows, plus 195
`politics_residuals` records and 423 residual-country rows. Both domains report
zero problems. Earlier strict classification and advisor proof claims remain
documented in the live-capture README; their original session directories are
not present in this checkout, so this packet does not pretend those prose
claims are independently rehashed artifacts.

The complete 214-RDC retained-store manifest was also checked. No capture is
explicitly named for politics/advisor/minister/national-spirit work. That is an
exhaustive filename/path result, not OCR or visual-content proof, and is recorded
with that limitation.

## Typed replay promotion fix

`PoliticsDetailView` previously exposed only the old A24 subset and silently
dropped already-recorded classification and residual fields. It now preserves
the full politics detail contract with `Leaf` trichotomy:

- stability, war support, political/command power and ruling party;
- party popularities and ordered `ideas`, `ideas_all`, `ideas_other`, and
  `national_spirits`;
- `idea_groups`, `advisor_ideas`, and the complete generic law map;
- idea/party counts, election cadence/date/allowed state, and confidence;
- appointed advisor tokens, typed detail rows, slot maps and ledger maps;
- command ability tokens.

Wrong-typed token maps fail the entire leaf closed as `Unobtained`; empty
measured maps/lists remain real. The old convenience law leaves remain for
compatibility, while the generic law map prevents current live keys from being
dropped.

The positive typed-contract test and the packed `fx_country_management`
round-trip both pass.

## Computed tooltip inputs

Authored tooltip rows are promotion-ready now: resolved name/description,
picture, authored modifier/research-bonus maps, base cost/removal cost, advisor
slot/ledger/traits/cost/fireability, and the recorded active politics state.

Runtime truth is a different input class. Exact dynamic lines require recorded
resolved values for trigger outcomes/reason trees, effective hire/removal cost,
aggregated conditional modifier rows, runtime slot capacity/vacancy/eligibility,
timed/dynamic modifier value/source/expiry, and any displayed popularity-change
breakdown. The current replay schema has no `politics.tooltip_inputs` or
equivalent resolved-computation leaf. Static `has_allowed`/`has_available`
presence is not a runtime truth value.

This is a complete disposition, not a guessed or partial tooltip law: static
authored rows render with source identity; a dynamic line renders only from a
recorded resolved input carrying frame hour, country, target token/slot, typed
value, source and availability state. Otherwise it is explicitly unavailable.
The replay layer must not evaluate Clausewitz triggers or aggregate modifiers.
A visible tooltip capture is useful visual validation but cannot substitute for
the missing semantic input, so no new human capture is required to promote this
data/rendering contract.

## Incidental finding

The legacy Python `politics_live.py`, `politics_residuals_live.py`, and residual
offset draft are build-gated to executable SHA-256
`16022851eaa571728aa3a0b5fd5f990a56da57e2249a4c1b9cfb30336f59b120`,
not the current a729 authority SHA. Their historical semantic/save validation
is corroboration only; their offsets are not promoted as a729 binary evidence.

The canonical replay verifier passes 99/99 checks twice, including the targeted
typed politics contract and exact `rt_country_management` round-trip. The
external `politics.tooltip_inputs` producer is recorded separately with
`replay_success_counted=false`; no extractor package is executed or scored by
the replay verifier.

## Provenance and frozen tuple

Authority is `hoi4.exe 1.19.2.0.a729`, image base `0x140000000`, SHA-256
`870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`.

This is a FILE/data/replay-binding packet. Ghidra function address, job ID and
Ghidra artifact path are **N/A**; no new binary claim or Ghidra operation was
used. The normative artifact is
`replay_interface/re_evidence/rendering_laws/XUI11/production_contract.json`.

- Production contract SHA-256:
  `24726b3dae839fd2a35760e5ce30d167355d7fb219650ffc341d65e3e001779e`.
- Verifier SHA-256:
  `a2c2da529148ae70df13681ebebd03bed0544e0cd53684ed90284a833998aa01`.
- Verification SHA-256:
  `3c038a9194914f9a5646954b51e6d212210a916d9b0e5cd7b4037fbccbdb437d`
  (`99/99`, byte-identical across consecutive runs).
- Evidence manifest SHA-256:
  `ea5602d65f8c68a4552982f342b2362bf396543003deadfeed446087aff97a4e`
  (19/19 canonical relative bindings, zero failures).
- Typed source SHA-256 (`world.rs`):
  `0d16aa03c664213e0e35a0bfa31488f6cec4cd3f15decfc47f55b21b0400450f`.
- Public export SHA-256 (`lib.rs`):
  `9e0d44fe54e8120e55ccfd3cd5f5fee649cefb6bffe555fabe1ed14b5bd7bde4`.

No extractor, collector, existing `UI11`, shared integration artifact, or
`PROGRAM_STATUS.md` was modified.

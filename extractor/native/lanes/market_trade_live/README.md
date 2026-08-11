# market_trade_live
> Active lane note: this README is the only active lane-local documentation. Historical plans, proof reports, blocker notes, and registration packets for this lane are archived under ../../../documents/archive/native_lanes/market_trade_live/. Current extraction completion status lives in ../../../documents/MASTER_EXTRACTION_CONTRACT.md.


Lane-local decoder, gates, probes, and registration plan for compact domain `43`
(`market_trade_live`).

## Exact XUI15 activation trailer

`MTRA` v1 is a 32-byte independent country-trade window lifecycle carrier:

- flags bit 0: active; bit 1: exact authored root;
- `u32 country_tag_db_id` (zero iff closed);
- `u64 observation_sequence` from the Open/Close hook lifecycle;
- root kind `1` (`countrytradeview`);
- exact Open RVA `0x015EC520`.

The producer hooks exact-a729 `CCountryTradeView::Open` and `Close`, validates
their pinned instruction sentinels and the view vtable, resolves the displayed
country through the engine consumer path, and joins that pointer to the same
current exact country census before emission. No selected POV or semantic d43
presence is an activation substitute. The lane decoder rejects malformed or
duplicate MTRA trailers and lifecycle sequence regression. For acquisition,
`market_trade_gate --require-x15-activation-transition` requires an exact
closed-active-closed sequence; `Invoke-X15AutonomousTradeActions.ps1` drives
the authored `trade_button` shortcut `r` without coordinate tuning.

Status: `NOT COMPLETE` for full trade economy. The lane now has production
purchase-contract core capture, dynamic D-tag buyer resolution through shared
commit `1f38b65d1`, omitted-candidate classification proof, and a lane-local
`MTD1` purchase-contract detail trailer that captures price/amount. The remaining
full trade-economy residuals are exact source/fixture blockers, not accepted
omissions.

## Validated Build Roots

Current live build/profile: `870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`.

Validated roots and offsets:

- `CPurchaseContract` RTTI decorated name:
  `.?AVCPurchaseContract@NInternationalMarket@@`
- `CPurchaseContract.vtable_rva = 0x2954C70`
- rejected stale `16022851` purchase-contract vtable RVA: `0x2944B80`
- `CEquipmentVariant.vtable_rva = 0x293AFB8`
- purchase contract fields:
  - `id_type = +0x08`, expected `81`
  - `id_id = +0x0C`
  - `equipment_subbuf = +0x38`
  - `seller_tag_db_id = +0x58`
  - `buyer_tag_db_id = +0x5C`
  - `delivery_subbuf = +0x80`
  - `buyer_ptr = +0x128`
- price source:
  `CPurchaseContract+0x38 equipment_subbuf -> CEquipmentVariant payload i32 amount`
- transfer amount source:
  `CPurchaseContract+0x80 delivery_subbuf -> CEquipmentVariant payload i32 amount`

The shared D-tag buyer validator patch is already committed as `1f38b65d1`. The
proof run `extractor/native/runs/market_trade_live_dtag_shared_20260701T174028/`
decoded 71 domain-43 data records, 71 unique game hours, 3,555 emitted rows,
3,621 observed rows, 66 omitted candidates, zero partial rows, zero skipped
hours, and a verified `.battleplan` container.

## Compact Payload

Shared v1 payload:

- domain id: `43`
- compact stream record magic: `H4CR`
- record version: `1`
- record header length: `32`
- data payload header length: `48`
- row length: `48`
- fixed scale: `100000`

Payload header fields:

- `u64 elapsed_wall_ms`
- `u32 row_count`
- `u32 changed_rows`
- `u32 status_flags`
- `u32 observed_contract_rows`
- `u32 emitted_contract_rows`
- `u32 buyer_ptr_valid_rows`
- `u32 variant_valid_rows`
- `u32 amount_valid_rows`
- `u32 partial_field_rows`
- `u32 scale_raw`

Rows:

- `u32 contract_id`
- `u32 contract_type`
- `u32 flags`
- `u32 seller_tag_db_id`
- `u32 buyer_tag_db_id`
- `u32 equipment_variant_id`
- `i64 amount_raw`
- `u32 equipment_subbuf_status`
- `u32 delivery_subbuf_status`
- `u32 buyer_ptr_status`
- `u32 reserved`

The gate allows omitted candidates only through
`observed_contract_rows - emitted_contract_rows`. It fails on partial field rows,
partial/source/worker/bridge status, decode errors, skipped game hours, or
insufficient hourly coverage.

## Optional Detail Trailer

Lane-local `MTD1` trailer, appended after the v1 purchase-contract rows:

- trailer magic: `MTD1`
- schema version: `1`
- trailer header length: `32`
- row length: `64`

Rows are keyed by `contract_id` and carry:

- `flags`
- `price_raw`
- `amount_raw`
- `speed_raw`
- `completed_raw`
- `days`
- `delivery_country_tag_db_id`
- `convoys_assigned`
- `convoys_total`
- `efficiency_raw`
- `factory_cic_progress_raw`
- `equipment_transfer_progress_raw`

Anti-fabrication rule: every nonzero optional field must have its corresponding
valid flag set. The decoder rejects unflagged speed/completed/delivery/progress
values.

Current proof:
`extractor/native/runs/market_trade_live_contract_same_tick_20260701_v3/`.

`purchase_contract_same_tick_probe.py` joined 63 live purchase contracts to the
same-tick `IRE_1943_11_09_17.hoi4` save oracle and emitted 63 detail rows:

- price-valid rows: 63
- amount-valid rows: 63
- speed-valid rows: 63
- completed-valid rows: 63
- days-valid rows: 63
- delivery-country-valid rows: 63
- convoy-valid rows: 25
- efficiency-valid rows: 63
- progress-valid rows: 63

The Rust gate accepted
`purchase_contract_same_tick_stream.bin` with one domain data record, one unique
game hour, zero skipped hours, zero partial rows, and zero parse errors.

The detail stream is also packed and verified as a lane-local replay container:
`extractor/native/runs/market_trade_live_contract_same_tick_20260701_v3/replay_pack/`.
`replay_pack_proof.json` reports one `collector_compact_stream` entry, one
`recorder_manifest` entry, 7,200 verified stream bytes, and a byte-equal stream
readback from `native_session.battleplan`. Independent `pack_replay verify`
returns `verify: OK` for `market_trade_live_contract_same_tick_v3`.

Validated current-build direct offsets:

- `CPurchaseContract+0xC0`: speed, 63/63 exact.
- `CPurchaseContract+0x25C`: completed, 63/63 exact.
- `CPurchaseContract+0x1D0`: days, 63/63 exact.
- `CPurchaseContract+0x5C`: delivery-country tag db id, 63/63 exact.
- `CPurchaseContract+0x118/+0x11C`: assigned/total convoys, 25/25 exact.
- `CPurchaseContract+0x1C0`: efficiency raw, 63/63 exact.
- `CPurchaseContract+0x1E0`: factory CIC progress raw, 63/63 exact.
- `CPurchaseContract+0x1E8`: equipment-transfer progress raw, 63/63 exact.

## Strategic-resource trailers: `MTRR` and `MTRF`

Two trailers carry a country's strategic resources. **Neither has a fixed
width**: `common/resources` is moddable content, so the resource count is a
field on the wire, discovered from the live `CStrategicResourceDatabase`, and
every reader takes it from the stream.

### `MTRR` -- per-country, per-resource display rows

| offset | width | field |
|---|---|---|
| `+0`  | `[u8; 4]` | magic `MTRR` |
| `+4`  | `u16` | schema version -- `1` fixed-seven, `2` count-prefixed, `3` count-prefixed with identity |
| `+6`  | `u16` | header length, `32` |
| `+8`  | `u16` | row length, `64` |
| `+10` | `u16` | reserved, `0` |
| `+12` | `u32` | row count, `= country_count * resource_count` |
| `+16` | `u32` | country count |
| `+20` | `u32` | **resource count** |
| `+24` | `u32` | display scale, `1` |
| `+28` | `u32` | reserved `0` (v1, v2) / **identity stride `64`** (v3) |

Rows are 64 bytes and unchanged between all three versions. `resource_index` and
`visible_order` are bounded by the header's `resource_count`, not by seven, and
each country's `visible_order` set must be exactly `0..resource_count`.

- **v1** admits `resource_count == 7` and nothing else. A v1 header claiming any
  other count is refused, so a stream cannot be widened by editing the count
  under the old version stamp.
- **v2** admits `1..=MAX_COMPACT_STRATEGIC_RESOURCE_COUNT`.
- **v3** is v2 plus the resource-identity block below. Producers emit v3.

### `MTRR` v3 resource-identity block

`resource_count` entries of `COMPACT_STRATEGIC_RESOURCE_IDENTITY_STRIDE` (64)
bytes each, between the header and the rows. Entry `i` is slot `i`'s resource.

| offset | width | field |
|---|---|---|
| `+0` | `u16` | token length, `1..=MAX_COMPACT_STRATEGIC_RESOURCE_TOKEN_LEN` (62) |
| `+2` | `[u8; 62]` | token bytes, zero-padded |

So a v3 trailer's extent is
`32 + resource_count * 64 + row_count * 64`, and its rows do **not** start at
`header_len`.

Every reader refuses, typed, on: a length outside the bound, a body that is not
UTF-8, a body outside the `common/resources` key charset
(`[A-Za-z0-9_\-.:]`), a non-zero byte in the padding, or a token repeated
across slots. Nothing is truncated or sanitised -- **a shortened token is a
different resource**, and publishing one would fabricate exactly the identity
this block exists to carry.

The token is carried once per **slot**, not once per row. The rows are per
(country, resource), so a per-row column would restate one country's worth of
identity for every other country, and two rows for one slot could then disagree
-- a drift a decoder would have to police. One entry per slot makes that state
unrepresentable, which is the same rule the count already follows: one
statement of the resource set per payload.

It is a token and not a hash because the replay side has two jobs with it:
match resources across recordings, and **display** them. A hash does the first
and cannot do the second.

### `MTRF` -- per-country resource-flow pools

| offset | width | field |
|---|---|---|
| `+0`  | `[u8; 4]` | magic `MTRF` |
| `+4`  | `u16` | schema version -- `1` legacy exported-only, `2` fixed-seven six-kind matrix, `3` count-prefixed six-kind matrix |
| `+6`  | `u16` | header length, `32` |
| `+8`  | `u16` | row length -- `72` in v1/v2, `16 + 8 * resource_count` in v3 |
| `+10` | `u16` | reserved, `0` |
| `+12` | `u32` | row count |
| `+16` | `u32` | exported rows (v1) / country count (v2, v3) |
| `+20` | `u32` | scale, `100000` |
| `+24` | `u32` | **resource mask `0x7F` (v1, v2) / resource count (v3)** |
| `+28` | `u32` | reserved `0` (v1) / kind mask `0x3F` (v2, v3) |

Row:

| offset | width | field |
|---|---|---|
| `+0`  | `u32` | `tag_db_id` |
| `+4`  | `u32` | `flow_kind`, `1..=6` |
| `+8`  | `u32` | resource mask `0x7F` (v1, v2) / resource count echo (v3) |
| `+12` | `u32` | reserved, `0` |
| `+16` | `i64 * resource_count` | per-resource values |

The row's `+8` must equal the header's `+24`, so the header and the rows cannot
drift apart. Producers emit v3.

### Version policy

**A version never changes meaning; a reading is never inferred.** `MTRF`'s
`+24` is a seven-bit mask in v1/v2 and a count in v3; `MTRR`'s `+28` is reserved
zero in v1/v2 and an identity stride in v3. In both cases the *version field* --
never the value -- decides which. A decoder that meets a version outside its
accepted set refuses the trailer with the version in the reason and decodes no
rows. That is what makes an old reader safe against a new stream: it cannot read
an `MTRF` v3 header under the v2 grammar, compare a small count against `0x7F`,
and mis-size every row; and it cannot read an `MTRR` v3 trailer as though its
rows began at `header_len`, which they do not.

The `MTRR` case has a second, independent guard: v1/v2 require `+28 == 0` and a
v3 header states a stride there, so even a decoder that somehow admitted the
version would refuse on the header shape rather than read identity bytes as
rows.

v1 and v2 stay readable under exactly the validation they shipped with. No
compact stream carrying these trailers exists on disk (the `.battleplan`
fixtures are fold output, not compact records), so this is insurance for
retained streams rather than a data obligation.

### Admission bound

`compact_contract::build_profile::MAX_COMPACT_STRATEGIC_RESOURCE_COUNT` is the
one ceiling the producer and every consumer apply. It is the collector's own
already-shipped structural guard on the `CStrategicResourceDatabase` item count
at `+0x34`, promoted to a shared constant so no side invents its own; the
constant carries its full provenance, including that it is a structural guard
rather than an engine-proven maximum. A count outside `1..=MAX` is refused
before anything is sized from it, so `row_count * resource_count * 8` cannot be
driven by two stream-supplied `u32`s.

### Resource identity: where it comes from

The trailers carry each resource's SLOT, its `descriptor_db_index`, and -- from
`MTRR` v3 -- its TOKEN. The producer reads it from the live descriptor:

- `CStrategicResource + 0x08` is a `u32` Clausewitz lexer token id, the id the
  resource's `common/resources` key was interned to.
- `CStrategicResourceDatabase + 0x40` is a parallel `u32` key vector, one
  shorter than the item vector because `items[0]` is the null object. The
  producer requires `keys[db_index - 1] == items[db_index]->token_id`, so a
  shifted or half-read items array cannot put a real name on the wrong object.
- The text is `*(0x35C8B80) + token_id * 0x20` read as an MSVC `std::string`,
  admitted only while `token_id <= *(i32*)0x35C8B54` -- the exact predicate
  `CPdxLexer::GetTokenString` applies.

Writer and reader chains, with function ids and instruction-level citations:
`extractor/native/runs/resource_identity/re_descriptor_token.md`.

The passive read does not take the lexer's read lock (`0x1435c8b78`), which the
engine's own reader does. The producer closes that window by re-reading the
table base and highest-admitted id after resolving every name and refusing the
whole set if either moved, rather than by pretending the window is not there.

**What the positional table is still for.** A v1/v2 stream, and an `MTRD`-only
payload, state a width and no identity. For those the fold's only name source
remains `MARKET_TRADE_RESOURCE_NAMES`, and that is the honest answer for exactly
one set: the one it describes. Any other set is still refused at ingest with a
reason naming both counts. The refusal was retargeted, not deleted.

## Optional Lend-Lease Mapping Trailer

Lane-local `MTLL` trailer, appended after the v1 purchase-contract rows:

- trailer magic: `MTLL`
- schema version: `1`
- trailer header length: `32`
- row length: `48`

Rows are keyed by `contract_id`, `perspective_tag_db_id`, and `direction`.
Every accepted purchase contract must produce exactly two rows:

- seller perspective: `direction = outgoing`
- buyer perspective: `direction = incoming`

Each row carries counterpart tag, equipment variant, amount, optional price, and
`source_record_kind = purchase_contract`. The decoder rejects duplicate
contract/perspective/direction keys and contracts without exactly one outgoing
and one incoming mapping row.

Current proof:
`extractor/native/runs/market_trade_live_lend_lease_mapping_20260701_v1/`.

`lend_lease_mapping_probe.py` found 63 live purchase-contract rows and emitted
126 MTLL mapping rows:

- outgoing rows: 63
- incoming rows: 63
- amount-valid rows: 126
- price-valid rows: 126
- asymmetric contracts: 0
- duplicate mapping keys: 0
- partial/source/worker/bridge failures: 0

The Rust gate accepted `lend_lease_mapping_stream.bin` with one domain data
record, one unique game hour, zero skipped hours, 63 contract rows, and 126
decoded MTLL rows. The stream is packed and verified as
`market_trade_live_lend_lease_mapping_v1`; independent `pack_replay verify`
returns `verify: OK`.

This is a validated slice for the directed market-contract mapping backbone. It
does not complete full lend-lease replay because market stockpile/automation,
non-market lend-lease roots, and diplomacy relation joins are still separate
blockers.

## Residual Trailer

Lane-local `MTRB` residual trailer, appended after the v1 rows:

- trailer magic: `MTRB`
- schema version: `1`
- trailer header length: `32`
- badge row length: `48`

Current proof:
`extractor/native/runs/market_trade_live_residuals_20260701_v12/`.

The historical v12 residual proof recorded the pre-X3/X15 classification. The
current carrier classification, after the exact resource-trade and D53 joins,
is:

- residual gap rows: 10
- validated slices: 9
- source-blocked residuals: 1
- schema-blocked residuals: 0
- owned-elsewhere context rows: 0
- shared-patch-blocked rows: 0

The v12 replay-pack receipt below remains historical evidence for its captured
bytes; it is not reused as proof of the later X3/X15 carrier joins. Those joins
are admitted by the current strict decoder/fold/typed-replay gates and the
wave-level central validation.

The residual stream is also packed and verified as a lane-local replay
container:
`extractor/native/runs/market_trade_live_residuals_20260701_v12/replay_pack/`.
`replay_pack_proof.json` reports one `collector_compact_stream` entry, one
`recorder_manifest` entry, 624 verified stream bytes, and a byte-equal stream
readback from `native_session.battleplan`. Independent `pack_replay verify`
returns `verify: OK` for `market_trade_live_residuals_v12`.

Classified residuals:

- `purchase_contract_dynamic_tag_buyer_identity`: validated slice.
- `omitted_candidate_classification`: validated slice.
- `purchase_contract_delivery_route_progress`: validated slice.
- `purchase_contract_price_detail`: validated slice.
- `purchase_contract_speed_completed`: validated slice.
- `market_stockpile_sellable_equipment`: `MTS1` carrier/fold/replay route wired
  from the exact static owner chain, but still source-blocked on the fresh live
  owner-chain validation fixture.
- `country_resource_import_export_flows`: validated exact six-pool `MTRF v2`
  slice.
- `license_production_contract_detail`: validated partial slice from ordered
  `PLIC v2` variants joined to the exact current-hour D53 licence lifecycle;
  royalty and factory-cost rates remain omitted.
- `market_access_diplomatic_context`: validated D43/D53 equipment-purchase
  relation join with exact pair, order, and lifecycle witnesses.
- `lend_lease_market_contract_mapping`: validated slice for the core directed
  purchase-contract mapping, not full lend-lease replay.

Additional narrowed residual proofs:

- A residual proof sampled 57 current contract rows; rejected speed/completed/delivery/progress
  emission because candidate offsets are structural only without same-tick
  fixture validation keyed by `contract_id`.
- A same-tick `1943.11.9.17` fixture validates speed (`CPurchaseContract+0xC0`),
  completed (`+0x25C`), days (`+0x1D0`), delivery-country tag id (`+0x5C`),
  and convoys (`+0x118/+0x11C`) for compact `MTD1` emission. The same bounded
  direct/first-level pointer scan accepted no efficiency or progress root.
- A follow-up extends the same fixture to fixed-point raw values and validates efficiency
  (`CPurchaseContract+0x1C0`), factory CIC progress (`+0x1E0`), and
  equipment-transfer progress (`+0x1E8`) for compact `MTD1` emission.
- No accepted RTTI/vtable root was found for the guessed market stockpile/status classes;
  found an unaccepted `CCountry+0x1230` type-70 variant/amount vector candidate
  for 190 countries that needs a same-tick market-stockpile fixture.
- A proof compared live `CCountry+0x1230` rows to parsed `autosave.hoi4` market
  stockpile rows. It is fixture-blocked, not rejected: save `1943.6.1.1`,
  live `1943.6.19.16`, delta `447` game hours.
- A repeat after a newer autosave is still fixture-blocked,
  not rejected: save `1943.9.1.1`, live `1943.9.18.23`, delta `430` game
  hours, 270 live rows, 481 save rows, and 0 exact matches.
- A same-tick fixture rejects `CCountry+0x1230` as the market stockpile root.
  Save and live are both `1943.11.9.17`; the candidate produced 272
  live rows while the save oracle had 494 market-stockpile rows, with
  0 exact matches, 26 amount mismatches, 246 live-only rows, and
  468 save-only rows.
- A bounded probe rejected `CCountry+0x5B8` and nearby `0x540..0x640` compact-vector
  interpretations as a production-ready resource-flow root.
- A broad probe scanned compact-vector shapes across `CCountry+0x400..0x1800`. The only
  stable accepted-like family was `CCountry+0x1218` with resource id sequence
  `[0]`.
- A state-shape probe rejects `CCountry+0x1218` as the full produced/exported/to_export flow root.
  In the same live frame it read 1,081 state roots, 718 state-resource rows,
  467 states with resources, and 0 invalid rows/vectors; 28/28 compared
  countries had more owned-state resource ids than the `+0x1218` candidate.
- A probe found current-build license-production RTTI candidates, including
  `CEquipmentProductionLicense`, `CLicensedProductionConsumer`,
  `CLicensedProductionStatus`, and `CLicenceProductionAccessRelation=0x2949858`;
  found 0 direct `CCountry` owner hits and 15 unaccepted vector hits to
  `CLicensedProductionConsumer` at `CCountry+0x1070/+0x10A0`.
- A vector-candidate probe rejects the exact `CCountry+0x1070/+0x10A0` vector path as a license-detail
  root. The vectors are dense `size=cap=440` country-indexed tables; all hits
  reference one shared `CLicensedProductionConsumer` object, and that object has
  0 nested license/status objects and 0 equipment variant pointers.
- A live proof validates the `MTLL` directed mapping slice from purchase contracts: 63
  core contracts, 126 mapping rows, 63 outgoing, 63 incoming, 126 amount-valid,
  126 price-valid, 0 asymmetric contracts, 0 duplicate keys, and 0
  partial/source/worker/bridge failures.

## Omitted Candidate Proof

Result:

- current `CPurchaseContract.vtable_rva=0x2954C70`
- stale `0x2944B80` vtable rejected for `870ebf`
- candidate rows: 51
- emitted complete rows: 50
- omitted-not-partial rows: 1
- missing-extractable rows: 0
- omission reason: `candidate_vtable_invalid`
- strict country rows: 432
- valid dynamic fallback country rows: 7 (`D01` through `D07`)

So omitted candidates beyond D-tag buyer rows are classified as non-structural in
the current proof. Structural missing extractables still fail through
`partial_field_rows`.

## Local Commands

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\extractor\native\scripts\Invoke-CentralRustValidation.ps1 -Mode Check -Workspace Extractor -Package market_trade_live_lane -CacheMode Iterative
python -m py_compile extractor\native\lanes\market_trade_live\tools\purchase_contract_detail_probe.py extractor\native\lanes\market_trade_live\tools\omitted_candidate_probe.py extractor\native\lanes\market_trade_live\tools\market_stockpile_root_probe.py extractor\native\lanes\market_trade_live\tools\resource_flow_root_probe.py extractor\native\lanes\market_trade_live\tools\resource_flow_broad_probe.py extractor\native\lanes\market_trade_live\tools\resource_flow_state_shape_probe.py extractor\native\lanes\market_trade_live\tools\purchase_contract_field_census.py extractor\native\lanes\market_trade_live\tools\license_detail_root_probe.py extractor\native\lanes\market_trade_live\tools\license_vector_candidate_probe.py extractor\native\lanes\market_trade_live\tools\stockpile_save_compare.py extractor\native\lanes\market_trade_live\tools\lend_lease_mapping_probe.py
python extractor\native\lanes\market_trade_live\tools\market_stockpile_root_probe.py --pid 45880 --out-json extractor\native\runs\market_trade_live_market_stockpile_20260701_v2\market_stockpile_root_probe.json --max-examples 16
python extractor\native\lanes\market_trade_live\tools\resource_flow_root_probe.py --pid 45880 --out-json extractor\native\runs\market_trade_live_resource_flow_20260701_v1\resource_flow_root_probe.json --max-countries 64 --min-countries 24
python extractor\native\lanes\market_trade_live\tools\resource_flow_broad_probe.py --pid 45880 --out-json extractor\native\runs\market_trade_live_resource_flow_broad_20260701_v1\resource_flow_broad_probe.json --max-countries 96 --min-countries 32
python extractor\native\lanes\market_trade_live\tools\resource_flow_state_shape_probe.py --pid 45880 --out-json extractor\native\runs\market_trade_live_resource_flow_state_shape_20260701_v1\resource_flow_state_shape_probe.json --max-countries 96 --min-countries 32
python extractor\native\lanes\market_trade_live\tools\purchase_contract_field_census.py --pid 45880 --out-json extractor\native\runs\market_trade_live_contract_fields_20260701_v1\purchase_contract_field_census.json --max-contracts 96
python save_oracle\parse_market.py --out-dir extractor\native\runs\market_trade_live_save_oracle_20260701_v4 "$env:USERPROFILE\OneDrive\Documents\Paradox Interactive\Hearts of Iron IV\save games\IRE_1943_11_09_17.hoi4"
python extractor\native\lanes\market_trade_live\tools\stockpile_save_compare.py --pid 45880 --market-json extractor\native\runs\market_trade_live_save_oracle_20260701_v4\market.json --out-json extractor\native\runs\market_trade_live_stockpile_fixture_20260701_v4\stockpile_save_compare.json
python extractor\native\lanes\market_trade_live\tools\license_detail_root_probe.py --pid 45880 --out-json extractor\native\runs\market_trade_live_license_detail_20260701_v2\license_detail_root_probe.json --max-rtti 32 --max-strings 64 --max-examples 24
python extractor\native\lanes\market_trade_live\tools\license_vector_candidate_probe.py --pid 45880 --out-json extractor\native\runs\market_trade_live_license_vector_20260701_v1\license_vector_candidate_probe.json --max-examples 96 --max-objects 16
python extractor\native\lanes\market_trade_live\tools\lend_lease_mapping_probe.py --out-bin extractor\native\runs\market_trade_live_lend_lease_mapping_20260701_v1\lend_lease_mapping_stream.bin --out-json extractor\native\runs\market_trade_live_lend_lease_mapping_20260701_v1\lend_lease_mapping_probe.json --examples 16
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\extractor\native\scripts\Invoke-CentralRustValidation.ps1 -Mode Test -Workspace Extractor -Package market_trade_live_lane -CacheMode Iterative
cargo run --manifest-path extractor\native\lanes\market_trade_live\Cargo.toml --bin market_trade_gate -- --stream extractor\native\runs\market_trade_live_dtag_shared_20260701T174028\collector_stream.bin --min-game-hours 12
cargo run --manifest-path extractor\native\lanes\market_trade_live\Cargo.toml --bin market_trade_gate -- --stream extractor\native\runs\market_trade_live_detail_20260701_v1\purchase_contract_detail_stream.bin --min-game-hours 1
cargo run --manifest-path extractor\native\lanes\market_trade_live\Cargo.toml --bin market_trade_gate -- --stream extractor\native\runs\market_trade_live_lend_lease_mapping_20260701_v1\lend_lease_mapping_stream.bin --min-game-hours 1
cargo run --manifest-path extractor\native\lanes\market_trade_live\Cargo.toml --bin residual_report -- --stream-out extractor\native\runs\market_trade_live_residuals_20260701_v10\residual_badges_stream.bin --out extractor\native\runs\market_trade_live_residuals_20260701_v10\residual_badges.json
cargo run --manifest-path extractor\native\lanes\market_trade_live\Cargo.toml --bin market_trade_gate -- --stream extractor\native\runs\market_trade_live_residuals_20260701_v10\residual_badges_stream.bin --min-game-hours 1
cargo run --manifest-path extractor\native\lanes\market_trade_live\Cargo.toml --bin market_trade_replay_pack_proof -- --stream extractor\native\runs\market_trade_live_detail_20260701_v1\purchase_contract_detail_stream.bin --out-dir extractor\native\runs\market_trade_live_detail_20260701_v1\replay_pack --game-id market_trade_live_detail_v1 --min-game-hours 1
cargo run --manifest-path extractor\native\lanes\market_trade_live\Cargo.toml --bin market_trade_replay_pack_proof -- --stream extractor\native\runs\market_trade_live_lend_lease_mapping_20260701_v1\lend_lease_mapping_stream.bin --out-dir extractor\native\runs\market_trade_live_lend_lease_mapping_20260701_v1\replay_pack --game-id market_trade_live_lend_lease_mapping_v1 --min-game-hours 1
cargo run --manifest-path extractor\native\lanes\market_trade_live\Cargo.toml --bin market_trade_replay_pack_proof -- --stream extractor\native\runs\market_trade_live_residuals_20260701_v10\residual_badges_stream.bin --out-dir extractor\native\runs\market_trade_live_residuals_20260701_v10\replay_pack --game-id market_trade_live_residuals_v10 --min-game-hours 1
cargo run --manifest-path extractor\native\lanes\compact_replay_verification_gates\Cargo.toml --bin compact_replay_gate -- --run-dir extractor\native\runs\market_trade_live_dtag_shared_20260701T174028 --require-domain market_trade_live --min-game-hours 12 --write-report extractor\native\runs\market_trade_live_dtag_shared_20260701T174028\compact_gate_report.json
cargo run --manifest-path extractor\native\snapshot\Cargo.toml --bin pack_replay -- verify extractor\native\runs\market_trade_live_dtag_shared_20260701T174028\native_session.battleplan
cargo run --manifest-path extractor\native\snapshot\Cargo.toml --bin pack_replay -- verify extractor\native\runs\market_trade_live_detail_20260701_v1\replay_pack\native_session.battleplan
cargo run --manifest-path extractor\native\snapshot\Cargo.toml --bin pack_replay -- verify extractor\native\runs\market_trade_live_lend_lease_mapping_20260701_v1\replay_pack\native_session.battleplan
cargo run --manifest-path extractor\native\snapshot\Cargo.toml --bin pack_replay -- verify extractor\native\runs\market_trade_live_residuals_20260701_v10\replay_pack\native_session.battleplan
```

## Registration Plan

The lane packet does not edit shared entrypoints. The orchestrator should
serialize shared registration after assigning final sibling record ids or a
domain-43 schema bump.

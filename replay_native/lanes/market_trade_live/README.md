# market_trade_live

Lane-local decoder and gate for compact domain `43` (`market_trade_live`).

This lane currently verifies the production-shaped compact purchase-contract
slice that already exists in the shared collector/recorder. It does not mark the
full trade economy complete. Resource import/export, lend lease, licensing,
market access, and diplomatic trade context remain open until each is captured
through compact DLL/Rust records or exactly blocked/rejected.

## Current validated slice

Build/profile evidence from the current `870ebf` `hoi4.exe` run:

- `CPurchaseContract` RTTI decorated name:
  `.?AVCPurchaseContract@NInternationalMarket@@`
- `CPurchaseContract.vtable_rva = 0x2954C70`
- rejected stale `16022851` purchase-contract vtable RVA: `0x2944B80`
- `CEquipmentVariant` vtable RVA: `0x293AFB8`
- purchase contract fields:
  - `id_type = +0x08`, expected `81`
  - `id_id = +0x0C`
  - `equipment_subbuf = +0x38`
  - `seller_tag_db_id = +0x58`
  - `buyer_tag_db_id = +0x5C`
  - `delivery_subbuf = +0x80`
  - `buyer_ptr = +0x128`

Live proof output:
`replay_native/runs/market_trade_live_probe_20260701_current.json`.

That proof found 50 live `CPurchaseContract` candidates:

- 47 complete purchase-contract rows
- 0 omitted-not-partial rows
- 3 missing-extractable rows
- all 3 missing rows are `candidate_buyer_country_unresolved`
- all 3 unresolved rows use buyer tag `D04`, tag-db id `368`

Follow-up D04 identity proof:
`replay_native/runs/market_trade_live_d04_identity_20260701_current.json`.

The D04 rows have contract id, seller, buyer tag text, equipment variant,
delivery variant, amount, price, and a live buyer pointer. The follow-up proof
removes the previous root blocker:

- `tag_table[368] == "D04"`
- `CGameState.country_table[368] == 0x22BE6A3DEA0`
- each unresolved buyer pointer from the purchase-contract probe equals
  `0x22BE6A3DEA0`
- that pointer has the current `CCountry` vtable `0x7FF6AD7C0E90`
- the old object-id validator rejected it only because `CCountry + 0x08` reads
  `447`, not `368`
- table census: 439 country-vtable slots scanned; 432 pass object-id equality;
  exactly 7 `D01`-`D07` dynamic tags are valid slot-index fallback candidates

So the D04 buyer rows are not a hard data wall. They are blocked from shared
emission by an overly strict country identity validator. The bounded fix is to
accept `country_table[tag_db_id]` when the slot pointer is heap-aligned, has the
validated `CCountry` vtable, the tag-table row has a valid 3-byte tag, and any
contract buyer pointer equals that table slot. Keep `CCountry + 0x08` as the
primary validator for normal rows and record mismatches for diagnostics.

## Compact payload contract

The lane gate decodes the shared compact payload shape:

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

## Local commands

```powershell
cargo test --manifest-path replay_native\lanes\market_trade_live\Cargo.toml
python memory_reader\market_trade_live_probe.py --pid 45880 --examples 32 --out replay_native\runs\market_trade_live_probe_20260701_current.json
cargo run --manifest-path replay_native\lanes\market_trade_live\Cargo.toml --bin market_trade_gate -- --stream replay_native\runs\<run>\collector_stream.bin --min-game-hours 12
```

## Registration patch plan

No shared entrypoints are edited by this lane packet. When the orchestrator
serializes the shared registration patch:

1. keep domain id `43` and name `market_trade_live`;
2. keep one injected collector/coordinator;
3. keep the current compact payload shape above unless a schema bump is assigned;
4. add market-trade gate requirements to require:
   `market_trade_contract_rows`,
   `market_trade_observed_contract_rows`,
   `market_trade_omitted_candidate_rows`,
   `market_trade_buyer_ptr_valid_rows`,
   `market_trade_variant_valid_rows`,
   `market_trade_amount_valid_rows`, and
   `market_trade_partial_field_rows`;
5. fail merge gates when `market_trade_partial_field_rows > 0`;
6. patch the shared country identity builder used by market trade to allow the
   bounded dynamic-tag fallback proved above; this should remove the three D04
   missing-extractable rows without weakening normal country validation;
7. add separate compact record kinds or sibling domains for resource
   import/export, lend lease, licensing, market access, and diplomatic trade
   context once each root is validated.

## Current status

`NOT COMPLETE`.

Backlog reduced: yes. The purchase-contract residual blocker is removed at the
root-evidence level: buyer `D04`, tag-db id `368`, is a valid dynamic-tag table
slot whose pointer matches the purchase-contract buyer pointer. Shared emission
still needs the registration patch above. Full trade economy remains open.

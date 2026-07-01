# country_economy_counters_live

Lane-local production packet for compact domain `58`.

This lane owns country economy counters that are not already covered by
`fuel_live` (`10`), `production_live` (`11`), map/state detail, or
`market_trade_live` (`43`). It does not edit shared collector or recorder
entrypoints.

## Scope

Target master rows:

- fuel capacity/gain/use audit;
- factories/industry capacity: civilian/military/naval totals, damaged, used/free,
  consumer goods;
- military/naval production residual counters: requested factories, efficiency,
  resource shortage, conversion, and line history;
- non-market country resource produced/imported/exported/used/deficit counters.

Excluded owner boundaries:

- domain `43` owns market contracts, market stockpile/sellable equipment,
  licensing, and contract-level trade-factory effects;
- map/detail lanes own state resource deposits;
- politics/characters own law and idea identity; this lane only owns numeric
  country economy effects once a live root is validated.

## Compact Domain 58

The lane-local stream uses normal `H4CR` records and a domain-58 data payload:

- payload header: 52 bytes;
- row length: 48 bytes;
- rows: `counter_id`, status, evidence mask, current support, current domain,
  proposed record kind, current row length, root status, proof status, owner
  domain, exclusion mask, source ref id.

The current packet contains eight counter rows:

- validated slice: fuel capacity/gain/use through domain `10`;
- validated slice: production line active/output counters through domain `11`;
- schema-blocked: requested factory counts;
- source-blocked: factory capacity totals, damaged/free, consumer goods;
- source-blocked: production line efficiency, shortage, conversion, history;
- source-blocked with rejected candidate warning: non-market resource flows;
- owned elsewhere: market contract/factory/trade effects under domain `43`;
- owned elsewhere: state resource deposit context under map/detail.

## Current Proof Commands

```powershell
cargo test --manifest-path replay_native\lanes\country_economy_counters_live\Cargo.toml
cargo run --manifest-path replay_native\lanes\country_economy_counters_live\Cargo.toml --bin counter_report -- --source-stream replay_native\runs\compact_fuel_production_speed5_v1\collector_stream.bin --stream-out replay_native\runs\country_economy_counters_live_20260701_v1\domain58_counter_badges_stream.bin --out replay_native\runs\country_economy_counters_live_20260701_v1\counter_report.json
cargo run --manifest-path replay_native\lanes\country_economy_counters_live\Cargo.toml --bin counter_gate -- --stream replay_native\runs\country_economy_counters_live_20260701_v1\domain58_counter_badges_stream.bin --out replay_native\runs\country_economy_counters_live_20260701_v1\counter_gate.json --min-game-hours 1
cargo run --manifest-path replay_native\lanes\country_economy_counters_live\Cargo.toml --bin replay_pack_proof -- --stream replay_native\runs\country_economy_counters_live_20260701_v1\domain58_counter_badges_stream.bin --out-dir replay_native\runs\country_economy_counters_live_20260701_v1\replay_pack --game-id country_economy_counters_live_v1 --min-game-hours 1
cargo run --manifest-path replay_native\snapshot\Cargo.toml --bin pack_replay -- verify replay_native\runs\country_economy_counters_live_20260701_v1\replay_pack\native_session.battleplan
```

## Registration Patch Plan

When the orchestrator grants the shared slot:

1. Register compact domain id `58` as `country_economy_counters_live`.
2. Add a lane-owned collector domain module that emits the domain-58 counter
   payload after domains `10` and `11` have run for the scheduled frame.
3. Keep domain `10` and `11` payloads authoritative for fuel and base production
   rows; domain `58` must not duplicate those records.
4. Add recorder-host decode/gate for the domain-58 row format above.
5. Require one domain-58 data record per scheduled frame, hourly-or-better,
   zero skipped hours, and no partial/source/worker/bridge flags.
6. Keep market/trade, licensing, state deposits, and law-token rows out of
   domain `58` unless an orchestrator-owned aggregate join is explicitly added.

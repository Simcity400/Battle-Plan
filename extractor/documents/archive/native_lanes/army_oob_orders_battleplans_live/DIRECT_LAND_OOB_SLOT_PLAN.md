# direct_land_oob integration slot plan

This is the exact patch plan for a future orchestrator-granted slot. It does not
grant permission to edit shared collector or recorder entrypoints.

## Scope

Add a new additive compact domain, `direct_land_oob` (`domain_id = 44`), for land
OOB roots that the current active-army slice cannot fully represent:

- direct `CArmy` roots, including empty/inactive armies not reachable through
  live `CDivision+0xB0`;
- army-group roots;
- theatre roots.

Keep existing domains stable:

- `army_orders` (`domain_id = 27`);
- `oob_command` (`domain_id = 28`).

## Validated Build Contract

Current build:
`870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`.

Validated root checks:

- `CArmy` vtable `MODULE+0x293BC60`, handle type `53`;
- army-group vtable `MODULE+0x293BCF8`, handle type `53`;
- theatre vtable `MODULE+0x295EE18`, handle type `67`;
- `CDivision` body vtable `MODULE+0x2943CB0`;
- `CDivision+0xB0` active army parent;
- `CDivision+0x1C8` owner tag-db id;
- `CArmy+0x5C` engine member count;
- `CArmy+0x98` orders vector;
- `CArmy+0x1B8` army-group parent.

The lane proof accepted 660 armies, 470 army groups, 97 theatres, and 3051
division roots with zero rejected vtable hits. It also proved 8 direct armies
not reachable through active division-parent traversal.

## Collector Slot Patch

Add an isolated collector module, proposed path:
`extractor/native/collector/src/lanes/army_oob_orders_battleplans_live/direct_land_oob.rs`.

The module should:

1. Reuse the validated vtable/type/id checks from `direct_oob_probe.py`.
2. Run direct-root discovery only in a bounded preflight/cache-refresh stage.
3. Never sweep the heap in the per-frame hot loop.
4. Revalidate cached roots by vtable, handle type, handle id, alignment, and
   sanity bounds before each emission.
5. Fail closed on build, vtable, handle, vector, or layout mismatch.
6. Emit `root_status = cold_preflight_only` for rows whose root came from the
   slow scan until a true bounded production root is implemented.
7. Consume division ids, owner tag-db ids, and `CArmy*` parent joins only; do not
   emit division scalar or movement state.

Shared entrypoint changes required only after slot grant:

- register compact domain `44`;
- schedule `direct_land_oob` beside, not inside, existing `army_orders` and
  `oob_command`;
- write one `direct_land_oob` data record per frame where the domain is
  scheduled;
- surface domain-specific source/partial status flags if the cache is stale or
  layout validation fails.

## Compact Payload

Use `direct_oob_compact.rs`:

- 32-byte header;
- 64-byte rows keyed by `(kind, object_id)`;
- kind `1 = army`, `2 = army_group`, `5 = theatre`;
- 8-byte member child rows;
- status `5 = cold_preflight_only`.

The first recorder slot should reject unknown flags, bad kinds, duplicate rows,
bad member ranges, nonzero reserved fields, and unknown status codes.

## Recorder Slot Patch

Additive decoder branch only:

- add domain `44` decoder;
- preserve existing domains `27` and `28` without row-width or version changes;
- add manifest counters:
  - `direct_land_oob_rows`;
  - `direct_land_oob_armies`;
  - `direct_land_oob_army_groups`;
  - `direct_land_oob_theatres`;
  - `direct_land_oob_member_rows`;
- pack decoded rows into `.battleplan` only after decode and cadence gates pass.

## Required Gates

Before accepting the slot:

```powershell
python extractor\native\lanes\army_oob_orders_battleplans_live\direct_land_oob_slot_gate.py extractor\native\lanes\army_oob_orders_battleplans_live\direct_land_oob_slot_contract.json
```

```powershell
rustc --edition 2021 --test extractor\native\lanes\army_oob_orders_battleplans_live\direct_oob_compact.rs -o $env:TEMP\direct_oob_compact_tests.exe
& $env:TEMP\direct_oob_compact_tests.exe
```

After shared slot integration:

- compact stream ordered and decodable;
- hourly-or-lower frame coverage;
- one data record per scheduled domain per frame;
- `.battleplan` verifies;
- zero skipped game hours;
- no source-unavailable, partial-data, bridge-unavailable, or worker-panic
  failures.

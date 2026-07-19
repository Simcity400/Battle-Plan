# B8/X6 building runtime-id registry

Status: **PROMOTION-READY**. The shared registry, exact evidence, collector/recorder
integration, and focused tests are complete.

Pinned binary: `hoi4.exe 1.19.2.0.a729`, image base `0x140000000`, SHA-256
`870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`.

## Promoted law

`CBuilding+0x08` is a full, non-dense 32-bit runtime building type id. For this
pinned executable, `compact_contract::building_registry::BUILDING_TYPE_ID_TO_TOKEN`
is the single production id-to-script-token registry. It contains all 55 vanilla
definitions loaded from `common/buildings/00_buildings.txt` and
`01_landmark_buildings.txt`.

The collector and recorder must consume that shared table. Unknown or mod-added ids
remain numeric and fail closed: map-detail uses `type_0xXXXXXXXX`; D48 retains its
numeric type key and omits the optional `token` field. No runtime id is truncated and
no unknown id is guessed.

The complete 55-row table is in
`extractor/native/compact_contract/src/building_registry.rs`. The machine-readable
proof, including the exact 43 observed and 12 uninstantiated rows, is
`B8_X6_BUILDING_REGISTRY_PROOF.json`.

## Mechanical proof

The release D04 stream was folded at game hour `1939.12.26.19` and joined to the
same-hour save `IRE_1939_12_26_19.hoi4`. Rakaly converted the save to JSON, preserving
per-state and per-province building tokens. For each live runtime id, the verifier
compared the complete level vector over all locations against every save token vector.

- Locations: 1,081 states and 13,413 provinces.
- Live rows: 2,589 state rows + 4,867 province rows = 7,456.
- Instantiated runtime ids: 9 state + 34 province = 43.
- Result: 43/43 ids had exactly one token candidate; zero ambiguous ids; zero registry
  mismatches.
- Game-file census: 55 definitions; registry census: 55 unique ids and 55 unique
  tokens; exact token-set equality.
- Remaining definitions absent from this save: exactly 12. Their numeric ids were
  independently proved by resolver-token extraction and the executable/game-file
  loader law in `B8_X6_TOKEN_ID_ASSIST.md`.

The 12 absent definitions are:

| Runtime id | Token | Id law |
|---:|---|---|
| `0x32B4` | `nuclear_reactor` | legacy identity |
| `0x33F7` | `rocket_site` | legacy identity |
| `0x760B` | `stronghold_network` | content-defined name token + 1 |
| `0x7734` | `mega_gun_emplacement` | content-defined name token + 1 |
| `0x782E` | `nuclear_reactor_heavy_water` | content-defined name token + 1 |
| `0x7860` | `commercial_nuclear_reactor` | content-defined name token + 1 |
| `0x7A24` | `energy_infrastructure` | content-defined name token + 1 |
| `0x7A57` | `industrial_infrastructure` | content-defined name token + 1 |
| `0x7C1A` | `landmark_berlin_volkshalle` | content-defined name token + 1 |
| `0x7D46` | `landmark_nanjing_presidential_palace_prc` | content-defined name token + 1 |
| `0x7E0E` | `landmark_national_diet` | content-defined name token + 1 |
| `0x7E40` | `landmark_hakko_ichiu` | content-defined name token + 1 |

This is a classified two-family result, not permission to apply a blanket `+1` rule
to future unknown entries. The independent 55-row census found 42 content-defined
`+1` rows, 13 legacy identity rows, and no other delta.

Input hashes recorded by the proof:

- folded snapshot: `b07464036dc3dd538227635729ac9a865f49faf7b60882aab1a0c35e03126af3`
- same-hour save: `862e96338e64c985a075d50b62a9bae251e5b0ed32d33dbaa33ae6f5f42d2273`
- `00_buildings.txt`: `9fa27d266f726de04c71311551698c6466257636dcc34d2573a3568d397bd539`
- `01_landmark_buildings.txt`: `9528c42836e08ba01019d62d42607d0d5fb2473b8aa7aa7cb852d95e7a08002c`
- resolver executable: `971d44ee56657f060edfd3745a570f6e0dc01a7f2e59f2cb8322751afc15b269`

## Ghidra evidence ledger

Every executable claim below records function address, job, artifact, and manifest
hash. Artifact hashes are included where returned.

| Finding | Address | Job ID | Artifact path | Manifest SHA-256 | Artifact SHA-256 |
|---|---:|---|---|---|---|
| Exact legacy enum-to-save-token serializer switch, including dockyard, infrastructure, air base, rocket site, nuclear reactor, supply node, fuel silo, and naval supply hub | `0x141C1DC20` | `20260716T225417Z-b8-legacy-serializer-address-check-v1-f1c9fec6` | `evidence/func_141c1dc20.decompile.c` | `c7a4a403f3a47b6c21768c51967178c84f753c9e5807420ded540f68f2851059` | `72e50f0807c75ade491f86a297fbd1621bd9661948bf780844ecca6057174843` |
| Top-level database startup loads `common/buildings` and dispatches the building database loader | `0x14018B610` | `20260716T234458Z-b8-x6-building-definition-loader-v4-52cb7d47` | `evidence/func_14018b610.decompile.c` | `8e834019ea39d2e4d268ab9816fad562a6f4f1952275989162aa072744ceac45` | not separately returned |
| Building database file enumeration | `0x14067AE90` | `20260716T234631Z-b8-x6-building-db-parser-v5-dd6acd4f` | `evidence/func_14067ae90.decompile.c` | `0b4cca7310bd53bedde8cfc03f07ae54cc049c3fd48d4cae8a9f7a4d467596ad` | not separately returned |
| Parsed declarations become database objects/building templates and are appended to `CBuildingDatabase` | `0x140677080` | `20260716T234631Z-b8-x6-building-db-parser-v5-dd6acd4f` | `evidence/func_140677080.decompile.c` | `0b4cca7310bd53bedde8cfc03f07ae54cc049c3fd48d4cae8a9f7a4d467596ad` | not separately returned |
| Generic persistent database-object token parser used by the building loader | `0x1424A96C0` | `20260716T234800Z-b8-x6-database-object-id-parser-v6-843aa2dd` | `evidence/func_1424a96c0.decompile.c` | `88cfc05a52f4fdc04429f780c1f801cc206855c7e82699ff8378af371f234876` | not separately returned |
| Legacy raid-source conversion recognizes `rocket_site` | `0x14157A3C0` | `20260716T233400Z-b8-x6-building-token-hardcoded-users-v2-f5541253` | `evidence/func_14157a3c0.decompile.c` | `d509cfa12b8c95df3c3727c843d3e852d7949427aa2c64450d7658f9189053f7` | not separately returned |
| Raid-source conversion recognizes `mega_gun_emplacement` and `rocket_site` | `0x141581940` | `20260716T233400Z-b8-x6-building-token-hardcoded-users-v2-f5541253` | `evidence/func_141581940.decompile.c` | `d509cfa12b8c95df3c3727c843d3e852d7949427aa2c64450d7658f9189053f7` | not separately returned |
| Negative finding retained: this function registers state scripted variables/effects and is not a building registry | `0x140069990` | `20260716T225404Z-b8-building-enum-token-switch-v1-006b942e` | `evidence/func_140069990.decompile.c` | `499c1bb81fcb43ad16608c3ab28b0250846986ddbf3ada0c49c98d1e740baf1d` | `c061ae14be7d05885a77a6bc6c1caedb2070425f074de3dc553f627b0e535efe` |

String evidence:

- Central roots: job `20260716T225219Z-b8-x6-building-registry-roots-v1-7c767e25`,
  artifact `string-xrefs.txt`, identity-manifest SHA-256
  `cd33527ef2f8f5d73ac87272d13b3c6219927272875ffe379ec7f4e1f4f8f8fe`,
  artifact SHA-256 `588688251912f5118193dc93ff226da0538469468798122b23c3905e6be91c09`.
- Missing-token executable search: job
  `20260716T225744Z-b8-x6-missing-building-tokens-v1-58455949`, artifact
  `string-xrefs.txt`, identity-manifest SHA-256
  `cd33527ef2f8f5d73ac87272d13b3c6219927272875ffe379ec7f4e1f4f8f8fe`,
  artifact SHA-256 `d3515cb2258b0f6c15742a0dfc8c149c6f8f650c29472137255df2ca431048a7`.
- Twelve-token candidate xrefs: job
  `20260716T233109Z-b8-x6-missing-building-token-strings-v1-26f1f9df`, artifact
  `string-xrefs.txt`, identity-manifest SHA-256
  `cd33527ef2f8f5d73ac87272d13b3c6219927272875ffe379ec7f4e1f4f8f8fe`,
  artifact SHA-256 `289ee28b0375858cb625dc727a9bcae35a9a7ca052fc2cdba5bffcf28d8ef922`.
- Loader strings and `common/buildings` xref `0x14018FA17`: job
  `20260716T233427Z-b8-x6-building-loader-strings-v2-5369591c`, artifact
  `string-xrefs.txt`, identity-manifest SHA-256
  `cd33527ef2f8f5d73ac87272d13b3c6219927272875ffe379ec7f4e1f4f8f8fe`,
  artifact SHA-256 `246e26a113ebf0ff059a0d25a82244d2481005aa002f15ae16c7ddb4dc5a925d`.

Auxiliary result retained: most content-defined names are not executable literals;
they enter through `CBuildingDatabase`. Literal presence for `mega_gun_emplacement`,
`rocket_site`, and `nuclear_reactor` comes from separate engine users and does not
define the runtime-id law. Global token-initializer xrefs load token globals and call
the registration helper; they do not contain usable inline numeric immediates.

## Production integration and tests

- `compact_contract/src/building_registry.rs`: complete 55-entry registry, pinned
  executable hash, binary-search lookup, uniqueness/sort/current-build tests.
- `collector/src/lib.rs`: obsolete local switch replaced by shared lookup and
  eight-hex numeric fallback. This edit predates the collector-owner freeze.
- `recorder_host/src/world_fold.rs`: map-detail names use the shared lookup; D48
  rows keep numeric keys and add `token` only for exact known ids.
- `recorder_host/src/world_fold/tests.rs`: asserts exact known tokens and verifies
  unknown ids remain tokenless through clean and expected-partial folds.
- Reproducible verifier:
  `probes/verify_b8_x6_building_registry.py`.

Verification completed:

- `cargo test -p compact_contract building_registry -- --nocapture`: 2 passed.
- `cargo test -p collector --lib cbuilding_type_and_level_fast_path_matches_contract -- --nocapture`: 1 passed.
- `cargo test -p recorder_host --lib chunk2_d48 -- --nocapture`: 6 passed.
- `cargo test -p recorder_host --lib chunk2_folds_d48_under_domain_owned_extras_without_d23_collisions -- --nocapture`: 1 passed.
- `cargo check -p collector -p recorder_host`: passed.
- `verify_b8_x6_building_registry.py`: PASS, 7,456 rows, 55 definitions, 55 registry
  entries, 43 observed exact joins, 12 independently proven absent rows, zero
  ambiguity, zero mismatch.

Final collector verification also confirmed that the shared lookup survived the
serialized handoffs unchanged and that none of the obsolete prior-build ids remain in
the collector-local building naming path.

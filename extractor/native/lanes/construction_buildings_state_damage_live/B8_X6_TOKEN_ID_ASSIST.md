# B8/X6 building token-id assist

Status: **PROMOTION-READY** for the twelve previously missing building-name mappings.

Binary contract: `hoi4.exe 1.19.2.0.a729`, image base `0x140000000`, SHA-256 `870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`.

## Exact result

| Runtime `type_id` | Canonical building token | Save/name token | Applied law |
|---:|---|---:|---|
| `0x760B` | `stronghold_network` | `0x760A` | content-defined id = name token + 1 |
| `0x7734` | `mega_gun_emplacement` | `0x7733` | content-defined id = name token + 1 |
| `0x33F7` | `rocket_site` | `0x33F7` | legacy engine id = name token |
| `0x32B4` | `nuclear_reactor` | `0x32B4` | legacy engine id = name token |
| `0x782E` | `nuclear_reactor_heavy_water` | `0x782D` | content-defined id = name token + 1 |
| `0x7860` | `commercial_nuclear_reactor` | `0x785F` | content-defined id = name token + 1 |
| `0x7A24` | `energy_infrastructure` | `0x7A23` | content-defined id = name token + 1 |
| `0x7A57` | `industrial_infrastructure` | `0x7A56` | content-defined id = name token + 1 |
| `0x7C1A` | `landmark_berlin_volkshalle` | `0x7C19` | content-defined id = name token + 1 |
| `0x7D46` | `landmark_nanjing_presidential_palace_prc` | `0x7D45` | content-defined id = name token + 1 |
| `0x7E0E` | `landmark_national_diet` | `0x7E0D` | content-defined id = name token + 1 |
| `0x7E40` | `landmark_hakko_ichiu` | `0x7E3F` | content-defined id = name token + 1 |

This is a closed two-family rule, not a guessed blanket increment. A mechanical census over the 55-entry B8 registry and the current the game resolver table produced **42 exact `+1` content-defined rows, 13 exact identity legacy rows, and zero other deltas**. The two legacy rows in this twelve-item set are therefore deliberately not incremented.

## Independent data provenance

The installed resolver executable contains its the game save-token table as literal `0xNNNN token` records. Exact-line extraction returned:

```text
stronghold_network: 0x760A
mega_gun_emplacement: 0x7733
rocket_site: 0x33F7
nuclear_reactor: 0x32B4
nuclear_reactor_heavy_water: 0x782D
commercial_nuclear_reactor: 0x785F
energy_infrastructure: 0x7A23
industrial_infrastructure: 0x7A56
landmark_berlin_volkshalle: 0x7C19
landmark_nanjing_presidential_palace_prc: 0x7D45
landmark_national_diet: 0x7E0D
landmark_hakko_ichiu: 0x7E3F
```

Source: `C:\Users\Example\Downloads\rakaly_cli\rakaly-0.8.17-x86_64-pc-windows-msvc\rakaly.exe`, SHA-256 `971d44ee56657f060edfd3745a570f6e0dc01a7f2e59f2cb8322751afc15b269`.

The installed game definitions independently establish that all twelve strings are building declarations:

- `common/buildings/00_buildings.txt`: lines 200, 317, 338, 402, 433, 463, 711, and 730; SHA-256 `9fa27d266f726de04c71311551698c6466257636dcc34d2573a3568d397bd539`.
- `common/buildings/01_landmark_buildings.txt`: lines 261, 429, 540, and 568; SHA-256 `9528c42836e08ba01019d62d42607d0d5fb2473b8aa7aa7cb852d95e7a08002c`.

## Executable provenance

| Finding | Function address | Job ID | Artifact path | Manifest SHA-256 |
|---|---:|---|---|---|
| Top-level database startup loads `common/buildings` and dispatches the building database loader | `0x14018B610` | `20260716T234458Z-b8-x6-building-definition-loader-v4-52cb7d47` | `evidence/func_14018b610.decompile.c` | `8e834019ea39d2e4d268ab9816fad562a6f4f1952275989162aa072744ceac45` |
| Building database file enumeration | `0x14067AE90` | `20260716T234631Z-b8-x6-building-db-parser-v5-dd6acd4f` | `evidence/func_14067ae90.decompile.c` | `0b4cca7310bd53bedde8cfc03f07ae54cc049c3fd48d4cae8a9f7a4d467596ad` |
| Each parsed declaration is materialized as a `CDatabaseObject` / intermediate building template and appended to `CBuildingDatabase` | `0x140677080` | `20260716T234631Z-b8-x6-building-db-parser-v5-dd6acd4f` | `evidence/func_140677080.decompile.c` | `0b4cca7310bd53bedde8cfc03f07ae54cc049c3fd48d4cae8a9f7a4d467596ad` |
| Generic persistent database-object token parser used by the building loader | `0x1424A96C0` | `20260716T234800Z-b8-x6-database-object-id-parser-v6-843aa2dd` | `evidence/func_1424a96c0.decompile.c` | `88cfc05a52f4fdc04429f780c1f801cc206855c7e82699ff8378af371f234876` |
| Legacy raid-source name conversion confirms `rocket_site` is an engine-recognized legacy building spelling | `0x14157A3C0` | `20260716T233400Z-b8-x6-building-token-hardcoded-users-v2-f5541253` | `evidence/func_14157a3c0.decompile.c` | `d509cfa12b8c95df3c3727c843d3e852d7949427aa2c64450d7658f9189053f7` |
| Raid-source conversion also recognizes `mega_gun_emplacement` and `rocket_site` | `0x141581940` | `20260716T233400Z-b8-x6-building-token-hardcoded-users-v2-f5541253` | `evidence/func_141581940.decompile.c` | `d509cfa12b8c95df3c3727c843d3e852d7949427aa2c64450d7658f9189053f7` |

Supporting string jobs:

- Candidate-name xrefs: job `20260716T233109Z-b8-x6-missing-building-token-strings-v1-26f1f9df`, artifact `string-xrefs.txt`, artifact SHA-256 `289ee28b0375858cb625dc727a9bcae35a9a7ca052fc2cdba5bffcf28d8ef922`, identity-manifest SHA-256 `cd33527ef2f8f5d73ac87272d13b3c6219927272875ffe379ec7f4e1f4f8f8fe`.
- Loader strings including the `common/buildings` xref at `0x14018FA17`: job `20260716T233427Z-b8-x6-building-loader-strings-v2-5369591c`, artifact `string-xrefs.txt`, artifact SHA-256 `246e26a113ebf0ff059a0d25a82244d2481005aa002f15ae16c7ddb4dc5a925d`, identity-manifest SHA-256 `cd33527ef2f8f5d73ac87272d13b3c6219927272875ffe379ec7f4e1f4f8f8fe`.

## Auxiliary findings retained

- Nine of the content-defined names and `stronghold_network` have no literal in `hoi4.exe`; they are loaded from installed content through `CBuildingDatabase`. This is expected and is why exact token-table plus definition-file provenance is required.
- `mega_gun_emplacement`, `rocket_site`, and `nuclear_reactor` do have executable literals because separate engine subsystems refer to them directly; literal presence is not the registry-id law.
- Dumping bytes at the global token-initializer xrefs did not expose inline numeric immediates: those sites load token globals and call the registration helper. The exact numeric source is the resolver table above.

## Promotion instruction

Promote the twelve rows exactly as listed. Do not apply `+1` to `rocket_site` or `nuclear_reactor`, and do not attempt to infer unknown future entries from a blanket arithmetic rule without first classifying them as legacy versus content-defined.

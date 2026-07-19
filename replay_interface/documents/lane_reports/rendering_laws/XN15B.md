# XN15B — shipped semantic visual-trigger census

Status: **PROVEN / PROMOTION READY / FAIL CLOSED**

XN15B closes the shipped-file producer catalog for ground-unit, country/map,
and weather visuals. It preserves the full authored entity-event payload and
joins every particle or light token to the exact effective-VFS definition.
Air and naval entity rows remain in the evidence census so no finding is lost,
but their runtime presentation belongs to XN13/A4/A21. Particle simulation is
owned by XN12 and is not re-derived here.

The executable authority is `hoi4.exe 1.19.2.0.a729`, image base
`0x140000000`, SHA-256
`870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`.
The ten-shard Ghidra pool passed the required identity probe before this work.

## Production law

1. Resolve the ordered VFS as base, always-active `integrated_dlc/*` in
   deterministic order, enabled DLC, then mods. Later layers win relative-path
   collisions. The recording header pins that exact manifest.
2. Key an authored visual trigger by resolved entity digest, semantic state,
   authored state ordinal, and authored event ordinal. Preserve the ordered
   event payload, including duplicate keys; convenience fields never replace
   that payload.
3. Resolve `particle` through the case-sensitive `pdxparticle` registry and its
   `type` through the particle-definition catalog. A missing chain is explicit
   null. All 18,131 shipped chains in the pinned namespace resolve.
4. Resolve `light` through the ten-token light-definition catalog and preserve
   the complete ordered definition payload, including color, position,
   intensity, radius, falloff, animation, and curve rows. A missing definition
   is explicit null/no-light; never alias a similar token.
5. Static authored values do not decide an ambiguous runtime trigger. XN15's
   recorded sequence/update, actual trigger tick, instance identity, parent
   anchor, and post-evaluation submitted state are authoritative. This rule
   makes duplicate authored keys exact without inventing first- or last-wins.
6. `common/weather.txt` contributes nine ordered `weather_effect` rows. Their
   entity/fading-entity identities join the same entity catalog; XN14 owns
   runtime weather-manager activation and composition.

## Exhaustive census

The v2 census consumes the frozen XN13 entity corpus and scans all shipped
`.gfx` and `.asset` files plus `common/weather.txt`.

| Item | Exact count |
|---|---:|
| all authored entity events | 22,311 |
| visual entity events | 18,134 |
| particle-bearing events | 18,131 |
| light-bearing events | 816 |
| sound-only auxiliary events | 3,949 |
| id-only auxiliary events | 228 |
| unique particle registrations | 180 |
| unique particle effect tokens | 146 |
| unique light definition tokens | 10 |
| weather-effect rows | 9 |
| unresolved particle chains | 0 |
| unresolved light chains | 0 |

Producer accounting is exact: ground-unit 10,392; country/map static 611;
weather 17; excluded air 3,790; excluded naval 3,324. Their sum is 18,134.

## Closed authored anomalies

Two rows contain duplicate keys. Both ordered payloads are normative and the
runtime stream supplies the actually selected timing/anchor:

- `gfx/entities/nuke.asset`, `nuke_big_entity`, state `attack` index 0/event 2
  contains ordered `time = 4`, then `time = 0.5`. Its
  `nuke_big_top_particle` chain resolves normally. Its `light = nuke_flash`
  has no definition in any effective shipped layer and no executable
  hardcoded token/fallback literal. The exact replay result is a valid particle
  event plus explicit null/no-light. It must not be aliased to
  `nuke_big_light`.
- DLC050 `wotp_units_ships.asset`, `USA_carrier_0_entity`, state `idle` index
  1/event 10 contains ordered nodes `idle_foam`, then `flying_plane_loc` for
  `air_trail_particle`. This is retained evidence but belongs to the excluded
  XN13 air/naval presentation owner. XN15 records the actual parent anchor; no
  static node winner is invented.

The earlier XN12A claim that four DLC049/DLC052
`amphibious_move_water_particle` rows were null was corrected and refrozen.
The globally unique registration is shipped by always-active
`integrated_dlc/dlc023_man_the_guns/gfx/entities/MTG_particles.gfx` and resolves
to `amphibious_move_water_effect` in the same integrated package. XN12A now
passes 22 checks with census SHA-256
`c736085bbea45ba6688cccbaef3810b11d3d4590f435ed9ea6f1768f514d0b0c`
and verification SHA-256
`306430bc5643d2db4813f371650e2b9122286e00525e22f5269e71dfe8886eec`.

## Ghidra evidence ledger

Every executable-derived row records address, job, artifact, artifact hash,
and manifest hash. `N/A` means an exact absence scan produced no function xref.

| Finding | Function/address | Job ID | Artifact and SHA-256 | Manifest SHA-256 |
|---|---|---|---|---|
| exact entity-event `light` grammar key | `0x140082CB0` | `20260717T013324Z-xn15b-exact-light-key-xrefs-b0965d90` | `string-xrefs.txt` — `dd245d25f15e29346b43cf1ce7efb64295578d1e2c2affa22565497c47d3e057` | `cd33527ef2f8f5d73ac87272d13b3c6219927272875ffe379ec7f4e1f4f8f8fe` |
| exact entity-event `time` grammar key | `0x140082CB0` | `20260717T014143Z-xn15b-entity-event-time-key-xrefs-27c7da6f` | `string-xrefs.txt` — `29ca0bd8f1ddd9cb34d37e566bf08191d28e0b0640b68e3c2d2d70e9621bbb46` | `cd33527ef2f8f5d73ac87272d13b3c6219927272875ffe379ec7f4e1f4f8f8fe` |
| `nuke_flash`, named nuke lights, map-light file, and missing-light fallback literals absent from executable | `N/A` | `20260717T013106Z-xn15b-nuke-light-dispatch-anomaly-5fc5ea07` | `string-xrefs.txt` — `d99fdfc617c4f99b3308fc0abe6097728490f18cb10a28b4010d3385f67d122a` | `cd33527ef2f8f5d73ac87272d13b3c6219927272875ffe379ec7f4e1f4f8f8fe` |
| light grammar registration neighborhood | `0x140082CB0` | `20260717T015508Z-xn15b-entity-event-light-parser-contiguo-61380775` | `datdump.txt` — `25b3f8459b8a5afb11c09412643f08c2985962922abc00ece50771bc6b7aecdc` | `cd33527ef2f8f5d73ac87272d13b3c6219927272875ffe379ec7f4e1f4f8f8fe` |
| time grammar registration neighborhood | `0x140082CB0` | `20260717T014258Z-xn15b-entity-event-time-parser-contiguou-299726e2` | `datdump.txt` — `00cfb91d7a3419b44674c4ab05a7a0cd085ee7b27b64c475395141c134a8f82d` | `cd33527ef2f8f5d73ac87272d13b3c6219927272875ffe379ec7f4e1f4f8f8fe` |
| broader light-resource diagnostic terms absent | `N/A` | `20260717T013229Z-xn15b-light-resource-dispatch-b09c9a54` | `string-xrefs.txt` — `7d84ce0ca9102f28ef16e83eda6bf66b614ef19e0bcf4220bf27bdd017d0ecac` | `cd33527ef2f8f5d73ac87272d13b3c6219927272875ffe379ec7f4e1f4f8f8fe` |
| radius/falloff/intensity keys bind the same asset grammar | `0x140082CB0` | `20260717T015804Z-xn15b-map-light-runtime-anchors-0d1ab08d` | `string-xrefs.txt` — `2e9581f5fd8789f834d463d3eb69052e95fd12596755a8f45f6728bcf5c79759` | `cd33527ef2f8f5d73ac87272d13b3c6219927272875ffe379ec7f4e1f4f8f8fe` |

The exact `light` search also produced useful false leads. They are recorded so
future lanes do not repeat them: `0x140218A70` uses “light” for ship-hull class
logic (`func_140218a70.decompile.c`, SHA
`29cd517d6bae4203b20e3885c6ece3df678b79759d927d9bd6c0ec24188e33c4`);
`0x14135F0E0` and `0x140E5E6E0` are gameplay/unit aggregators (SHAs
`ad8871a875ba03cb25b9163f81de758aa39f1767bdda924b84fa6de73ec0c8cf`
and `c0fa8efae1e5e1e6831e3eb14df1e6ee95b3db18fca9f359239cae432acf3dfa`);
`0x141181940` and `0x141181E90` build registration tables but do not resolve an
entity-event light token (SHAs
`fc5ef927c5df3a6f3ae73e8cb31e4e1b681086e44fbc9685f04caef91f89692d`
and `61b12f1ce4181bd3d00781c18b700a0a4c83ac2bc2b27ff1438b9329ff8ad963`).
All five belong to job
`20260717T013404Z-xn15b-light-key-consumers-84ae0e42`, artifact paths
`evidence/func_<address>.decompile.c`, manifest SHA-256
`fb1f9d0890e3b0def0bc5c10eae1d742463414f55f233a140170e2c15fc805c9`.

## Mechanical gate and promotion handoff

`verify_xn15b.py` passes 40/40 checks. The normative artifacts are
`gamefile_trigger_census.json`, `build_xn15b_trigger_census.py`, and
`verification.json`. Promotion consumes this static catalog through XN15's
runtime lifecycle/submission stream. There is no XN15B data blocker and no
XN15B capture blocker.

# Garrison Scalar Reverse-Engineering Closure

Lane: `occupation_resistance_compliance_live`

Build gate: `hoi4.exe 1.19.2.0.a729`, image base `0x140000000`, SHA-256
`870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`.

Status: **closed for the three production per-state garrison scalars**. This
supersedes this file's pre-wave-3 candidate/rejection state. Manpower-pool,
equipment-need, suppression, and reinforcement-request detail are separate
contracts and are not promoted by this finding.

## Exact object and field contract

The production walk is:

`CCountryOccupationStatus+0x48` -> occupation-data container ->
`CCountryOccupationData` (vtable RVA `0x296DD00`, parent status `+0x08`, occupied
grouping tag `+0x10`, garrison container `+0xD8`) ->
`CStateGarrisonData` (vtable RVA `0x296DCB0`, state `+0x50`, occupation-data
back-reference `+0x58`).

The fields are:

| Engine key | Key id | Object offset | Native storage | Production meaning |
|---|---:|---:|---:|---|
| `army_manpower_need` | `0x36FC` | `+0x88` | signed 32-bit | `garrison_manpower_need` |
| `strength_ratio` | `0x34FF` | `+0xF0` | signed 64-bit fixed x100000 | `garrison_strength_ratio` |
| `garrison_required` | `0x4AD1` | `+0xF8` | signed 64-bit fixed x100000 | `garrison_required` |

as a dword, pairing them with those key ids. The deserializer
independently bind `army_manpower_need`, `strength_ratio`, and
`garrison_required` to the ids. The scalar serializer helpers confirm an
eight-byte value parameter for `+0xF0/+0xF8` and a four-byte parameter for
`+0x88`.

The independent string/xref anchors are `army_manpower_need` at
`0x142AD9100` / xref `0x14008FFE5`, relevant `strength_ratio` xref
`0x1400A9521`, `state_garrison_data` at `0x142AD9EB8` / xref `0x140103143`,
and `garrison_required` at `0x142AD9F80` / xref `0x1401031EE`.

Domain-52 wire v1 has signed 32-bit slots for the two fixed-point values. The
live reader therefore reads the complete native `i64` and performs checked
`i32` conversion plus the existing domain bound before emission. A nonzero
high word or any other non-representable value fails closed as a garrison
anomaly; no native value is silently truncated.

## Retained dynamic proof

The paused D04 same-tick save/live oracle produced a clean joined diagonal:

- `army_manpower_need +0x88`: `18/18` discriminating rows, `280/280` all rows;
- `strength_ratio +0xF0`: `288/288` rows;
- `garrison_required +0xF8`: `282/282` rows;
- full joined proof: `303` rows, `0` stale rows.

This explicitly overturns the earlier v19/v23 rejection, which was caused by
an under-discriminating fixture and is preserved only as historical negative
evidence. The retained proof is summarized in
`extractor/documents/archive/WAVE3_CRACKS_870EBF_D04.md`.

## Production and replay integration

- `collector_module.rs` performs the bounded pointer walk, exact vtable,
  parent/back-reference and validated-state joins, complete-width scalar reads,
  conflict rejection, and unavailable/anomaly marking.
- `domain44_compact.rs` carries the three scalar slots and rejects impossible
  flag/value combinations.
- `extractor/native/collector/src/lib.rs` emits the lane payload and tracker
  counts.
- `extractor/native/recorder_host/src/world_fold.rs` requires the measured flag,
  rejects measured-plus-unavailable rows, and divides the two fixed-point
  values by exactly `100000.0`.
- `replay_interface/native/replay_core/src/world.rs` and
  `replay_interface/native/game_ui/src/occupation_binding.rs` consume the typed
  fields.

## Retired placeholder vocabulary

The executable contains no `garrison_equilibrium` or `garrison_progress`
schema strings. Those names belong to the early Python probe's placeholder
slots; they are not authenticated engine field names and must not be promoted
as aliases. The production contract exposes the three exact engine-derived
fields above. The Python `live_probe.py` remains a legacy diagnostic producer
and deliberately emits its old scalar slots as unavailable; it is not the
production collector.

## Static evidence ledger

- String/xref search job
  `20260719T133543Z-occupation-garrison-residual-scalars-202-932d24fb`, artifact
  SHA-256 `400ab2ed74ec8b03f9c4308b0ecad73deec39bacea28c2dcff2e54350f6ca8bd`.
- Field-key data dump job
  `20260719T133849Z-occupation-garrison-field-schema-ids-202-3104849f`, artifact
  SHA-256 `36c82b105863455ccff83b2d132ea9d14c23db4c2b7d0d445ab17db8fa2613b9`.
- `CStateGarrisonData` vtable dump job
  `20260719T133722Z-occupation-garrison-vtable-methods-20260-01f5c210`.
- Xref-window disassembly job
  `20260719T133628Z-occupation-garrison-serializer-xref-wind-ed39e90b`, artifact
  SHA-256 `73b34f108de00e470be0cef0c0a9b5a77b0dfbb2bd75c7c4b9305578f7136819`.
- Vtable-method export job
  `20260719T133735Z-occupation-garrison-vtable-methods-20260-215a4083`, manifest
  SHA-256 `d3884b98ca3a1e39dc69860666f68c4d40ae1304f33600e8d1c0b7879cae5bf5`.
  The serializer and deserializer decompiles have SHA-256
  `d73e46f0ccd2e02393c4b06869b081633403caaf292228932fc24e09165ec0c5`
  and `9078d02e90608993d63491ce284cf70e9a26b5fc5fee87eb328397013be162a3`.
- Scalar-codec export job
  `20260719T134008Z-occupation-garrison-serializer-scalar-co-57b57a5b`, manifest
  SHA-256 `c3889c28440f705b469dc4a834a0299504a58f11cd95695fb6dbc75829efc3ce`.
- The monolithic registration function at `0x140082CB0` did not decompile in
  job `20260719T133604Z-occupation-garrison-serializer-root-2026-7584d456`.
  This is retained negative evidence; the leaf serializer/deserializer, key
  data, xrefs, and dynamic oracle independently close the field contract.

Machine-readable form:
`GARRISON_SCALAR_STATIC_COMPLETION_20260719.json`.

## Remaining non-human-input caveats

- A theoretical one-frame ABA window remains because the garrison walk precedes
  the state-row read and the objects expose no generation counter. All joins
  fail closed, but an A-to-B-to-A controller change wholly inside that window
  is not mechanically distinguishable.
- A transient state-row read failure omits that state for the frame, so the
  downstream fold retains its prior value until a successful row arrives. This
  is a lane-wide snapshot behavior, not unresolved scalar reverse engineering.

No human input is required to identify, name, read, encode, or replay these
three scalar fields on the pinned build.

<!-- WAVE2_EXACT10_CONSUMER_BINDING_BEGIN -->
## Exact-ten replacement authority

This machine-readable block replaces discovery-only custody with exact-ten receipts. It does not alter the lane's independent runtime or human activation gates.

```json
{
  "schema": "hoi4-wave2-exact10-inline-consumer-binding-v1",
  "status": "exact10_consumer_rewrite_complete",
  "consumer_path": "extractor/native/lanes/occupation_resistance_compliance_live/GARRISON_RESIDUAL_ANALYSIS.md",
  "reconciliation_path": "extractor/documents/WAVE2_10_SHARD_REEXPORT_RECONCILIATION_20260720.json",
  "binary_id": "hoi4.exe 1.19.2.0.a729",
  "image_base": "0x140000000",
  "sha256": "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad",
  "old_discovery_jobs_are_non_promotable_aliases_only": true,
  "lane_activation_unchanged": true,
  "roots": [
    {
      "root_call_id": "ADDR-02-02",
      "canonical_claims": [
        {
          "discovery_job_id": "20260719T133628Z-occupation-garrison-serializer-xref-wind-ed39e90b",
          "actual_job_id": "20260719T220100Z-wave2-r10-addr-02-02-897a01a5",
          "artifact_sha256": "74ba4709ad0c169547f4b07f8abad42d79e7a4488175af3ecebfd9b8731491bb",
          "source_action_sequence": 57,
          "source_semantic_request_sha256": "4b87e388fcb0ac44a715b8b45fb7e33c745e095af0814c8505bc84c888b47b13",
          "request_membership": {
            "addresses": [
              "0x14008ffb0",
              "0x14008ffe5",
              "0x1400a94f0",
              "0x1400a9521",
              "0x140103110",
              "0x140103143",
              "0x1401031c0",
              "0x1401031ee"
            ],
            "depth": null,
            "pcode": null
          }
        },
        {
          "discovery_job_id": "20260719T133722Z-occupation-garrison-vtable-methods-20260-01f5c210",
          "actual_job_id": "20260719T220100Z-wave2-r10-addr-02-02-897a01a5",
          "artifact_sha256": "74ba4709ad0c169547f4b07f8abad42d79e7a4488175af3ecebfd9b8731491bb",
          "source_action_sequence": 63,
          "source_semantic_request_sha256": "6656917cb0de3f334f176d62c1ba077ddca0688642b2a10e022d353816642ef4",
          "request_membership": {
            "addresses": [
              "0x14296dcb0",
              "0x14296dcc0",
              "0x14296dcd0",
              "0x14296dce0",
              "0x14296dcf0",
              "0x14296dd00",
              "0x14296dd10",
              "0x14296dd20",
              "0x14296dd30",
              "0x14296dd40"
            ],
            "depth": null,
            "pcode": null
          }
        },
        {
          "discovery_job_id": "20260719T133849Z-occupation-garrison-field-schema-ids-202-3104849f",
          "actual_job_id": "20260719T220100Z-wave2-r10-addr-02-02-897a01a5",
          "artifact_sha256": "74ba4709ad0c169547f4b07f8abad42d79e7a4488175af3ecebfd9b8731491bb",
          "source_action_sequence": 69,
          "source_semantic_request_sha256": "230ded5c6a99e0424ca3240afb86b797cc87efcf998c0ee517eebf271cf43543",
          "request_membership": {
            "addresses": [
              "0x142ad9e80",
              "0x142ad9eb8",
              "0x142ad9f08",
              "0x142ad9f48",
              "0x142ad9f80",
              "0x142ad9fb8",
              "0x142adfef4",
              "0x142adff20",
              "0x142adff48",
              "0x142adff88",
              "0x142adffc8",
              "0x142adeb1c",
              "0x14276fab4"
            ],
            "depth": null,
            "pcode": null
          }
        }
      ],
      "supplemental_action_ids": [],
      "receipt": {
        "binding_kind": "successful_artifact",
        "status": "complete",
        "receipt_authoritative": true,
        "actual_job_id": "20260719T220100Z-wave2-r10-addr-02-02-897a01a5",
        "actual_shard_id": "shard-03",
        "job_json_path": "<local-path>",
        "job_json_sha256": "3c58cdeccef1232d03ff18696dab1045de76db3623a3fd5ec1827b4a37430f1f",
        "actual_request_sha256": "5bdbf977fbbe12f656f764fe7322c8e6822e62efaae8950f2480afe1eb30223e",
        "identity_manifest_sha256": "cd33527ef2f8f5d73ac87272d13b3c6219927272875ffe379ec7f4e1f4f8f8fe",
        "artifact": "<local-path>",
        "artifact_sha256": "74ba4709ad0c169547f4b07f8abad42d79e7a4488175af3ecebfd9b8731491bb",
        "pool_status_path": "<local-path>",
        "pool_status_sha256": "df67a663059a645bf8f039f34659fed76274a7259af2fc137a2a60b5e8e41395"
      }
    },
    {
      "root_call_id": "ADDR-03-02",
      "canonical_claims": [
        {
          "discovery_job_id": "20260719T133735Z-occupation-garrison-vtable-methods-20260-215a4083",
          "actual_job_id": "20260719T220140Z-wave2-r10-addr-03-02-bda79b33",
          "artifact_sha256": "dd19cb97391dcaa77b8eb4c2f9980d4b86fa1e703342e5e9843b76995a9edb31",
          "source_action_sequence": 64,
          "source_semantic_request_sha256": "62d8e061671a1871ea2f2cbb900cb176c0e7d88cdfa3628d7d13c492372c1128",
          "request_membership": {
            "addresses": [
              "0x140fde3d0",
              "0x1424a9cf0",
              "0x140feb450",
              "0x1424a9730",
              "0x140fe7170",
              "0x14011cd10",
              "0x140fde0e0",
              "0x140fea660",
              "0x140fe5070"
            ],
            "depth": 1,
            "pcode": true
          }
        }
      ],
      "supplemental_action_ids": [],
      "receipt": {
        "binding_kind": "successful_artifact",
        "status": "complete",
        "receipt_authoritative": true,
        "actual_job_id": "20260719T220140Z-wave2-r10-addr-03-02-bda79b33",
        "actual_shard_id": "shard-03",
        "job_json_path": "<local-path>",
        "job_json_sha256": "c428b6b55ddb4500706b05b1bacf4dfa7378ef0a64c2d0b5560dc19b74e8fbbe",
        "actual_request_sha256": "9b78d135679253090fcaa5691e9ce7f3ef706e1c4e98fc64e707bed3bc2059c1",
        "identity_manifest_sha256": "cd33527ef2f8f5d73ac87272d13b3c6219927272875ffe379ec7f4e1f4f8f8fe",
        "artifact": "<local-path>",
        "artifact_sha256": "dd19cb97391dcaa77b8eb4c2f9980d4b86fa1e703342e5e9843b76995a9edb31",
        "pool_status_path": "<local-path>",
        "pool_status_sha256": "df67a663059a645bf8f039f34659fed76274a7259af2fc137a2a60b5e8e41395"
      }
    },
    {
      "root_call_id": "ADDR-03-04",
      "canonical_claims": [
        {
          "discovery_job_id": "20260719T134008Z-occupation-garrison-serializer-scalar-co-57b57a5b",
          "actual_job_id": "20260719T221841Z-wave2-r10-addr-03-04-4e5baffd",
          "artifact_sha256": "919730d27c6dd097fd8a1876cdbf6125f1a1bddf349307e63d440b6eff3e7f11",
          "source_action_sequence": 72,
          "source_semantic_request_sha256": "95a09c8fc2fb618c97e4e3c9f9d2d80e78330f62168ddc8c506db5b7dace1095",
          "request_membership": {
            "addresses": [
              "0x1424abb10",
              "0x1424ab970",
              "0x1424ae590",
              "0x1424adfe0"
            ],
            "depth": 1,
            "pcode": true
          }
        }
      ],
      "supplemental_action_ids": [],
      "receipt": {
        "binding_kind": "successful_artifact",
        "status": "complete",
        "receipt_authoritative": true,
        "actual_job_id": "20260719T221841Z-wave2-r10-addr-03-04-4e5baffd",
        "actual_shard_id": "shard-03",
        "job_json_path": "<local-path>",
        "job_json_sha256": "0bb73774de2d1c0213da53abcfe0e098e619e12f8e54e5faf9aec7c8d71fee18",
        "actual_request_sha256": "28936f9836c5cfb968b0a86f51f683d722186b0fe621fddd2c00a14937a95eeb",
        "identity_manifest_sha256": "cd33527ef2f8f5d73ac87272d13b3c6219927272875ffe379ec7f4e1f4f8f8fe",
        "artifact": "<local-path>",
        "artifact_sha256": "919730d27c6dd097fd8a1876cdbf6125f1a1bddf349307e63d440b6eff3e7f11",
        "pool_status_path": "<local-path>",
        "pool_status_sha256": "df67a663059a645bf8f039f34659fed76274a7259af2fc137a2a60b5e8e41395"
      }
    },
    {
      "root_call_id": "EF-02-D1-P1",
      "canonical_claims": [
        {
          "discovery_job_id": "20260719T133604Z-occupation-garrison-serializer-root-2026-7584d456",
          "source_action_sequence": 17,
          "source_semantic_request_sha256": "30759e0f2664321c7e2386a8e50c34134336ce1cacab7f39dda9593d2cffb3b3",
          "request_membership": {
            "addresses": [
              "0x140082cb0"
            ],
            "depth": 1,
            "pcode": true
          }
        }
      ],
      "supplemental_action_ids": [],
      "receipt": {
        "binding_kind": "expected_failure",
        "status": "failed",
        "actual_job_id": "20260719T220127Z-wave2-r10-addr-03-01-631965a4",
        "actual_shard_id": "shard-03",
        "job_json_path": "<local-path>",
        "job_json_sha256": "cb920f99096b966aef68c47d6797855bf447950399acb3b3601ad2d17ab5d537",
        "actual_batched_request_sha256": "2be62849de394bee987a26cb11e6c3d51ab9ffbe1da1ad0bc0890c0412926e1c",
        "isolated_expected_request_sha256": "30759e0f2664321c7e2386a8e50c34134336ce1cacab7f39dda9593d2cffb3b3",
        "identity_manifest_sha256": "cd33527ef2f8f5d73ac87272d13b3c6219927272875ffe379ec7f4e1f4f8f8fe",
        "pool_status_path": "<local-path>",
        "pool_status_sha256": "df67a663059a645bf8f039f34659fed76274a7259af2fc137a2a60b5e8e41395",
        "error": "java.lang.IllegalStateException: java.lang.IllegalStateException: decompile failed for 140082cb0",
        "artifact": null,
        "artifact_sha256": null,
        "authority_scope": "required one-retry deterministic negative outcome only"
      }
    },
    {
      "root_call_id": "STR-011",
      "canonical_claims": [
        {
          "discovery_job_id": "20260719T133543Z-occupation-garrison-residual-scalars-202-932d24fb",
          "actual_job_id": "20260719T215331Z-wave2-r10-056-occupation-garrison-residu-d40b83dc",
          "artifact_sha256": "400ab2ed74ec8b03f9c4308b0ecad73deec39bacea28c2dcff2e54350f6ca8bd",
          "source_action_sequence": 56,
          "source_semantic_request_sha256": "9c17c7598d5433104b7cf7227e170249bdef1956a2d02dc7db2b96fd064e6b23",
          "request_membership": {
            "terms": [
              "army_manpower_need",
              "garrison_required",
              "strength_ratio",
              "state_garrison_data",
              "GARRISON_MANPOWER_NEED",
              "GARRISON_STRENGTH",
              "GARRISON_EQUIPMENT_NEED",
              "garrison_equilibrium",
              "garrison_progress"
            ]
          }
        }
      ],
      "supplemental_action_ids": [],
      "receipt": {
        "binding_kind": "successful_artifact",
        "status": "complete",
        "receipt_authoritative": true,
        "actual_job_id": "20260719T215331Z-wave2-r10-056-occupation-garrison-residu-d40b83dc",
        "actual_shard_id": "shard-03",
        "job_json_path": "<local-path>",
        "job_json_sha256": "652ebf81969ca68dd215cf076f01965ff30e00d24ab357abeea83fbe7b917f5b",
        "actual_request_sha256": "9c17c7598d5433104b7cf7227e170249bdef1956a2d02dc7db2b96fd064e6b23",
        "identity_manifest_sha256": "cd33527ef2f8f5d73ac87272d13b3c6219927272875ffe379ec7f4e1f4f8f8fe",
        "artifact": "<local-path>",
        "artifact_sha256": "400ab2ed74ec8b03f9c4308b0ecad73deec39bacea28c2dcff2e54350f6ca8bd",
        "pool_status_path": "<local-path>",
        "pool_status_sha256": "df67a663059a645bf8f039f34659fed76274a7259af2fc137a2a60b5e8e41395"
      }
    }
  ]
}
```
<!-- WAVE2_EXACT10_CONSUMER_BINDING_END -->

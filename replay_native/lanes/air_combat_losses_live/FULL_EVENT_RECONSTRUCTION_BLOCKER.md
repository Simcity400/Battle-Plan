# air_combat_losses_live domain 42 full-event reconstruction blocker

Status: validated slice only. Do not call full air combat complete.

The accepted production slice is air-wing strength/equipment delta rows for
losses, reinforcements, deletes, mission region, and equipment amount deltas.
The full sortie/combat event reconstruction backlog remains open except for
the direct opposed event-root candidate rejected here.

## Backlog item narrowed

Rejected candidate:

- Direct `StrategicAirManager` / airbase-owner-container / air-wing-pool object
  that holds opposed air-wing body/object references and can serve as a full
  sortie/combat/loss event root.

This rejection narrows the search for:

- full sortie lifecycle;
- combat pairing;
- kill attribution;
- cause/source of loss;
- tactical engagement state;
- ace participation;
- rich reinforcement reason.

It does not prove those fields are impossible. It only proves this bounded
direct-reference route is not the production path on the live 870ebf profile.

## Live proof

Proof artifact:

- `replay_native/lanes/air_combat_losses_live/proofs/direct_event_root_rejection_20260701_v1/air_combat_losses_probe_870ebf.json`

Command:

```powershell
python memory_reader/naval_air/air_combat_losses_probe_870ebf.py --pid 45880 --duration-s 30 --interval-s 0.10 --max-body-field-wings 128 --out-dir replay_native/lanes/air_combat_losses_live/proofs/direct_event_root_rejection_20260701_v1
```

Root validation:

- exe SHA-256 matched `870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`;
- daily update sentinel matched at `module+0x1D4390`;
- long-string sentinel matched at `module+0x526A40`;
- `CGameState` root: `module+0x3316E90 -> CGameState*`;
- current date: `CGameState+0x468`;
- `StrategicAirManager`: `CGameState+0x690`;
- `StrategicAirManager` vtable: `module+0x2942118`;
- live `StrategicAirManager` pointer: `0x22C1E3D95D0`;
- live vtable: `0x7FF6AD932118`.

Observed air-wing topology:

- air-wing rows observed: `426`;
- rows with mission region: `109`;
- temporal samples: `194`;
- advanced game hours: `51`;
- skipped game hours: `0`;
- loss-like deltas: `8`;
- reinforcement-like deltas: `10`;
- hourly-or-lower sampling: passed.

## Rejected direct-reference search

Scanned source spans:

- `StrategicAirManager`: `0x0..0x200`, sources scanned `1`;
- `airbase_owner_container`: `0x0..0x120`, sources scanned `439`;
- `air_wing_pool`: `0x0..0x160`, sources scanned `227`.

Criterion:

- unknown-vtable target object in a bounded root field contains at least two
  direct air-wing body/object refs in the first `0x400` bytes;
- event-root upgrade requires opposition-capable refs: at least two owner tags
  with region-bearing wings;
- same-owner pool-local collections are rejected.

Result:

- unknown-vtable pointers seen from air-wing pools: `774`;
- direct-reference candidates found: `6`;
- opposed event-root candidates found: `0`;
- status: `rejected_no_direct_opposed_event_root`;
- classification: `rejected_direct_opposed_event_root`.

Rejected candidate rows:

| source | source addr | field | target | vtable RVA | reason |
| --- | ---: | ---: | ---: | ---: | --- |
| air_wing_pool | `0x22CDEC23140` | `0xd8` | `0x22CDE51B8B0` | `0x29bbe58` | owner `16` only, region `1`, wings `16:110`, `16:274`, `16:354` |
| air_wing_pool | `0x22CDEC239B0` | `0xd8` | `0x22CDE51B8B0` | `0x29bbe58` | owner `16` only, region `1`, wings `16:110`, `16:274`, `16:354` |
| air_wing_pool | `0x22CDEC25260` | `0x128` | `0x22CDE51B830` | `0x29bbe58` | owner `16` only, region `1`, wings `16:110`, `16:274`, `16:354` |
| air_wing_pool | `0x22CDEC24220` | `0xd8` | `0x22CDE51B8F0` | `0x29bbe58` | owner `16` only, region `1`, wings `16:110`, `16:274`, `16:354` |
| air_wing_pool | `0x22CDEC26B10` | `0xd8` | `0x22CDE51B770` | `0x29bbe58` | owner `16` only, region `1`, wings `16:110`, `16:274`, `16:354` |
| air_wing_pool | `0x22CDEC29090` | `0x88` | `0x22CDE51B930` | `0x29bbe58` | owner `16` only, region `1`, wings `16:110`, `16:274`, `16:354` |

Do not retry this exact bounded direct-reference route as a full event root
without a new anchor or a changed criterion. Useful next roots would need to
come from a different evidence source, such as an event hook/function xref,
combat-resolution disassembly, a proven active ace assignment frame, or a wider
root chain outside the scanned manager/container/pool fields.

## Current validated slice, not full completion

The integrated compact delta slice already has production-shaped proof in:

- `replay_native/runs/air_combat_losses_live_integrated_20260701_v1/native_session_manifest.json`;
- `replay_native/runs/air_combat_losses_live_integrated_20260701_v1/native_session.battleplan`;
- `replay_native/runs/air_combat_losses_live_integrated_20260701_v1/collector_stream.bin`.

That run scheduled `air_combat_losses_live`, advanced `96` game hours, saw `0`
skipped hours, emitted `97` domain data records and `97` domain perf records,
decoded an ordered compact stream, saw no source-unavailable, partial-data,
bridge-unavailable, or worker-panic flags, and packed a `.battleplan`.

This proof accepts only the strength/equipment delta slice. It does not close
sortie lifecycle, combat pairing, kill attribution, cause/source of loss,
tactical engagement state, ace participation, or rich reinforcement reason.

# A26 non-suspending production source

## Decision

The shipped A26 recorder must not patch the tooltip controller entry at
`hoi4.exe+0x002A4080`. MinHook code mutation is not admitted because MinHook
temporarily suspends process threads while enabling and disabling hooks.

The exact replacement is the `CInGameIdler` update vtable slot:

- vtable: `hoi4.exe+0x02952510`
- slot: `hoi4.exe+0x02952530` (`vtable+0x20`)
- expected target: `hoi4.exe+0x00DBF240`
- ABI: `void(this, u8)`
- RTTI name: `.?AVCInGameIdler@@`

The slot target calls the exact tooltip controller at `hoi4.exe+0x002A4080`.
The observer calls the original slot target first and then copies the final
tooltip owner, text, rectangle, viewport, authored identity and QPC into
pointer-free custody. The slot is redirected only through the shared
authenticated atomic image-pointer primitive; A26 does not perform a raw CAS,
write code bytes, suspend a thread, pause the game, or send client input.

## Coverage

The controller has exactly three static code callers in exact a729:

| Caller | Vtable owner | Production disposition |
|---|---|---|
| `hoi4.exe+0x00B2D8F0` | `CFrontEndIdler` | excluded: front end |
| `hoi4.exe+0x00DBF240` | `CInGameIdler` | admitted production source |
| `hoi4.exe+0x012BF910` | `CNudgeIdler` | excluded: nudge tooling |

Hourly capture runs only in the in-game idler. Redirecting the other two slots
would add hooks without adding any production observation.

## Exact evidence

- FunctionEvidence job:
  `20260728T234737Z-nonsuspend-power-tooltip-source-closure-0eafceda`
- FunctionEvidence manifest SHA-256:
  `d1cd35e661d9184f99e7efa3c8a7a92c5cf5976fd15df10bc449804c5f1bdb44`
- `CInGameIdler` vtable dump:
  `20260728T235015Z-tooltip-vtable-rtti-and-slots-5245dcb1`
- vtable dump artifact SHA-256:
  `2de0fae45a0a6256d4ee0f2adec8de0e8b9c281d42ab15fc983cab537330c051`
- RTTI type-name dump:
  `20260728T235423Z-tooltip-owner-type-names-5270e48f`
- RTTI artifact SHA-256:
  `ee7c9c0921bc55c7d713931b2c3bd1d754f8a155a2d5bb5d74951efedf4c076d`
- binary identity:
  `hoi4.exe 1.19.2.0.a729`,
  SHA-256 `870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`,
  image base `0x140000000`

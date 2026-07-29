# A37 non-suspending production source

## Decision

The shipped A37 recorder must not patch the terminal power-balance writer at
`hoi4.exe+0x01C4BAA0`. The exact replacement is the
`CCountryPoliticsView` refresh vtable slot:

- view vtable: `hoi4.exe+0x029C2A58`
- refresh slot: `hoi4.exe+0x029C2B38`
- expected target: `hoi4.exe+0x01570510`
- ABI: `void(this)`
- owned power-balance controller pointer: `*(view+0x7A50)`
- required controller vtable: `hoi4.exe+0x02A421C0`

The refresh slot target calls the final power-balance update chain. The
observer calls the original refresh first, revalidates the view and owned
controller, and then reads the final controller leaves. Slot redirection uses
only the shared authenticated atomic image-pointer primitive. A37 does not
perform a raw CAS, write code bytes, suspend a thread, pause the game, or send
client input.

## Coverage

FunctionEvidence reports no code caller for `hoi4.exe+0x01570510`; its only
semantic data reference is the `CCountryPoliticsView` vtable slot above. The
other data reference is PE unwind metadata and is not a dispatch pointer.
Therefore the single authenticated slot covers the production refresh
dispatch without adding a broader hook.

The view owns the current controller directly at `+0x7A50`. The exact accessor
at `hoi4.exe+0x0156BA90` returns the pointer word unchanged, so the observer
uses a read rather than calling engine code.

## Exact evidence

- country-politics caller job:
  `20260728T235113Z-power-country-politics-root-callers-d0de9be2`
- caller manifest SHA-256:
  `75e5ad668779f9ed64a4dd4b22be71a5d4651a1d61a3d1ffe283979ec5d09598`
- controller accessor job:
  `20260728T235149Z-power-controller-owned-pointer-accessors-90a524dd`
- accessor manifest SHA-256:
  `2a82828205188ac3a1d578bca9d3d31c3bc8db023962ebb24e5da25ef052ac32`
- terminal writer/source-closure job:
  `20260728T234737Z-nonsuspend-power-tooltip-source-closure-0eafceda`
- source-closure manifest SHA-256:
  `d1cd35e661d9184f99e7efa3c8a7a92c5cf5976fd15df10bc449804c5f1bdb44`
- binary identity:
  `hoi4.exe 1.19.2.0.a729`,
  SHA-256 `870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`,
  image base `0x140000000`

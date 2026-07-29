# A37 non-suspending production source

## Decision

The shipped A37 recorder must not patch the terminal power-balance writer at
`hoi4.exe+0x01C4BAA0`. A post-only wrapper around the
`CCountryPoliticsView` refresh slot is also insufficient: it cannot prove the
inner writer ran and loses the writer's transient `full_refresh` and
`range_refresh` arguments.

The exact replacement is a correlated pair of object-scoped vtable slots:

- primary view vtable: `hoi4.exe+0x029C2A58`
- active-predicate slot: `hoi4.exe+0x029C2A80`
- expected predicate target: `hoi4.exe+0x01222470`
- `+0x30` interface vtable: `hoi4.exe+0x029C2B00`
- refresh slot: `hoi4.exe+0x029C2B38`
- expected target: `hoi4.exe+0x01570510`
- ABI: `void(this)`
- normalized outer view: `raw_refresh_this-0x30`
- owned controller: `*(raw_refresh_this+0x7A20)`, equivalently
  `*(outer_view+0x7A50)`
- required controller vtable: `hoi4.exe+0x02A421C0`

The refresh wrapper establishes a stack-safe thread-local correlation frame
and authenticates the normalized outer view. The scoped predicate wrapper
requires `this` to equal that exact outer view, calls its original, and records
exactly one AL-semantic boolean return for the top refresh frame. Immediately
after the original predicate returns, that wrapper resolves and authenticates
the owned controller and captures its active byte at `+0x30` plus the old
selected-record and selected-range pointers at `+0x598` and `+0x5A0`.

This is the latest available correlated pre-writer boundary. In
`hoi4.exe+0x01570510`, the controller member at raw `this+0x7A20` is not
addressed until `0x141570B11`; the only loads are the accessors immediately
feeding the active check at `0x141570B20` and writer call at `0x141570B3A`.
No intervening prefix/body call receives the controller pointer. The wrapper
also requires the callback to remain on the refresh owner thread.

After the original refresh returns, the wrapper requires the same normalized
outer view, the same controller pointer and vtable, and a still-active
controller. This last check matters: `hoi4.exe+0x01C4A860` closes the
controller and returns without calling the terminal writer when no current
balance exists. The wrapper admits the inner writer only when the predicate
was observed true and the controller-active byte was true both before and
after the refresh.

The refresh calls `hoi4.exe+0x01C4A860` with `force=0`. Therefore, after the
original refresh returns, the exact terminal arguments are:

- `full_refresh = new_selected_record != old_selected_record`
- `range_refresh = new_selected_range != old_selected_range`

The terminal writer reads `+0x598` and `+0x5A0` but does not write either
field, and the refresh suffix after `0x141570B3A` does not address the
controller again. The post-refresh values therefore remain the exact values
compared by `hoi4.exe+0x01C4A860`.

Both slots use only the shared authenticated atomic image-pointer primitive.
A37 does not call the predicate an extra time, perform a private CAS, write
code bytes, suspend a thread, pause the game, or send client input.

## Coverage

FunctionEvidence reports no code caller for `hoi4.exe+0x01570510`; its only
semantic data reference is the `CCountryPoliticsView` interface vtable slot
above. The other data reference is PE unwind metadata and is not a dispatch
pointer.

RTTI binds the primary and `+0x30` secondary vtables to
`.?AVCCountryPoliticsView@@`. The primary `+0x28` predicate checks the child
at `outer_view+0x578`, invokes that child's existing `+0x248` virtual method,
and only then reads visibility bit `0x08` at child `+0xA5`. It is not safely
replaceable with a plain field read. Correlating the original predicate return
captures the real admission decision without introducing another engine call.

Promotion additionally requires a representative speed-five
callback-to-selected-Present fixture. Static slot identity alone does not
prove that every selected Present receives the required observation.

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
- interface-vtable neighborhood job:
  `20260729T002144Z-a37-interface-vtable-neighborhood-5a5055fa`
- interface-vtable artifact SHA-256:
  `415385ae39b6588a6a6819099ec29d338a081a56ff980c159bb3c9bec75b3fc1`
- `CCountryPoliticsView` RTTI group job:
  `20260729T002250Z-a37-country-politics-rtti-group-34ae173e`
- RTTI artifact SHA-256:
  `939ac8cc60184ce7e9f9ee6f371e9c8d0a7eaa1fbd3deecbb36498acf583e3c8`
- primary-vtable locator scan:
  `20260729T002319Z-a37-primary-vtable-col-scan-d293952b`
- primary-vtable scan artifact SHA-256:
  `1a45da607bab0c55f578a71436626525a2924eab924ef8f5bb88d5319bfdf33c`
- predicate/refresh semantic job:
  `20260729T002330Z-a37-outer-admission-and-refresh-semantic-c626e383`
- predicate/refresh manifest SHA-256:
  `da79d15b95f478b3e2d3890e794f89076562f43c733da81f3e5e9bf11d3294b5`
- binary identity:
  `hoi4.exe 1.19.2.0.a729`,
  SHA-256 `870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`,
  image base `0x140000000`

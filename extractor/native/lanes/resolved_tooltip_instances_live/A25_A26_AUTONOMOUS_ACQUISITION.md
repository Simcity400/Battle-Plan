# A25/A26 autonomous live acquisition

The remaining A25 owner-glyph observation and A26 nine-case tooltip matrix can
be acquired without a human choosing or interpreting UI state. The acquisition
is deliberately split across two owners:

- the custom in-process harness owns hook installation, hook epochs, raw
  pointer lifetime, selected-Present sealing, and create-new raw receipts;
- `Invoke-A25A26AuthenticatedActions.ps1` owns only exact-a729 process/window
  authentication, deterministic `SendInput`, exact client-size transitions,
  and create-new action signals.

Neither side can promote itself. `verify_a25_a26_acquisition.py` independently
regrades the assembled A26 matrix, validates A25's mixed ordinary-glyph and
inline-icon owner join, checks every action-to-raw-receipt hash, and emits one
portable content-addressed admission receipt. Production activation still
requires a reviewed source change and root-owned central Rust validation.

## Harness protocol

The orchestrator launches the harness with these process-local variables:

| Variable | Meaning |
|---|---|
| `BATTLE_PLAN_A25_A26_GAME_EXE` | exact-a729 `hoi4.exe` |
| `BATTLE_PLAN_A25_A26_COLLECTOR_DLL` | create-new session copy of the collector |
| `BATTLE_PLAN_A25_A26_OUTPUT_DIR` | empty acquisition root |
| `BATTLE_PLAN_A25_A26_ACTION_PROFILE` | authenticated action profile |
| `BATTLE_PLAN_A25_A26_ACTION_SIGNAL_DIR` | create-new signal directory |
| `BATTLE_PLAN_A25_A26_A25_RECEIPT` | required A25 raw receipt path |
| `BATTLE_PLAN_A25_A26_A26_CASE_DIR` | required A26 raw case directory |

The harness watches `action_signals/*.json` in ordinal order. It must reject a
gap, duplicate, wrong process epoch, wrong profile hash, wrong exact viewport,
or unknown command. For an A25 signal it writes exactly the schema in
`a25_owner_glyph_receipt.schema.json`. For an A26 signal it writes
`a26_cases/<case_id>.json` in the existing raw case shape consumed by
`capture_a26_same_present.py`. Files are create-new; a retry uses a new empty
acquisition directory.

The two nontrivial harness commands are explicit:

- `seal_selected_owner_glyph_present` selects the current render candidate,
  admits its A25 owner capture, and copies its pointer-free receipt.
- `reinstall_owner_then_seal_current_controller_observation` removes the A26
  owner, proves quiescence, installs a fresh owner while the tooltip is already
  visible, waits for a controller observation, and seals that same selected
  Present. It is the capture-start-visible case, not an inferred copy of the
  ordinary show case.

The action profile uses a 1918x1078 to 1920x1080 client-size round trip to force
the global tooltip owner through destruction/recreation, then requires A26's
owner generation to prove the lifecycle. The final 2560x1440 case is a real
authenticated client resize and is rejected unless five consecutive client
geometry reads agree. Coordinates are never scaled implicitly.

## Invocation

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass `
  -File extractor\native\scripts\Invoke-A25A26TooltipGlyphAcquisition.ps1 `
  -HarnessExe <custom-owner-harness.exe> `
  -Hoi4Exe '<local-path> of Iron IV\hoi4.exe' `
  -CollectorDll .cargo-target\release\collector.dll `
  -OutDir <local-path><timestamp> `
  -HarnessArgument @(<harness-specific launch arguments>)
```

Success is only
`a25_a26_acquisition_admission.json` with `status=PASS`. Screenshots, OCR,
action success by itself, an unassembled case set, or a raw receipt that has
not crossed the independent verifier remain non-authoritative.

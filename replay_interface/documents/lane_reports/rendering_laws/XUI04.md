# XUI04 ? widget visual states

Status: **PROVEN / PRODUCTION_PROMOTION_READY_FAIL_CLOSED**

Authority: `hoi4.exe 1.19.2.0.a729`, image base `0x140000000`, SHA-256 `870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`.

## Closed scope

XUI04 now has a typed production admission law for every visual-state carrier in the effective GUI namespace: authored/static, frame, hover, pressed, disabled, selected, active, toggle, attention, progress, drag, and mutation disposition. The XUI01 census contributes all 49 case-preserved construct types and 27,135 parsed elements across 454 effective GUI files. They normalize to 18 widget families with 23,472 occurrences; the other 3,663 generic constructs are preserved and never coerced into widgets.

The machine-readable matrix has 216 cells (18 families by 12 state columns). Every cell is either an exact admitted carrier, an explicit producer-resolved carrier, an inapplicable state, or a rejection. There are no unresolved or capture-dependent cells.

## Exact promotion law

Standard buttons, GUI buttons, and scrollbar compound buttons consume the proven UI02 state carrier exactly:

| State | Raw | One-based frame |
|---|---:|---:|
| Up | 0 | 1 |
| Down / pressed | 1 | 2 |
| Over / current hover target | 2 | 3 |
| Disable | 3 | 4 |

An Over object that is not the current hover target draws frame 1. Frame indices are one-based at the contract boundary and must be within the authored frame count.

Selected, active, and toggle are never guessed from names, booleans, or strip order. They require a producer-resolved one-based frame. This is exact rather than partial: the installed corpus proves that checkbox strips use both two and four frames, so a universal boolean-to-frame mapping does not exist.

Attention admits the exact UI01 `active_goal/goal_icon` event or a separately provenance-bound producer snapshot. Drag admits only UI06/UI07 provenance-bound legacy or extended scrollbar snapshots. Missing, empty, non-finite, out-of-range, wrong-family, or disabled-state-mismatched inputs reject.

All replay presentations are read-only. Navigation controls may mutate viewer state; `allows_game_mutation` is always false. Recorded mutation widgets must be inert snapshots or exact disabled presentations.

## Progress effects

The installed shaders prove seven typed effects:

- Linear and StartEnd use foreground when `u <= CurrentState`; StartEnd additionally uses its shipped three-row cap sampling.
- Reverse uses foreground when `u >= 1 - CurrentState`.
- MinMaxColor and MinMaxTexture preserve their exact half-domain inequalities.
- Radial uses the shipped angle carrier and saturated 50x transition.
- Circular uses `vTextureToRender.x` as its texture-two blend carrier, not `CurrentState`.

The existing generic compositor's CPU width crop is not promoted as a substitute for StartEnd, Reverse, MinMax, Radial, or Circular. Those effects must consume `game_ui::widget_visual_state::ProgressFill`. This prevents the old approximation from being mistaken for the production law.

## Production integration

The exported entrypoint is `game_ui::widget_visual_state::admit_widget_visual`. It consumes the shared XUI07 `runtime_graphics::ButtonVisualState`, returns only bounded frame selections and provenance-bound carriers, and rejects unsupported combinations.

Focused gate:

```text
cargo test --manifest-path replay_interface/native/Cargo.toml -p game_ui --test xui04_widget_visual_state
```

The packet verifier rebuilds the game-file census and promotion contract, checks all 49 constructs exactly once, checks all 216 matrix cells, runs the focused gate, exercises fail-closed source invariants, and freezes a deterministic evidence manifest. No Ghidra call, new capture, adversarial review, or PROGRAM_STATUS edit was made.

Evidence: `replay_interface/re_evidence/rendering_laws/XUI04/`.

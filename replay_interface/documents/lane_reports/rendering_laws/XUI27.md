# XUI27 - final replay UI composition audit

Status: **BLOCKED - NOT PRODUCTION PROMOTABLE**.

This is the strongest offline result possible from the consolidated checkout
and retained evidence on 2026-07-19. No the game process was launched. The gate
does not promote sample canvases, packet-specific hook traces, or replay
self-determinism into engine pixel parity.

## Current product result

The current normal product route has code paths only for XUI24, XUI25, and
XUI27's four authenticated HUD roots: `topbar`, `mini_map_interface`,
`MapmodesInterface_Ingame`, and `theatre_selection_window`. Twenty-four
historical packets are omitted from product pixels. That fail-closed subset is
useful, but it is not a complete the game UI. XUI25's route is not currently
admissible in real recorder output: the production input-family observer and
collector bind are inactive, so d77 lacks the required input receipt.

The current installed-file probe parses 162 GFX files / 25,379 sprites and 453
GUI files / 9,153 windows / 27,123 elements with zero parser failures. The HUD
sample emits 183 commands with zero missing or undecodable sprites. Eight major
view samples also decode their referenced sample assets, but every one records
runtime-populated collections or hidden-state skips. They are offline
parser/compositor coverage, not authenticated product panels.

The focused product renderer is green at 11/11. The complete `game_ui` run is
not green: its library tier passed 134 tests with two ignored, but
`diplomacy_binding::measured_nonempty_real_fold_without_country_names_keeps_d53_roster`
failed because the concurrently changing extractor could not compile
`recorder_host`. The exact errors were missing
`ControlCapabilities::auto_attach_duplex_v1` and a `PackIdentity` initializer
without `content_manifest`.

## Recursive evidence result

`recursive_upstream_audit.json` validates 458 path/hash/byte or Ghidra-manifest
entries and 107 status values across XUI01-XUI26, after treating CRLF checkout
conversion as the same Git-normalized text. It still finds 165 real stale or
unresolved bindings. Shared integration files such as `world.rs`, `lib.rs`,
`schema.py`, recorder folding, and packet-specific sources have changed after
their packet manifests were frozen. Therefore the historical packet statuses
cannot be used as a current recursive promotion proof.

The independent production UI promotion manifest is also stale. Its route
partition still parses, but 12 artifact pins no longer match the consolidated
checkout. Several expected hashes never existed in Git history; they describe
pre-commit working bytes rather than the committed integration state.
`production_authority_audit.json` records expected, current raw, and current
LF-normalized SHA-256 values for every mismatch.

## Retained engine/capture evidence

The old XUI27 audit searched only the RenderDoc and preserved-map roots and
incorrectly reported zero synchronized CPU state. The canonical
`<local-path>` store has 77 relevant sessions: 76
with `cpu_state.jsonl`, 24 with UI-pass bundles, 42 with RDCs, nine with
screenshots, and 34 with manifests.

The strongest retained UI session is
`20260717T013524Z-ui13-human-final`: its 1,895,403,256-byte producer trace is
SHA-256 `147248c4be2dca0f35ede4dfe7d746e69ce498213fb24268c8f6263c4ccb4f83`.
Its three action markers do not form three clean engine image joins. The
decision-open marker points to a Codex-window screenshot; the clicked-decision
image is occluded by a TeamViewer notice; only the war-open marker points to a
clean the game image. A separate clean decision screenshot exists in the same
session but is not the file named by its action marker. The producer trace is
authenticated packet-specific formatter/setter evidence, not a complete replay
`WorldFrame`.

An exhaustive filename scan of the canonical evidence root finds zero
`.battleplan` files paired to any engine UI image. Consequently an honest
engine-vs-replay pixel diff cannot be computed: the pixels alone do not recover
all dynamic values, visibility decisions, list order, modal/focus/tooltip state,
or the XUI25 input envelope. The former `map_hud_topbar.png` and
`map_modal_production.png` are superseded; they composited replay UI over an
engine frame that already contained UI and are not parity proofs.

The retained-corpus parity utility cannot close this gap: its own command-line
contract is explicitly **non-UI** and admits only byte-bound event-scoped
pass-final pairs. The map parity scoreboard is likewise not a UI composition
oracle. Neither was repurposed to manufacture XUI evidence.

## Exact remaining human input

There is one XUI27 human card, frozen in `human_input_requirements.json`:
`XUI27-H1-SAME-PRESENT-ENGINE-REPLAY-PAIR`. It requires a lossless engine final
UI image and the matching flushed production `.battleplan`/`WorldFrame` at the
same Present boundary, plus the exact window/modal/focus/tooltip state,
selection/map-mode state, ordered content identities, and XUI25
display/language/input key. A screenshot alone does not satisfy it.

The minimum boundary set is: measured HUD; panel/modal; tooltip above the
panel; every dynamic leaf unavailable; and one selectively unavailable leaf
with measured peers retained. Closure requires a mechanical engine-vs-replay
pixel diff with exact mismatch coordinates and counts.

## Machine-readable authorities

- `compose_contract.json` - fail-closed result and current test disposition.
- `recursive_upstream_audit.json` - all 26 packet manifests and 165 failures.
- `production_authority_audit.json` - 12 stale central promotion pins.
- `engine_oracle_audit.json` - complete retained-root/session census and the
  authenticated UI13 join.
- `human_input_requirements.json` - the single exact capture card.

No new Ghidra operation was needed for this composition audit. Packet-level
Ghidra provenance is recursively inspected through the existing manifests;
XUI27 function address, job ID, and new export manifest are `N/A`.

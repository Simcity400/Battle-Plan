# Battle Plan / Replay

Battle Plan passively records supported Hearts of Iron IV campaigns into portable
`.battleplan` files and plays them back as an interactive reconstruction of the
game.

The recorder automatically finds running games, captures changing simulation
state without pausing or controlling HOI4, preserves reload and relaunch branches,
and accounts for unreadable periods as explicit gaps instead of discarding the
recording. The viewer authenticates the recording and reconstructs the map and
observer UI from the exact game, DLC, mod, language, and settings content the
campaign used. Unsupported builds or incompatible content are refused rather than
guessed.

`PROJECT.md` holds the product rules. `AGENTS.md` holds the rules for agents.

## Architecture

```text
hoi4.exe
  -> passive injected collector/coordinator
  -> typed compact records and explicit gaps
  -> external recorder host, validation, fold, content binding and packaging
  -> .battleplan
  -> authenticated Replay ingest and typed WorldTimeline
  -> shared map and authored observer-UI composition
  -> windowed, headless or benchmark presentation
```

### Extractor

`extractor/` owns process discovery, build admission, passive attach and teardown,
the injected collector, domain scheduling, compact record production, cadence and
coverage accounting, campaign and session composition, content binding, folding,
and replay publication.

One resident coordinator watches the current Windows session and supervises one
independent worker per proven HOI4 process. Candidate identity includes PID and
creation time so PID reuse cannot join attempts. Each attempt uses isolated
control, pipe, staging, collector, output, and teardown identities. Heavy
finalisation is serialised so it cannot starve live capture.

The collector observes in process and streams bounded compact data. The external
host authenticates the stream, accounts for every scheduled domain at each
accepted boundary, folds through file-backed hourly state, packages beside the
destination, reopens and verifies the candidate, and only then publishes it
atomically. A crash may preserve a verified prefix; unverifiable or truncated
material remains a named failed artifact.

### Portable boundary

`portable_replay/` owns the neutral, versioned format crates statically linked by
both products. Neither installed product requires the other's private types or
process. Unknown domains and fields may remain reachable for forward inspection,
but an unknown value is never admitted as a known carrier.

### Replay

`replay_interface/` owns container admission, certified pre-fold ingest, the typed
timeline, playback state, map and UI composition, interaction authority, headless
rendering, benchmarks, and package verification.

- `replay_core` authenticates ingest and exposes typed timeline state and provenance.
- `map_viewer` owns effective-game-file map providers, reference rendering, and the
  shared non-UI composition graph.
- `game_ui` owns authenticated GUI, GFX, font, localisation, authored observer UI,
  and hit geometry.
- `replay_app` owns lifecycle, window and swapchain, transport, selected country,
  mapmode and panel state, headless output, benchmarks, and Present authority.

Windowed playback, headless frame export, and the recording benchmark all call
`ProductMapCompositionSession::execute_on_target` and run the same composition
routes. Content admission completes before device or UI setup. Input and hit
authority advance only for the exact successfully presented generation. Seek,
rollback, discontinuity, occlusion, failed Present, or a country switch cannot
leave undisplayed or wrong-country interaction state active.

## Machine-readable ownership

These artifacts own enumerated truth and are never copied into Markdown tables:

- `extractor/native/domain_registry.json`: domain identity, production status, and
  capture scheduling.
- `replay_interface/re_evidence/rendering_laws/XUI27/external_producer_dependencies.json`:
  the external producer-to-UI consumer boundary, unavailable behaviour, evidence,
  and source-test bindings.
- `replay_interface/re_evidence/rendering_laws/XMAP00/non_ui_visual_input_ledger_20260730.tsv`:
  the non-UI map input and disposition boundary.
- Portable format and schema crates and their tests: binary and container grammar,
  limits, identity rules, and reader/writer compatibility.
- Evidence manifests under `extractor/**/re_evidence/`, `extractor/native/runs/`,
  and `replay_interface/re_evidence/`: build, artifact, packet, hash, and
  validation receipts.

These artifacts may prove a structural binding or fixture result without proving
that a real producer worked, a real capture was lossless, or the final framebuffer
matched HOI4. Claims stop at the boundary their evidence exercised.

## Distribution

The product ships as one offline, unsigned portable Windows ZIP containing the
desktop recorder, replay viewer, the recorder's collector DLL, user instructions,
and checksums. It has no installer, service, tray-only lifecycle, automatic
updater, network downloader, administrator requirement, publisher-signature gate,
or guessed install paths. Checksums detect corruption; they are not publisher
authentication.

Only an unfiltered central release of both native workspaces may produce a
promotable package. Promotion requires the exact current artifacts, a real passive
maximum-speed capture with zero loss across the production roster, authenticated
READY and teardown (restored hooks, unloaded collector), replay open and rendering
gates, closed-package byte verification, and a canonical receipt bound to all of
those inputs. Direct or filtered builds are diagnostic and cannot claim promotion.

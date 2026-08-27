# Project contract

This is the single canonical source for durable project truth: product
requirements, architecture, invariants, decisions, boundaries, ownership, and
definition of done. `README.md` describes the product to users. `AGENTS.md`
governs agent conduct and validation. Code, schemas, registries, tests, and
structured evidence own executable detail; no other Markdown file is project
authority.

Mutable completion state, task history, review dialogue, roadmaps, and lane
ledgers do not belong here. Git preserves history. Machine-readable registries
and evidence stores preserve current enumerations and proof.

## Product

Battle Plan consists of a recorder that observes a the game session and
produces a portable `.battleplan` file, plus a viewer that plays it back. It must
work without per-campaign setup for every supported game build and for arbitrary
valid combinations of mods and DLC.

Sound is part of the reconstructed presentation: playback derives world audio
(battle and ambient sound) and music viewer-locally from recorded authoritative
state and authenticated content. Audio is never captured and never a rendering-law
input. Each sound class ships only once its engine or game-file rule is proven and
its recorded inputs are present; a class whose rule or inputs are incomplete is
silent and named, never approximated into something the game would not play.

The intended experience is attach-and-play: start the recorder and play normally.
It identifies each running game and campaign, records through relaunches and
reloads, and produces the replay without asking for a PID, save, DLL, duration, or
output path. Playback reconstructs the game scene and observer interface rather
than presenting a statistical summary.

The product priorities, in order, are:

1. Never make the game wait or change its behaviour.
2. Preserve truthful, replayable state and name every loss.
3. Reconstruct the game's presentation faithfully from authenticated content.
4. Minimise capture overhead and recording size without sacrificing the first
   three priorities.

## Non-negotiable invariants

### Passive recording

- The production recorder never pauses the game, changes speed, writes simulation
  state, drives the UI, sends input, changes focus, or depends on what the player
  has selected or opened. Speed remains player-owned, including maximum speed.
- Injection is permitted as an observation and transport mechanism. The injected
  collector has no authority to control the client.
- Readable state is sampled passively. A hook is permitted only for a value proven
  unsampleable, through the single audited instrumentation primitive. Every hook
  site is registered, justified, bounded, and restored byte-for-byte at teardown.
- Bootstrap instrumentation needed to establish an observation boundary is held
  to the same registration, identity, and teardown rules. Evidence-only workflows
  are physically separate from the shipped recorder and cannot certify production
  behaviour.
- The production hook set is derived from scheduled consumers. A descheduled or
  optional companion domain installs no producer detour unless a scheduled domain
  requires that exact shared site; the exception is site-specific and independently
  proven absent/restored rather than inherited by the whole domain.
- An unsupported executable build records nothing and gives a specific reason.
  The recorder never guesses offsets, layouts, counts, or identities.

### Loss and coverage

- Once a campaign is admitted and storage remains writable, capture failures do
  not erase the recording. A bad hour costs that hour, with a named reason; it
  never costs the recording.
- A missed, partial, torn, incoherent, overloaded, or unavailable observation is a
  typed gap with its time range, affected domain or feature, cause, and witness.
  Silence means witnessed unchanged state, never an observation that did not run.
- Sweeps are tick-bracketed. A sweep spanning a tick boundary is a gap, not a
  coherent frame. State after a gap resumes from a complete baseline.
- Hourly sampling is the baseline. A domain may require finer cadence only when
  its exact behaviour or rendered result proves hourly sampling inadequate.
- Structural reach across mods and DLC outranks added depth. Production validation
  must exercise the real attach, capture, sink, teardown, and maximum-speed
  boundary. Package or source inspection alone is not capture proof.

### Fidelity and truth states

- The replay preserves absent/not-applicable, explicit null or unobtained,
  measured empty, zero, unknown, and measured value as distinct states. Only the
  owning producer contract may claim measured empty.
- No consumer substitutes a plausible value for unavailable data. Forbidden
  substitutes include another country or perspective, owner for controller,
  history state for current state, zero, an empty collection, a neutral visual,
  a representative or nearest object, a first row, a centroid, recorder
  selection, heap scans, hard-coded game content, pixels, or visual tuning.
- Exact values that determine an outcome or visible interpolation remain exact.
  Quantisation is permitted only when evidence proves that it changes neither
  meaning nor appearance.
- The capture records changing semantic state and justified divergences, not
  authored content already available in the certified effective installed content. It
  never records screenshots, framebuffer pixels, or map draw state.
- POV-local presentation state is outside production capture. Tooltips, open
  panels, cursor state, local UI selection, and recorder widgets are reconstructed
  from authoritative world state and viewer-owned state or are named unavailable.
  The structured domain registry owns the exact scheduled/descheduled roster and
  manifests publish descheduling positively rather than implying it by absence.

### Size and performance

- The production hot path is native DLL/Rust code. Python, capture corpora, and
  reverse-engineering tools are development and evidence facilities, never the
  hourly or sub-hourly production path.
- Recording is delta-first and cross-hour compressible: stable state is silent,
  identities are interned, repeated authored content is not copied, and memory is
  bounded by current fold state rather than campaign duration.
- File size is minimised only after fidelity and truthfulness. A ceiling is not a
  budget to spend, and no size target authorises dropping a field, merging truth
  states, reducing necessary cadence, or losing branch/gap evidence.
- Production and debug profiles may differ in instrumentation and retained
  forensics, but they share readers, schemas, fold semantics, and fail-closed
  outcomes. Debug artifacts are not distribution payloads.

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
coverage accounting, campaign/session composition, content binding, folding, and
replay publication.

One resident coordinator watches the current Windows session, reconciles process
events with enumeration, and supervises one independent worker per proven the game
process. Candidate identity includes PID and creation time so PID reuse cannot join
attempts. Each attempt uses isolated control, pipe, staging, collector, output, and
teardown identities. Simultaneous games remain independent. Heavy finalisation is
serialised so it cannot starve live capture.

The collector observes in process and streams bounded compact data. The external
host authenticates the stream, accounts for every scheduled domain at each accepted
boundary, folds through file-backed hourly state, packages beside the destination,
reopens and verifies the candidate, and only then publishes it atomically. A crash
may preserve a verified prefix; unverifiable or truncated material remains a named
failed artifact and is never promoted as a replay.

### Portable boundary

`portable_replay/` owns the neutral, versioned format crates statically linked by
both products. The `.battleplan` file is the only installed runtime interface
between Extractor and Replay; neither installed product requires the other's
private types or process.

Format changes land in the portable crates first and update all writers and readers
in lockstep. Replaced implementations and compatibility paths are deleted in the
same change unless the owner explicitly authorises a compatibility obligation.
Unknown domains and fields may remain reachable for forward inspection, but an
unknown value is never admitted as a known carrier.

The container and every referenced payload are strictly parsed, bounded, and
content-addressed. Membership, canonical manifest identity, lengths, hashes,
campaign/content identities, branch topology, indexes, hours, and sequences must
agree before exposure. Payload access remains authenticated; a failed check exposes
no bytes as trusted state. Publication is build-beside plus atomic replace and
never destroys the previous valid file or source material on failure.

### Replay

`replay_interface/` owns container admission, certified pre-fold ingest, the typed
timeline, playback state, map and UI composition, interaction authority, headless
rendering, benchmarks, and package/runtime verification.

- `replay_core` authenticates ingest and exposes typed timeline state and
  provenance.
- `map_viewer` owns effective-game-file map providers, reference rendering, and
  the shared non-UI composition graph.
- `game_ui` owns authenticated GUI, GFX, font, localisation, authored observer UI,
  and hit geometry.
- `replay_app` owns lifecycle, window/swapchain, transport, selected country,
  mapmode and panel state, headless output, benchmarks, and Present authority.
- Capture and reverse-engineering tools are diagnostic inputs to development, not
  playback prerequisites or runtime authorities.

Windowed playback, headless frame export, and the recording benchmark call
`ProductMapCompositionSession::execute_on_target` and execute the same Replay-owned
composition routes. Content admission completes before device or UI setup. A
packet or feature with invalid identity, completeness, hour, sequence, content, or
required state fails closed locally; unavailable is never rendered as measured
empty.

UI consumes the current typed world frame, the authenticated effective VFS, and
viewer-owned interaction state. Input and hit authority advance only for the exact
successfully presented generation. Seek, rollback, discontinuity, occlusion, failed
Present, or a country switch cannot leave undisplayed or wrong-country interaction
state active.

Replay never reads live game memory, launches or injects the game, consumes recorder
UI/POV, trusts an unverified game tree, or uses generated inspection caches,
capture corpora, Python tools, Ghidra, or `hoi4_re` as runtime input.

### Reverse-engineering and evidence

Exact-build exported artifacts from the controlled Ghidra broker are the static
reverse-engineering authority. Search indexes and cross-analysis are advisory
discovery aids: a promoted claim cites the exact artifact path, bytes/hash, build
identity, and relevant dynamic or game-file evidence. Cache reuse is opt-in and
requires exact path, size, and SHA-256 identity; mismatch or race falls back to a
fresh authoritative export.

Structured evidence is append-only and content-addressed where practical. It owns
exact domain/packet enumerations, proofs, manifests, and consumer bindings. Narrative
Markdown may explain a narrow local procedure or irreproducible observation, but it
cannot define project policy, mutable status, or a competing ledger.

## Content and mod contract

Every runtime game-file read goes through the recording-authenticated effective
VFS. Resolution preserves exact base-game, DLC, mod/archive, language, layer order,
override, and `replace_path` semantics. No code path may silently fall back to a
bare vanilla file or a compiled content count, identity, palette, layout, or
presentation constant.

The recorder authenticates the attached build and effective content selection,
records a path-redacted portable identity for the content actually required, and
binds each consumed winner by namespace/path, length, and hash. Machine-local
install, Workshop, profile, OneDrive, and temporary paths do not enter the packed
manifest. Mod/DLC counts and displayable identities are data, not constants.

On first use Replay asks for and validates a the game content root and user-data/profile
root; it never guesses Steam or profile paths. It reconstructs only the layers
required by the recording. Extra unused local content is irrelevant and cannot
override a recorded winner. Missing, reordered, mismatched, or changed required
content produces a precise refusal. The replay file may be renamed or moved and
contains no source-machine path.

Static definitions may describe authored content but never replace changing
campaign state. Derived presentation is allowed only when the owning implementation
proves the exact engine/game-file law and possesses every required input.

## Time, identity, sessions, and files

- One canonical tick coordinate spans all domains, branches, gaps, symbols, and
  seek positions. Per-domain cadence is declared against that coordinate.
- Stable machine identities for domains, collections, and fields are append-only
  and never reused for different meanings. Collection order is preserved whenever
  it is semantic.
- The current campaign identity is derived from the engine-persisted
  `game_unique_id` and signed `game_unique_seed`, retaining and validating both raw
  components. Missing, torn, malformed, conflicting, or unstable components never
  produce a fallback identity.
- PID and process creation time identify an attachment lease and session, not a
  campaign or output file.
- One campaign is one logical replay file across recorder/game restarts. Time
  rollback starts a branch/segment within the same campaign; a changed campaign
  identity starts a different campaign. Reload branches and their transition
  evidence are retained while playback exposes the surviving line.
- Two machines recording the same campaign produce independent files. Two
  simultaneous games on one machine are recorded independently. Modset, date, PID,
  wall clock, bookmark, player roster, or filename alone never authorises a join.
- A session seal does not rewrite the whole campaign. Files are automatically
  named, may be renamed without changing identity, and are deleted only by an
  explicit user action.

## Machine-readable ownership

The following artifacts own enumerated truth and must not be copied into Markdown
tables:

- `extractor/native/domain_registry.json`: domain identity, production status, and
  capture scheduling.
- `replay_interface/re_evidence/rendering_laws/XUI27/external_producer_dependencies.json`:
  the complete external producer-to-UI consumer boundary, unavailable behaviour,
  evidence, and source-test bindings.
- `replay_interface/re_evidence/rendering_laws/XMAP00/non_ui_visual_input_ledger_20260730.tsv`:
  the exhaustive non-UI map input and disposition boundary.
- Portable format/schema crates and their tests: exact binary/container grammar,
  limits, identity rules, and reader/writer compatibility.
- Evidence manifests under `extractor/**/re_evidence/`,
  `extractor/native/runs/`, and `replay_interface/re_evidence/`: exact build,
  artifact, packet, hash, and validation receipts.

These artifacts may prove a structural binding or fixture result without proving
that a real producer worked, a real capture was lossless, or the final framebuffer
matched HOI4. Claims stop at the boundary their evidence exercised.

## Distribution boundary

The product ships as one offline, unsigned portable Windows ZIP containing the
desktop recorder, replay viewer, the recorder's collector DLL, user instructions,
and checksums. It has no installer, service, tray-only lifecycle, automatic
updater, network downloader, administrator requirement, publisher-signature gate,
or guessed install paths. Checksums detect corruption; they are not publisher
authentication.

Only an unfiltered central release of both native workspaces may produce a
promotable package. Promotion requires the exact current artifacts, a real passive
maximum-speed capture with zero loss across the production roster, authenticated
READY and teardown (including restored hooks and unloaded collector), replay open
and rendering gates, closed-package byte verification, and a canonical receipt
bound to all of those inputs. Direct or filtered builds are diagnostic and cannot
claim promotion.

## Definition of done

The project is done only when all of the following are true on current supported
builds and representative vanilla/mod/DLC configurations:

1. The recorder provides both app-first and game-first automatic attach, including
   independent simultaneous games, without controlling or delaying HOI4.
2. Campaign identity, relaunch, reload, rollback, branch retention, and atomic
   publication produce one truthful replay per campaign with no silent join or
   omission.
3. Every scheduled producer either emits complete state at its declared cadence or
   emits an exact typed gap. No missing input is presented as empty, zero, stale, or
   inferred.
4. A real maximum-speed production capture on the physical support floor (at least
   four CPU cores and 8 GiB RAM) has zero loss and meets the capture overhead and
   bounded-memory gates. Affinity-only simulation is a lower bound, not proof of
   the hardware floor.
5. Replay eagerly authenticates and folds the complete certified stream before
   opening, makes declared gap hours unavailable, restores the last viewed
   position, and never changes the admitted fold during later lazy access.
6. Map and observer UI output match the game through engine/game-file-derived laws,
   authenticated effective content, and recorded semantic state, with no captured
   pixels, hard-coded content, tune-to-look constants, or recorder-POV substitute.
7. Content compatibility is exact, portable, path-redacted, mod/archive aware, and
   fail closed. Relocation and unrelated local content do not change playback.
8. The shipping viewer sustains 60 fps average with p95 frame time no worse than
   33.3 ms at 1920x1080 on the reference class (i5-8400 or Ryzen 5 2600, 16 GiB
   RAM, GTX 1060 6 GiB or RX 580 8 GiB).
9. Recording size is demonstrated near its measured information floor; debug
   forensics are excluded from production files, and no remaining duplication or
   static-content copy is justified only by being under a ceiling.
10. The exact portable package passes central validation, real-capture promotion,
    container/content admission, replay map/UI, performance, clean-machine, and
    adversarial review gates with reproducible structured receipts.

Anything requiring an owner decision, a live game, or unavailable external
evidence remains explicitly named and cannot be represented as complete.

# Project contract

Durable product truth: requirements, invariants, and the definition of done. `README.md` describes the product and its layout. `AGENTS.md` governs agent conduct. Code, schemas, registries, tests, and structured evidence own executable detail. No other Markdown is project authority. Task state, history, and roadmaps do not belong here.

## Product

Battle Plan is a recorder plus a viewer. The recorder watches a running the game session and writes a portable `.battleplan` file. The viewer plays it back as a reconstruction of the game scene and observer UI, not a statistical summary. It must work for every supported game build and any valid mod and DLC set, without per-campaign setup.

Attach and play: start the recorder and play normally. It finds each running game and campaign, records through relaunches and reloads, and produces the replay without asking for a PID, save, DLL, duration, or output path.

Sound is derived viewer-side from recorded state and authenticated content. Audio is never captured. A sound class ships only once its engine or game-file rule is proven and its inputs are recorded; otherwise it is silent and named.

Priorities, in order:

1. Never make the game wait or change its behaviour. No pausing, no freeze snapshots, no synchronous in-hook capture, no commit stalls, no writer locks. Maximum speed and auto-pause are normal play.
2. Preserve truthful, replayable state and name every loss.
3. Reconstruct the game's presentation faithfully from authenticated content.
4. Minimise capture overhead and recording size without touching the first three.

## Invariants

### Passive recording

- The production recorder never pauses the game, changes speed, writes simulation state, drives the UI, sends input, changes focus, or depends on what the player has selected or opened.
- Injection is an observation transport only. The collector has no authority over the client.
- State is sampled passively. A hook is allowed only for a value proven unsampleable, through the single audited instrumentation primitive. Every hook site is registered, justified, bounded, and restored byte for byte at teardown. Bootstrap instrumentation follows the same rules.
- The production hook set is derived from scheduled consumers. A descheduled domain installs no detour.
- Evidence-only workflows are physically separate from the shipped recorder and cannot certify production behaviour.
- An unsupported executable build records nothing and gives a specific reason. Support for a new build ships as an app update. The recorder never guesses offsets, layouts, counts, or identities.

### Loss and coverage

- Once a campaign is admitted and storage is writable, a capture failure never erases the recording. A bad hour costs that hour, with a named reason.
- A missed, partial, torn, incoherent, overloaded, or unavailable observation is a typed gap with time range, domain, cause, and witness. Silence means witnessed unchanged state.
- Sweeps are tick-bracketed. A sweep across a tick boundary is a gap. State after a gap resumes from a complete baseline.
- A value belongs to the hour it was read in. No queued, deferred, or cross-hour reads unless the engine proves the value cannot change.
- A producer whose hour is later than its label is refused. Stale is admitted with a declared age. Absent means unknown.
- Every domain reports one outcome: Complete, Partial, Failed, or NotScheduled. Status bits are derived, never combined by hand. A domain without the contract is a compile error.
- Hourly sampling is the baseline. Finer cadence needs proof that hourly is inadequate.
- Reach across mods and DLC outranks depth. Capture proof needs the real attach, capture, sink, teardown, and maximum-speed boundary. Package inspection alone is not capture proof.

### Fidelity and truth states

- The replay keeps absent, null or unobtained, measured empty, zero, unknown, and measured value as distinct states. Only the owning producer contract may claim measured empty.
- No consumer substitutes a plausible value for unavailable data: not another country or perspective, owner for controller, history for current state, zero, an empty collection, a neutral visual, a nearest object, a first row, a centroid, recorder selection, heap scans, hard-coded content, pixels, or visual tuning.
- Two independent reads that disagree are a named outcome. A zero is never invented.
- Exact values that decide an outcome or a visible interpolation stay exact. Quantisation needs proof that it changes neither meaning nor appearance.
- When the engine's behaviour is known, mirror it exactly; never a cheaper or cleaner alternative. Random visuals (effects, animation, palettes, weather) are drawn viewer-side from recorded census. Snow and mud are recorded.
- Capture a value only after proving the replay cannot derive it from authenticated content. The capture records changing semantic state and justified divergences, never authored content, screenshots, pixels, or draw state. No capture may need a person at the keyboard.
- POV-local presentation state (tooltips, open panels, cursor, local selection, recorder widgets) is outside production capture. It is reconstructed from world state and viewer-owned state, or named unavailable. The domain registry owns the scheduled roster; manifests state descheduling positively.
- Fail at the smallest unit and name it. One bad label blanks that label; it does not refuse the frame.

### Size and performance

- The production hot path is native DLL and Rust code. Python, capture corpora, and reverse-engineering tools are development facilities only.
- Capture critical path: at most 16 ms per game hour, at p95 too. Do not fix performance with more worker threads.
- Fold runs inside the recorder at lowest priority. A campaign is ready to replay within 30 s of its last seal. Repeat folds hit a content-keyed cache.
- Recording is delta-first: stable state is silent, identities are interned, authored content is never copied, memory is bounded by fold state, not campaign length.
- Sealed campaign size: under 1 GB now; 50 MB is the goal. Size wins come before compression. No size target authorises dropping a field, merging truth states, reducing needed cadence, or losing branch or gap evidence.
- Production and debug profiles share readers, schemas, fold semantics, and fail-closed outcomes. Debug artifacts are not distribution payloads.

## Format and evidence

- `.battleplan` is the only installed runtime interface between recorder and viewer. Format changes land in the portable crates first and update every writer and reader in the same change. Replaced implementations and compatibility paths are deleted in that change unless the owner authorises a compatibility obligation.
- The container and every payload are strictly parsed, bounded, and content-addressed. Nothing is exposed as trusted state until every identity, length, hash, and sequence agrees. Publication is build-beside plus atomic replace; a failure never destroys the previous valid file.
- Exact-build Ghidra exports from the controlled broker are the static reverse-engineering authority. Indexes and cross-analysis are discovery. A promoted claim cites the artifact path, bytes and hash, build identity, and dynamic or game-file evidence. Narrative Markdown never defines policy or status.

## Content and mods

- Every runtime game-file read goes through the recording-authenticated effective VFS, with exact base, DLC, mod, language, layer order, override, and `replace_path` semantics. No silent fallback to a bare vanilla file or a compiled constant.
- The recorder binds each consumed content winner by namespace and path, length, and hash, with a path-redacted portable identity. Machine-local paths never enter the manifest. Mod and DLC counts are data.
- On first use the viewer asks for a the game content root and a profile root; it never guesses. Missing, reordered, or changed required content is a precise refusal. Unused local content cannot override a recorded winner. The file may be renamed or moved.
- The viewer never reads live game memory, launches or injects the game, trusts an unverified game tree, or uses capture corpora, Python tools, Ghidra, or `hoi4_re` at runtime.

## Time, identity, sessions

- One canonical tick coordinate spans all domains, branches, gaps, and seek positions.
- Machine identities for domains, collections, and fields are append-only and never reused.
- Campaign identity comes from the engine's `game_unique_id` and signed `game_unique_seed`, both raw components retained and validated. No fallback identity.
- PID and process creation time identify an attachment lease, never a campaign or a file.
- One campaign is one logical replay file across restarts. Rollback starts a branch within the campaign; a changed identity starts a new campaign. Reload branches are retained; playback shows the surviving line.
- Modset, date, PID, clock, bookmark, roster, or filename alone never authorise a join. Two machines, or two simultaneous games on one machine, record independently.
- Files are auto-named, may be renamed, and are deleted only by explicit user action.

## Definition of done

All of these are true on current supported builds and representative vanilla, mod, and DLC sets:

1. App-first and game-first automatic attach, including simultaneous games, without controlling or delaying HOI4.
2. Campaign identity, relaunch, reload, rollback, branch retention, and atomic publication give one truthful replay per campaign with no silent join or omission.
3. Every scheduled producer emits complete state at its cadence or an exact typed gap.
4. A real maximum-speed capture on the support floor (4 CPU cores, 16 GB RAM) has zero loss and meets the overhead and bounded-memory gates. Affinity-only simulation is a lower bound, not proof.
5. The viewer authenticates and folds the whole certified stream before opening, makes gap hours unavailable, restores the last position, and never changes the admitted fold later. Seek anywhere is instant, like a video.
6. Map and observer UI match the game through engine and game-file laws, authenticated content, and recorded state: no pixels, no hard-coded content, no tune-to-look constants, no recorder-POV substitute.
7. Content compatibility is exact, portable, path-redacted, mod-aware, and fails closed. Relocation and unrelated local content change nothing.
8. The viewer sustains 60 fps average with p95 frame time at most 33.3 ms at 1920x1080 on the reference class (i5-8400 or Ryzen 5 2600, 16 GiB RAM, GTX 1060 6 GiB or RX 580 8 GiB).
9. Recording size is near its measured information floor; debug forensics stay out of production files.
10. The exact portable package passes central validation, real-capture promotion, container admission, map and UI, performance, clean-machine, and adversarial review gates with reproducible receipts.

Anything needing an owner decision, a live game, or unavailable evidence stays named and is never presented as complete.

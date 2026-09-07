# Project contract

This file owns durable product requirements and completion criteria. [README.md](README.md) describes layout; [AGENTS.md](AGENTS.md) governs agent conduct. Code, schemas, registries, tests, and structured evidence own executable detail and demonstrated status. Other Markdown does not override this contract.

## Product

Battle Plan records campaigns into portable `.battleplan` files and reconstructs the game scene and observer UI for every supported build and any valid mod/DLC set. Start the recorder and play: it discovers games and campaigns through reloads/relaunches without requesting a PID, save, DLL, duration, or output path. Mod/DLC reach outranks depth.

Priorities: (1) never delay or control the game, (2) preserve truthful state and name loss, (3) reconstruct faithfully, (4) reduce overhead and size without compromising the preceding priorities.

## Recording and coverage

- Recording never pauses, changes speed or simulation, drives UI/input/focus, or depends on player selection. No freeze snapshots, synchronous in-hook capture, commit stalls, or writer locks. Maximum speed and auto-pause are normal play.
- Injection only observes. Hooks require proof that sampling cannot obtain the value, use the single audited primitive, and are registered, justified, bounded, and restored byte-for-byte, including bootstrap hooks. Descheduled domains install no detours.
- Unsupported builds record nothing and name the reason; support requires an app update. Never guess offsets, layouts, counts, or identities. Evidence-only workflows remain separate from production.
- After admission with writable storage, capture failure preserves the recording. A bad hour costs that hour. Missed, partial, torn, incoherent, overloaded, or unavailable observations become typed gaps with time range, domain, cause, and witness. Silence means witnessed unchanged state; recovery requires a complete baseline.
- Sweeps are tick-bracketed; crossing a tick creates a gap. Values belong to their read hour; deferred/cross-hour reads require engine proof of immutability. Reject producer data newer than its hour label; declare stale age. Hourly cadence is the baseline; finer sampling requires evidence.
- Domains report Complete, Partial, Failed, or NotScheduled through derived status bits; missing contracts fail compilation. The registry owns scheduling; manifests explicitly record descheduling.

## Reconstruction and truth

- Preserve distinct absent, null/unobtained, measured-empty, zero, unknown, and measured states. Only the owning producer establishes measured-empty. Name conflicting reads; never substitute plausible data, another country/perspective, historical state, or visual approximations for unavailable values. Fail and name the smallest affected unit.
- Mirror proven engine behavior exactly. Preserve values affecting outcomes/interpolation; quantization requires proof of unchanged meaning and appearance. No hard-coded content, pixel substitutes, or tune-to-look constants.
- Record changing semantic state and justified divergences only after proving authenticated content cannot derive them. Never record authored content, screenshots, pixels, draw state, audio, or POV-local UI state; recording needs no keyboard operator.
- Reconstruct panels, tooltips, selection, and other local UI from world/viewer state or name them unavailable. Random visuals use recorded census; snow and mud are recorded. Sound derives from proven engine/game-file rules and recorded inputs; otherwise it is silent and named.
- Windowed, headless, and benchmark output share composition routes. Admit content before device/UI setup; interaction belongs only to the successfully presented generation, including across seeks, rollback, discontinuities, occlusion, failed presentation, and country switches.

## Format and content

- `.battleplan` is the sole installed recorder/viewer interface. Portable crates own format changes; update all readers/writers together and delete replaced/compatibility paths unless owner-authorized. Unknown fields/domains may remain inspectable but never become trusted known values.
- Strictly parse, bound, and content-address containers/payloads; verify identities, lengths, hashes, and sequences before trust. Build publication beside the destination, reopen/verify, then atomically replace; failure preserves the previous valid file. Retain unverifiable/truncated material as named failed artifacts.
- Runtime game-file reads use the recording-authenticated effective VFS with exact base/DLC/mod/language, layer order, override, and `replace_path` semantics. No vanilla/constant fallback. Bind consumed winners by namespace/path, length/hash, and portable identity; redact machine-local paths. Mod/DLC counts are data.
- First use requests content and profile roots, never guesses. Missing, reordered, or changed required content gets a precise refusal; unrelated local content cannot override winners. Replay relocation changes nothing.
- The viewer never accesses live game memory, launches/injects the game, trusts unverified trees, or depends on development tools/corpora at runtime.

## Time and identity

- One canonical tick spans domains, branches, gaps, and seeks. Domain/collection/field identities are append-only, never reused.
- Campaign identity retains and validates raw `game_unique_id` and signed `game_unique_seed`, without fallback. PID plus creation time identifies attachment only. No modset, date, clock, bookmark, roster, or filename alone authorizes joins.
- One logical replay spans campaign restarts; rollback/reload retains branches and playback shows the surviving line. Changed identity starts another campaign; simultaneous games and separate machines record independently. Attachment resources stay isolated; heavy finalization must not starve capture.
- Auto-name files; allow renaming; delete only through explicit user action.

## Performance

- Production hot paths use native DLL/Rust. Production/debug share readers, schemas, fold semantics, and fail-closed outcomes; exclude debug forensics from production files/packages.
- Capture critical path is at most 16 ms/game hour, including p95; do not compensate with more worker threads. Fold inside the recorder at lowest priority, cache repeat folds by content, and make campaigns replayable within 30 s of the last seal.
- Use deltas, intern identities, and bound memory by fold state rather than campaign length. Sealed campaigns must be under 1 GB; 50 MB is the goal. Reduce information overhead before compression without dropping fields, truth states, cadence, branches, or gaps.
- Authenticate and fold the whole certified stream before viewer opening; never mutate the admitted fold. Gap hours are unavailable; restore playback position and provide instant video-like seeking.
- Sustain 60 fps average and p95 frame time at most 33.3 ms at 1920x1080 on i5-8400/Ryzen 5 2600, 16 GiB RAM, GTX 1060 6 GiB/RX 580 8 GiB.

## Release

- Ship one offline, unsigned portable Windows ZIP: desktop recorder, viewer, collector DLL, instructions, checksums. No installer, service, tray-only lifecycle, updater, downloader, administrator requirement, publisher-signature gate, or guessed installation paths. Checksums detect corruption, not publisher authenticity.
- Only an unfiltered central release of both native workspaces is promotable; direct/filtered builds are diagnostic.
- Completion requires current-build, representative vanilla/mod/DLC evidence for automatic app-first/game-first attachment, simultaneous games, campaign/branch integrity, complete scheduled state or exact gaps, content compatibility, and faithful map/UI reconstruction.
- Prove zero-loss passive maximum-speed capture on the support floor (4 CPU cores, 16 GB RAM), including attach, sink, authenticated READY, restored hooks, unloaded collector, and overhead/memory limits. Affinity simulation is only a lower bound; package inspection is not capture proof.
- Bind exact current artifacts to reproducible central-validation, real-capture, container/open/rendering, performance, clean-machine, adversarial-review, and closed-package byte-verification receipts.
- Static engine claims require exact-build controlled-broker Ghidra exports plus dynamic or game-file evidence; indexes/cross-analysis only guide discovery. Evidence procedures live in [agent operations](extractor/native/scripts/AGENT_OPERATIONS.md#engine-evidence).
- Completion also requires size near the measured information floor. “Instant” seeking and “near” the floor remain qualitative requirements without numeric tolerances; do not claim quantitative acceptance or invent thresholds. Name unresolved decisions, live probes, and missing evidence.

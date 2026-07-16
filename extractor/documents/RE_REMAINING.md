# Remaining reverse-engineering (RE) — the consolidated list

**What this is.** The single authoritative list of extraction work that is blocked on
**reverse-engineering the game binary** — i.e. finding an unknown memory root, field offset, or
attribution that does not yet exist anywhere in the repo. Until now this information was scattered
across per-lane packets, dossiers, and the runbook's OWED-PRODUCER register; this file consolidates
it. Owner constraint in force: **RE is out of scope** until the owner lifts it, so every row here is
*parked, not faked* — the affected domain fail-closes (emits a badge / omits the field) rather than
guessing.

**What counts as RE here.** A gap where the extractor cannot emit a datum because the *location or
meaning of the bytes in memory is unknown*. This is distinct from:
- **Owner-run live gates** (the data is emitted; it just needs a live capture to certify) → those
  live in [LIVE_ACCEPTANCE_RUNBOOK.md](LIVE_ACCEPTANCE_RUNBOOK.md).
- **Activation-gated / delicate wiring** (the code exists but is frozen or needs a production call
  site) → listed at the bottom of this file, *not* RE.

Build pin for all offsets: `hoi4.exe` SHA-256 `870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`.

---

## RE-blocked extraction gaps

| ID | Domain | What's missing (plain English) | What specifically must be RE'd | Source of record | Unblocks |
|----|--------|--------------------------------|--------------------------------|------------------|----------|
| **A1** | d62 hub motorization | We know *whether* a country has a saved motorization map for a supply node (`map_size > 0`), but not the **per-country motorization level** inside it. | The layout + attribution of the inner multi-entry `country → level` hashmap (which country entry maps to which level within the node's saved map). | `native/lanes/hub_motorization_live/src/lib.rs:19-21`; `INTEGRATION_PACKET.md` | A1 fixture |
| **A2** | d45 focus/decisions | Whether a national focus is currently **paused** is emitted only as a fail-closed badge, never the real bit. | A production-safe memory **root for the focus-pause flag**. | `native/lanes/focus_research_decisions_depth_live/README.md:27-30` | A2 fixture |
| **A12** | d45 focus/decisions | Decision **targets and cooldowns** are excluded — no numeric target ids, cooldown timers, or day/state body fields. | Roots for the decision **target state/province ids, cooldown/days fields**, and re-enable timers. | `native/lanes/focus_research_decisions_depth_live/SLICE_PACKET.md:13-16` | A12 fixture |
| **A4** | d42 air combat | We emit strength/equipment **deltas** (losses, reinforcements, deletes, mission region), but not the **sortie/combat lifecycle** — no sortie starts/ends, no combat pairings, no reinforcement *reason*. | Roots for sortie/combat **lifecycle events** and combat pairings (`ROOT_DISCOVERY_OPEN`). | `native/lanes/air_combat_losses_live/AIR_RESIDUALS_STATE.md:13-22` | A4 fixture |
| **A21** | d42 air combat | For each downed plane we cannot say **who/what killed it**. | **Kill/cause attribution** root (killer entity + cause), plus convoy/sunk-event variants. (Ace identity/assignment is additionally `FIXTURE_BLOCKED` — needs a live fixture even once the root is found.) | `native/lanes/air_combat_losses_live/AIR_RESIDUALS_STATE.md:13-22` | A21 fixture |
| **A14** | d48 construction | We emit building **health/damage** (`level`, `healthy_level`, `damaged`), but not the **broader repair semantics**. | **Repair-cause semantics** and the repair **target province/location** beyond `target_state_id`. | `native/lanes/construction_buildings_state_damage_live/INTEGRATION_PACKET.md:25,860-866` | A14 fixture |
| **A22** | d61 air-loss channels | Only **6 of 24** air-loss channels are labeled and value-pinned; the other ~18 are an unlabeled multi-writer trace. | Identify + label the remaining **~18 channel indices** (what each unlabeled channel means). | `native/lanes/air_loss_stats_live/src/lib.rs:30-33,66-75` | A22 fixture |
| **A15** | d57 session/MP | Multiplayer `Chat`, `SystemMessage`, `Hotjoin`, and `TagSwitch` event rows are rejected/ignored (only `Pause` with `value_i32` 0/1 is accepted). | Validated **source flags** for those event kinds — a stable pollable **message container + root probe** for the chat/system/hotjoin/tagswitch rows. (The chat-oracle *tooling*, `--expect-text D57_CHAT_PROBE_<ts>`, is buildable offline; the *root probe* is the RE part.) | `native/lanes/session_multiplayer_metadata_live/INTEGRATION_PACKET.md:219-225` | A15 fixture (MP) |

### Borderline — RE **or** a targeted live capture (owner's choice)
| ID | What's missing | Path A (RE) | Path B (live) | Source |
|----|----------------|-------------|---------------|--------|
| **B8 / X6** | The building-token registry join (`id → token` map) used to name building/registry entries. | RE the id→token registry layout. | Or harvest the mapping from a live capture that observes both sides. | session-state memory `residuals-and-no-re-directive`; contract B8/X6 rows |

---

## NOT reverse-engineering — deferred for other reasons (do not conflate with the above)

These are **activation-gated or production-wiring** items. The bytes are understood; the code exists
or is straightforward — they are frozen or need a call site / owner decision, *not* RE.

| Item | Why deferred (not RE) |
|------|-----------------------|
| **CID1 producer barrier** (sequence-0/CFA1) | Collector code path exists but the active path is Legacy; applying it is an **activation flip** (owner-gated). The read-only identity probe is already built + merged. |
| **P4 stitching live-driver** | The stitching engine is merged; it needs a **production call site** in `main.rs` (segmentation / per-segment folds / atomic publish) — offline code work, not RE. |
| **P1 packaging / tray UI** | The auto-attach supervisor exists; missing is **packaging + a graceful-quit tray UI** — product/UI work, not RE. |
| **P2 duplex activation** | Explicit product kill switch `AUTO_ATTACH_DUPLEX_ACTIVATION_ENABLED = false` + the H4CT client (built, held on `port/w1-h4ct`); **owner-gated activation**, not RE. |

---

## Where the raw per-lane detail lives
Each RE row's authoritative detail is in its lane packet under `extractor/native/lanes/<lane>/`
(`AIR_RESIDUALS_STATE.md`, `INTEGRATION_PACKET.md`, `SLICE_PACKET.md`, `README.md`) and the
per-domain rows in [MASTER_EXTRACTION_CONTRACT.md](MASTER_EXTRACTION_CONTRACT.md). Historical RE
dossiers are under `extractor/documents/dossiers/` (`W12C_RE_*`, `W13_RESIDUAL_COMBAT_PROJECT_RE`)
and archived sweeps under `extractor/documents/archive/`.

**When RE is authorized:** each row above is a self-contained target — pick a domain, RE the named
root/offset/attribution against the pinned build, wire the reader in its lane (fail-closed until
confirmed), then the corresponding A-fixture becomes a normal live-capture gate in the runbook.

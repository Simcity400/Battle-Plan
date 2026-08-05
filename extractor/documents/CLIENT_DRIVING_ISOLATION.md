# the game client-driving isolation

Production capture is passive. It may attach to and observe a game process the
user started and controls, but it must not launch or stop the game, synthesize
keyboard or mouse input, change focus or window geometry, write the game clock,
open the console, or rewrite launcher state.

Injected collection is allowed. Injection permits the production collector to
read in-process and to install the minimum evidence-backed observation hooks
needed for values proven unsampleable. It does not permit a semantic engine
state write or any client-driving operation. The product boundary is therefore
**zero semantic game-state mutation and zero client control**, not "no DLL" and
not "no hooks". Hook installation may temporarily instrument the exact pinned
target-code sites; authenticated teardown must prove every site in the
source-derived canonical production hook roster was restored to its baseline
bytes.

The policy above is binding, and `documents/DECISIONS.md` hardens it further:
the shipped recorder never writes game state and specifically never writes the
engine pause byte.

## Production enforcement

The production package enforces this boundary from the shipped PE bytes. The
recorder must retain `BATTLE_PLAN_OBSERVER_ONLY_RECORDER_V1` and
`BATTLE_PLAN_NO_SEMANTIC_ENGINE_STATE_WRITES_V1`; the collector must retain
`BATTLE_PLAN_NO_SEMANTIC_ENGINE_STATE_WRITES_V1`. Packaging rejects the
`BATTLE_PLAN_CLIENT_DRIVING_EVIDENCE_V1` marker and known target-launch, early-render
lifecycle, hour-latch, and pause-write evidence markers. Source-capability
checks separately prove the target-lifecycle actuator module is
evidence-feature-only and production auto-attach owns no target termination or
job assignment. Production is stricter at the shipped-byte boundary: recorder
and collector binaries containing process creation/termination, thread
suspend/resume, job-object ownership, or window/thread-message APIs are
rejected even when an import might otherwise be attributed to a helper process
or the recorder's own UI. The source gate lexically scans the complete
recorder/collector Rust source trees while excluding only the explicitly
feature-gated evidence actuator body. It also structurally parses multiline
Cargo dependency tables and inline tables so fixture-hook features cannot hide
behind formatting. The gate repeats its binary checks against bytes read back
from the completed ZIP and records the result as `interaction_closure` in the
central Release receipt.

`Start-Hoi4ProductionCaptureSafe.ps1` is passive. It accepts an already-running
game, launches only the observer recorder, monitors speed/hour/pause through
query/read access, and enables the read-only scheduler. Its capture plan and
final receipt attest zero client commands, zero semantic engine-state writes,
and no game-process or UI control. The production driver, package gates, and
`Read-Hoi4GameState.ps1` witness are digest-visible production inputs. Promotion
also requires exact `CAPTURE_READY_V2.json` and `CAPTURE_TEARDOWN_V1.json`
receipts. Teardown proves pipe EOF, hook-lease release, collector DLL absence,
target-epoch revalidation, zero semantic game-state mutation, and byte-identical
restoration of every site in the canonical pinned production hook roster.
Central Final/Release runs `Test-PassiveCapturePolicy.ps1`; both that policy
test, the read-only witness, and `PassiveCapture.Lifecycle.ps1` have
independently named SHA-256 fields in the portable-package interaction-closure
receipt.

Speed five is a capture requirement, not recorder authority. The user or an
explicit evidence setup step places the game at speed five before the graded
window. The passive harness then exercises the same injected collector,
authenticated stream, bounded scheduler, sink and teardown boundary used by the
product. This is the accepted real-use approximation for low-level
collector/extractor testing; it does not claim to automate the full game and
recorder UI workflow.

Passing these capability gates is not a production-completeness claim. The
portable product is promoted only after the exact current artifacts complete a
real passive capture at raw speed `4`, unpaused, with zero skipped hours, zero
domain gaps, and one data outcome per frame for every current production
domain. Raw source-unavailable, hard-disabled, partial, bridge, or decode flags
cannot be hidden by an effective-status mask. The central Release therefore
has two phases: candidate artifacts first, then a capture-bound Release that
alone may emit the ZIP and canonical package receipt.

The canonical `collector_record.exe` is observer-only. A client-driving recorder
exists, if needed, only as the separately named
`collector_record_evidence.exe`, requires the explicit
`evidence-client-driving` feature and central `Evidence` tier, carries
`BATTLE_PLAN_CLIENT_DRIVING_EVIDENCE_V1`, and is never accepted at the canonical
package path.

## Evidence-only control

Evidence-only control remains guarded by `Assert-Hoi4ClientDrivingOptIn` in
`portable_replay/native/scripts/Hoi4ClientDriving.OptIn.ps1`, which throws unless its
invoking process sets:

`BATTLE_PLAN_CLIENT_DRIVING_OPT_IN=I_ACKNOWLEDGE_THIS_TOOL_WILL_CONTROL_HOI4`

The evidence harness may use that authority before or after a graded
observation window. During the graded attach-through-teardown window it uses
the digest-visible `Read-Hoi4GameState.ps1` witness, which opens the game with
query/read rights only. It must issue zero client commands in that window.

The historical `Start-Hoi4EarlyLaunchCaptureSafe.ps1` path is a fail-closed
migration tombstone. Recorder-owned process creation, suspended-thread resume,
window ownership and game-clock control are not product capabilities.

`Hoi4ClientDriving.OptIn.ps1` is the canonical production-attestation exclusion
roster. The release source digest still covers runtime, passive observer,
verifier, packaging, toolchain, and product-modset inputs, but excludes the
listed evidence-only drivers. Consequently a production receipt cannot claim a
client-driving workflow as part of shipped authority. The passive production
driver, read-only witness, and package gates must never appear on that exclusion
roster.

Historical H-* cards are optional regression evidence. They do not activate
product code, block packaging, or convert an unobserved state into a value.
Unsupported observations remain feature-local and fail closed. The only genuine
external production dependency is CID1 M3 on a second physical machine.

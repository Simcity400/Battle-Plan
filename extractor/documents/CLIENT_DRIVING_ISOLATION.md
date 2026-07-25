# the game client-driving isolation

Production capture is passive. It may attach to and observe a game process the
user started and controls, but it must not launch or stop the game, synthesize
keyboard or mouse input, change focus or window geometry, write the game clock,
open the console, or rewrite launcher state.

The policy above is binding, and `documents/DECISIONS.md` hardens it further: the
shipped recorder never writes game state and specifically never writes the engine
pause byte.

**The enforcement described here does not currently exist.** The gate is
`Assert-Hoi4ClientDrivingOptIn` in
`extractor/native/scripts/Hoi4ClientDriving.OptIn.ps1`, which throws unless the
invoking process sets

`BATTLE_PLAN_CLIENT_DRIVING_OPT_IN=I_ACKNOWLEDGE_THIS_TOOL_WILL_CONTROL_HOI4`

— and a repository-wide search finds no caller. Nothing invokes it.

Meanwhile `extractor/native/scripts/Control-Hoi4Game.ps1` writes game speed and
the engine pause byte through `WriteProcessMemory`, and
`Invoke-HeadlessRawCapture.ps1` calls it without any acknowledgement. The only
isolation these helpers actually have is `Test-Hoi4ClientDrivingInputPath` in the
same file, which merely excludes them from the production attestation digest.

Treat this file as the intended contract and an open defect, not a description of
current behaviour. Either wire the assertion into every game-controlling helper,
or delete the helpers with the pause barrier when it goes.

`Hoi4ClientDriving.OptIn.ps1` is the canonical production-attestation exclusion
roster. The release source digest still covers runtime, passive-observer,
verifier, packaging, toolchain, and product-modset inputs, but excludes the
listed evidence-only drivers. Consequently a production receipt cannot claim a
client-driving workflow as part of the shipped authority, while changes to
runtime source remain digest-visible.

Historical H-* cards are optional regression evidence. They do not activate
product code, block packaging, or convert an unobserved state into a value.
Unsupported observations remain feature-local and fail closed. The only genuine
external production dependency is CID1 M3 on a second physical machine.

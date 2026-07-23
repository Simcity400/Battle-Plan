# the game client-driving isolation

Production capture is passive. It may attach to and observe a game process the
user started and controls, but it must not launch or stop the game, synthesize
keyboard or mouse input, change focus or window geometry, write the game clock,
open the console, or rewrite launcher state.

Historical evidence helpers that can perform any of those actions are legacy,
nonproduction tools. They are inert by default. PowerShell helpers either use a
dedicated positive switch or call `Assert-Hoi4ClientDrivingOptIn`; direct Python
helpers require the same process-local environment acknowledgement:

`BATTLE_PLAN_CLIENT_DRIVING_OPT_IN=I_ACKNOWLEDGE_THIS_TOOL_WILL_CONTROL_HOI4`

The acknowledgement is never set by production capture, centralized
validation, release packaging, or passive observers. A user-authorized legacy
experiment must set it only in that isolated invoking process and must also pass
any helper-specific switch. Child processes inherit the acknowledgement only
from that deliberately scoped parent.

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

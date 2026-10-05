# Raid log

Every country's observed raids, newest first, using the shared `raids` product and
`charts/log/LogList` (style-5). Live follows playback; off shows the whole campaign.
Rows show the read date, native target-specific artwork, raider and target controller
with flags, location, localized phase and any completed outcome, a content raid-type
tag, and labelled integer hours prepared inside the sentence while preparing.
Right-click jumps to the row's last phase, outcome, target or type change.

Type chips include only types observed in this branch. Equally localised names
include their content keys so the chips remain distinguishable. Settings retain
every content type; **All types** and none selected include every type. Country
selection defaults to every country, with chosen countries and smart
groups filtering by the raider at the observation hour. Only visible rows request
country names and flags, at their own dated hours, through `logCountryRequests`.
No human, AI or player status gates acquisition or inclusion.

Start and change dates say **by** the read. Nothing recorded supplies an exact
start date. A gap leaves current phase and preparation unknown; the historical
last observation remains explicitly labeled. An episode's observed result remains
in the campaign log even after the native raid disappears. Raids wholly between
reads may be absent. Missing text, target identities and artwork are said unknown
or use the recorded token/ID, with named, counted product diagnostics.

The picker uses the shared product-driven availability mechanism. Known content
raid definitions or recorded eligible types/actual raids establish presence;
unavailable inputs do not establish absence. An absent feature says **Not in this
recording** and disables the picker entry. Country roster existence alone is not
evidence for raid availability.

Native meaning lives beside `crates/dashboard/src/products/raids.rs`: phase and
outcome keys are executable literals resolved through stack localization; target
controllers follow the game's building/province/state/captor precedence until
the first ended read freezes the target country. Integer hours show the stored
preparation amount during the preparation phase (`GetPhaseElapsedHours@140FD53C0`,
tooltip `141612820` → `140FD5730` and instance view `141B1A610`); no percentage
denominator is guessed. The instance view selects outcome text only after the
raid ends, so an absent outcome never prints **None**. Compact per-episode
preparation reads update the amount and its read date without re-entering or
reordering the row. The tooltip names its exact preparation read and the raid's
last observation, independently of other countries' missing reads.
Per-instance building-suffixed art is resolved once per sprite in the reply.

Choices where the spec is silent: one row per episode, re-entering at its latest
phase, outcome, target or type change; category chips by observed raid type; country
filtering by raider; labelled hours inside the sentence rather than a percentage
or an unlabelled amount column; shared comfortable density/month headers/motion
and filter-chip settings. Browser fidelity and real recording checks are unverified.

# H-A37 panel-open and controller-visibility closeout

The a729 lifecycle producer is statically closed. The country-politics launcher
callback at `0x141570450` is the exact toggle owner. It reads the
`CPowerBalanceView+0x30` open byte through `0x1414C29C0`; an open panel routes
to close root `0x141C49570`, while a closed panel closes the competing politics
surfaces and routes to show root `0x141C4A6B0`.

The state is not inferred from authored positions. Close calls the GUI hide
method and clears `controller+0x30`. Show calls the GUI show method, sets that
byte, runs the first-match balance selector, and clears the byte again when no
match exists. The controller GUI root is `controller+0x48`; its matching
logical-visible state is bit `0x08` at `root+0xA5`.

Launcher visibility is independently produced by the country-politics refresh
at `0x141570510`. While that surface is active, it hides the container at
`country_politics_view+0x7BA0` when the first-match selector returns the null
object and shows it otherwise. If the panel is open, the same refresh calls
`0x141C4A860`; that path closes the panel on no match or joins
`controller+0x598` to the same selected record before the terminal writer.

The consolidated receipt is job
`20260720T044622Z-w4-a37-panel-open-visibility-authority-ca46b41d`, shard
`shard-01`, manifest
`2c921216fb9b825716685852c026aba92b7da88a5c88fdb625a569e49afedd9c`,
with all six requested roots resolved on the exact a729 binary. Ghidra status
was ready with all 20 configured shards, read-only/no-analysis mode, image base
`0x140000000`, and executable SHA-256
`870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`.

`src/a729_hook.rs` now has pointer-discarding guards for the country-politics
refresh and launcher toggle. The terminal post-refresh guard also requires the
separate lifecycle receipt, the open byte, and the controller GUI-visible bit.
Executable, manifest, target, entry sentinel, owner vtables, thread, Boolean,
selected-record, or GUI-state disagreement fails closed.

The installed `powerbalanceview.gui` and `countrypoliticsview.gui` files confirm
the authored hierarchy and animation properties. The retained corpus originally
had no open-panel live A37 sample. The reversible observation described below
now supplies exact live lifecycle corroboration, while remaining deliberately
separate from the authenticated selected-Present collector gate. Therefore
`A729_PANEL_VISIBILITY_OBSERVER_ACTIVATED` remains false, the existing
`A729_CONTROLLER_OBSERVER_ACTIVATED` remains false, and d74 remains reserved.
No human semantic judgment is required. The remaining collector promotion work
is the already-required same-transition fold and replay parity proof.

## Current-country owner closure (2026-07-21)

Fresh exact-a729 depth-one export
`20260721T120232Z-a37-current-country-owner-lifecycle-34341641` (shard 16,
8/8 seeds, manifest
`2c298279fa8e4c4c6c9794713d01987fd96f7b58f8bc936033523e38229b34e7`)
closes the previously implicit owner and country-switch route.
`CCountryPoliticsView` constructor `0x14156A2F0` allocates the
`CPowerBalanceView` and retains it at `+0x7A50`; the registered launcher
callback `0x141570450` always resolves that exact owner slot before opening or
closing the panel.

Launcher refresh, panel show, and open-panel refresh all select the same
current-country handle from game state: `+0x520` when its signed value is
positive, otherwise `+0x524`. They pass that handle and the balance registry at
game-state `+0x450` to first-match selector `0x140E424F0`. Therefore a player
country change is itself the lifecycle discriminator: switching to a country
with a matching row replaces controller `+0x598/+0x5A0`; switching to one with
no row hides the controller GUI and clears its open byte.

The loaded paused D04 save already supplies both halves without a new save:
ITA owns `ITA_power_balance`, while D04 owns no balance. The exact positive and
no-match-close route is consequently automatable as a reversible paused
`D04 -> ITA -> D04` harness sequence. It does not require human interpretation,
unpausing, or saving. The machine-readable authority and safety contract is
`H_A37_CURRENT_COUNTRY_LIFECYCLE_RE_20260721.json`.

## Live lifecycle observation (2026-07-21)

The bounded reversible route completed without unpausing or saving. Registered
read-only memory observations recorded D04 handle `368`, ITA handle `16`, and
restored D04 handle `368`; every phase retained game hour `60830441`. Physical
input opened Italy's real Politics surface and National Balance of Power panel.
Switching back to D04 while that panel was open closed both surfaces.

Bounded trace `20260721T121839Z-trace-calls-5cafaa7e` retained one execution of
the exact close root `0x141C49570`. At close entry the controller open byte was
`1`, selected record was `0x189278A1CF0`, and selected range was
`0x188B3476C70`. The 15-second trace completed with 515 messages, no errors or
cleanup errors, and result SHA-256
`edf0e832d3ce0cd34f1bf3c8dda63b80800bb998cc7d79273b3b37aa02826678`.

`H_A37_LIVE_CURRENT_COUNTRY_LIFECYCLE_20260721.json` binds all three
read-only observations and content-addressed visual receipts. This closes the
live lifecycle RE question without human input. As required by the facade
contract, the trace remains discovery corroboration and cannot itself promote
the selected-Present collector or schedule d74.

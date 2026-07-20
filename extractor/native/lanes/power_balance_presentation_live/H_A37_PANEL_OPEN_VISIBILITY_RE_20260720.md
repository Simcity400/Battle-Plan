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
the authored hierarchy and animation properties, but the retained capture
corpus still has no authenticated open-panel live A37 sample. Therefore
`A729_PANEL_VISIBILITY_OBSERVER_ACTIVATED` remains false, the existing
`A729_CONTROLLER_OBSERVER_ACTIVATED` remains false, and d74 remains reserved.
No human semantic judgment is required: the remaining work is an automatable
live wrapper/fixture run followed by the already-required same-transition fold
and replay parity proof.

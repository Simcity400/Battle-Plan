# H-A37 power-balance controller closeout

Exact a729 static RE now closes the live controller and transition route. The
`CPowerBalanceView` constructor at `0x141C47E90` installs primary vtable
`0x142A421C0`; the live view holds the selected balance record at `+0x598` and
the engine-selected active-range definition at `+0x5A0`.

The selector `0x140E424F0` scans `0x190`-byte records in stored order and
returns the first country-predicate match. Consequently, the base-game panel
has zero or one selected balance for a country, not a presentation list of all
applicable balances. `0x141C4A6B0` owns open/show refresh and `0x141C4A860`
owns state-change refresh; both end at `0x141C4BAA0`, the terminal writer for
the percentage, marker, sprite, icons, names, ranges and decisions.

The exact side laws are intentionally independent:

- Current value `< 0` selects the left active side; value `>= 0` selects right.
- Negative change selects `GFX_power_balance_left_moving`; positive change
  selects `GFX_power_balance_right_moving`.
- Only zero change selects a stable sprite, using current value sign.
- The engine trending-side definition is independently stored at record
  `+0x28`.

This proves two current replay checks are invalid: a moving sprite cannot be
used to derive active side, and trending side need not equal active side. The
right-active/trending-left and left-active/trending-right transitions are the
necessary discriminator cases.

The selected active range is already present at record `+0xB0`; capture that
definition identity rather than re-evaluating shared thresholds. `0x141C498D0`
uses stored range order and live GUI geometry to place indicators, then writes
engine frame `2` for the selected range and frame `1` for other ranges. The
current zero-based carrier field therefore needs an explicit normalization or
schema change.

The modifier surface also differs from the reserved schema. The
`GetPowerBalanceModDesc` dispatcher reaches `0x140E422E0`, which formats the
inline root at record `+0xC8` for modifier type `0x98`, appends one newline
when required, then formats type `0x99`. The UI receives one resolved aggregate
string. It does not expose a complete ordered list of modifier ids and source
records. D74 must carry an A25-resolved aggregate text reference instead of
inventing rows from D67 or parsing localized text.

`0x141C4AA30` closes the decision binding with two distinct publication
routes. It walks the `0x130`-byte on-map-locator source vector and inserts each
accepted presenter immediately through `0x141C48550`. It separately gathers
conventional decision, targeted-decision and timed-decision presenters after
the exact category, runtime and visibility gates, sorts only that vector
through `0x141C474B0`/`0x141C475A0` and comparator `0x141C48340`, then appends
the sorted suffix through `0x141C48550`. The finalized grid order must be
captured after both routes; combining and globally re-sorting them is false.

The root-owned exact20 receipt is now bound to the static observer ABI as job
`20260720T020812Z-w3-a37-power-balance-5c5d63f5`, shard `shard-11`, manifest
`a540af1c030d9ef2fbb1cd290627087627771a196e7a60e4241cb5287b66e4e2`, with
10/10 exact roots and 20/20 pool preflight. PBP2, its d74 fold, and XUI18
admission now carry the corrected first-match, three-independent-side-state,
frame-normalization and aggregate-text semantics. `replay_core` now admits the
folded `_r2_power_balance_presentation_live` projection as a strict typed view,
binding its replay hour and exact20 manifest while preserving completeness and
source-record identities. `replay_app::power_balance_runtime` resolves each
referenced A25 context only at the same hour and country-or-global POV before
constructing `PowerBalanceRuntimeSupplement` and entering the existing XUI18
`game_ui` admission gate. The supplement and admitted presentation retain the
d74 manifest, completeness identity and frame hour together with every A25
context/view/element and exact producer identity. The production selected-POV
UI runtime invokes this seam directly; an unavailable A37 feature remains
feature-local and cannot suppress the independently authenticated selected-
Present pass.

The same receipt also provides an unambiguous inert hook ABI for terminal
post-refresh `0x141C4BAA0`: Windows x64 `RCX=this`, `DL=full_refresh`,
`R8B=range_refresh`, RVA `0x01C4BAA0`, owner-vtable RVA `0x02A421C0`, and entry
sentinel `488bc44889581888501055565741544155`. `src/a729_hook.rs` rejects executable,
manifest, target, sentinel, thread, owner, flag, or normalized-state mismatch;
it cannot admit a live callback while activation is false and serializes no
process pointer.

Production collection and scheduling remain disabled. The real hook owner now
emits only a selected-Present-joined raw controller receipt; it cannot accept a
caller-authored PBP2 or invoke the production seal while the activation latches
are false. PBP2 v3 transport, fold, replay join and typed companion remain
dormant consumers, not proof that their semantic producers exist. Stable
balance/country/side/range identities, geometry, icons, text and decisions need
exact runtime producers plus a complete live PBP2 fixture before activation.
The three earlier discovery manifests remain
discovery provenance rather than the bound consolidated receipt. Their hashes
are `da22c2cf2e8be089907e025d6720485fa96b2e1f50c9485113c0c2f6d8b06906`,
`2992dda21dbf78d76288a255cfbcd066ea226bd741192fb8978f1771d6fd3c25`,
and `253cd925a507d18ee85d94826ca3540d1c44f51d484c1b1454e2e374a685e4c5`.
The two-case raw fixture closes only current/change/open/QPC behavior. Further
machine RE and integration, then root-owned central validation, are required
before d74 can be scheduled.

# dashboard-naval-regions

Prepared mine amounts and country-viewpoint naval supremacy above dashboard-base.
No viewer/GPU dependency, live capture, faction totals or per-side mode.

## Public API

`products::minefields::{Minefields, MinefieldsObserver, FAMILY, query}` serves
`minefields`, store `minefields`. `products::naval_supremacy::{NavalSupremacy,
NavalSupremacyObserver, FAMILY, query}` serves `naval-supremacy`, store
`naval-supremacy`. Each Product exports all nested wire types through `types()`
and retains the default prepared-only loader. The dashboard owns registration,
query caching, gate coverage/pending and contract export.

Both families use dashboard-keyed's block pre-pass, numeric pages, independent
known/supplying-read runs and persisted continuation. Numerical histories never
seek recording frames. Report detail alone uses the shared reached-hour frame
cache. Queries check cancellation and the shared complete-reply budget; oversized
replies refuse rather than dropping reports, regions or countries.

## Minefields

History parameters: `{branchId, tags?, firstHour?, lastHour?, view?:"history"}`.
Reply: `{window, regions:[{id,name}], scale:100000, warnings}`. Country parts are
`TAG@slot`; an explicit such selection also disambiguates reused tags.

Names `OWNER|extent` carry each owner's witnessed vector extent; `OWNER|REGION`
carry only nonzero signed fixed5 amounts. An absent amount below that hour's
extent proves zero. Beyond the extent or without it, the amount is unknown.
Owners without witnesses, malformed witnesses, canonical collisions and values
outside the vector refuse that owner, not independent valid owners. A refused
owner keeps dated evidence, `OWNER|refused` = 1, and `?|refused` counts owners
the hour holds vectors for but cannot name, so an incomplete inventory is never
read as an empty one. A country selection keeps `?|refused`: an unnamed owner
may be any selected country. No dense
owner-by-region zero expansion or chosen country/region cap.

Reports parameters: `{branchId, view:"reports", region, hour}`. Reply:
`{hour,readHour:null|number,region,reports:[{kind,ordinal,victim,mineOwner,ship,
equipment,damage,sunk,firstSeen}],warnings}`. Only kind 1 standing reports are
returned, in recorded owner/occurrence order; repeated reports remain repeated.
Countries are nullable `TAG@slot`. Damage is raw signed fixed5 decimal text,
not a hull percentage. Equipment resolves the full retained variant through
recorded tokens and the content catalogue. Ship names are available only when
an independently recorded cached name binds that full handle; other names are
null with counted refusals. No recycled partial-handle lookup.

`firstSeen` is always null: prepared mine pages do not certify report presence,
and ordinals do not establish persistent report identity. Expiry never becomes
a strike date. There are no event dates or cumulative mines-laid/strike totals.

## Naval supremacy

Parameters: `{branchId,tag,firstHour?,lastHour?,daily:true}` (`daily` defaults to
true; false refuses). Reply: `{window,dayHours:24,regions:[{id,name}],scale:100000,
warnings}`. Output names `REGION|control`, `|friendly`, `|enemy`, `|share`,
`|shareNum`, `|shareDen`, `|own` have one measure per series. `share` is the
displayed explanatory share, computed from `shareNum` and its enemy part by the
shared native quotient; the parts stay separate explanatory operands. Control codes are friendly 1,
enemy -1, neither 0; absence is a refusal, never numerical zero.

Raw per-country currents and reduced diplomacy operands are stored, not derived
viewer-by-region pairs. This keeps preparation and storage proportional to live
rows for 1,500 countries rather than a viewer cube (tested at 5,000). Distinct raw
aliases remain distinct current rows. Country names in series keys use TAG@slot;
raw references stay separate numeric identifiers. Reserved-country identity and
family/war branches use their independently recorded naming and hourly packets.
Internal `input-current` operands retain unresolved raw identities without
inventing tags. Internal
`input|table|row|length/fields/mask/word` series encode only the rows/fields used
by the native rules, using the existing lossless row delta codec. Eight-byte
numeric words keep nested reserved-country packets in prepared pages rather
than growing historical frames or packet intern pools in catalogue state.

Queries read those pages at each calendar day's last reached recorded hour,
including endpoints whose inputs are unavailable. They project the endpoint
across the clipped day, exactly as weather's daily sampling does. A preceding
known hour never fills an unknown endpoint. Accepted prefix, request end and
recording gaps determine endpoints from recorded metadata; no frame walk or
ETA-based reconstruction. `dayHours` describes the calendar span, not another
sampling cadence. Supply dates remain the original read hours. Equal fresh
reads are carried by the keyed READ channel. A sample must belong to the same
midnight epoch as its endpoint; no prior-day carry, including across gaps.

The shared replay-derive native rule keeps family-before-directed-war control,
wrapping sums, guarded fixed5 quotients, content minimum and threshold, and
friendly tie resolution. A frame-local control context reuses family and
directed-owner admissions across regions. Explanatory membership is separate:
unique friendly membership, ordered enemy aliases/occurrences, blockade additions
and its own minimum floor. Its parts are `shareNum` and wrapped `shareDen`.
A displayed share uses the shared native guarded fixed5 quotient, except a zero
denominator yields zero; plain floating division is not that rule. The parts must never
be combined with control's friendly/enemy sums. A bare tag reused across different
slots in the sampled window refuses derived series; select TAG@slot to
disambiguate. No 40/60 band or invasion/convoy permission inference. Missing diplomacy refuses only the affected projection;
missing terrain/defines never hides own current or known control sums.

Region identities, sea membership, names, terrain minima and defines resolve
through recorded content layers, with cached successes and failures per content
and language binding. The native terrain fallback resolves the content's ocean
category; there are no numerical vanilla defaults. Naming failures retain exact
numbers and are counted. A failed sea-membership read does not invent extra seas.

## Checks

Run only the lane-authorized `cargo test -p dashboard-naval-regions`,
`cargo test -p replay-derive`, and `cargo test -p viewer-foundation` with
`CARGO_TARGET_DIR=lane-airnavy`. Installation metadata tests read
`BATTLE_PLAN_GAME_DIR`; all added recording/content fixtures are synthetic.

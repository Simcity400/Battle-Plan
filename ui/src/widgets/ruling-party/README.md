# Ruling party

## What it shows

Party-popularity history with the current leader/party header and optional government, election and leader-change markers.

## Products it reads

Prepared [`party-history`](../../../../crates/dashboard-party-history/src/products/party_history.rs) supplies viewport popularity summaries, reached leader/name/share values and paged change events. Narrow windows draw through the legacy stack path; dense windows share the chart summary sampler. Prepared [`politics-ledger`](../../../../crates/dashboard-politics-ledger/src/products/politics_ledger.rs) (`governments`) supplies native cause clusters, supplemental open-day/late causes, reached details and event pages. Government causes read the prepared ledger; recording scans are test-only parity oracles. Shared dated country facts supply labels/flags.

Popularity draws before preparation reaches the viewport: broad tails use
keyframe estimates, and zooms covering at most thirteen recording blocks use exact
native reads. The hook publishes those replies immediately and replaces them on
accepted coverage changes, retaining approximate spans for shared drawing. Marker
pages show accepted events, refresh with accepted coverage and use the shared
prepared-range note.

## Settings

Country, campaign/follow/pin/range, playback style, Show after the playhead, government/election markers and leader markers.

## Notable behaviour

- Ideology groups/names/colours come from content. Each band keeps its content colour's hue at the categorical palette's lightness and (at most) chroma for the theme, so bands match the app's other categories; a grey stays grey. The legend uses ideology names or Ideology name unavailable; the header uses the game’s party-name rules. Unknown leader, absent leader and unknown name stay distinct.
- Party names use a stored key, else the party's own country tag, its positive original-tag reference (own tag otherwise), then ideology name. Missing operands refuse only reached fallbacks. Names are plain text; scripted brackets remain glyphs.
- Only proven recorded portraits draw; missing generated-portrait inputs use a helmet artwork fallback.
- Popularity summaries carry approximate spans beyond prepared coverage; both dense and exact stacks use the shared clipping and faded dashed styling, leaving exact regions unchanged.
- Native government causes draw at their execution hour once their supplying read is visible. Exact viewports run the before/after pair/read join to replace a matching census marker; election dates and unrelated actions do not prove a cause. Dense views use width-bounded native and census clusters and can retain an unproven census replacement. Reached clusters expose every cause through exact pages.
- Dense marks combine prepared party census clusters, closed-day native government clusters and supplemental tail clusters. Each opens its own tooltip and event page; party events and government changes can remain open together. Keyboard navigation chooses the nearest party event or native government change in the requested direction, with one source-owned keyboard position.
- `partyMarkSummaries(summary, censusBadges, clusters)` includes already-returned census clusters when governments are dense, including viewports party history considers exact; other callers keep the default `false`.
- Browser parity registers both "Party history values" and "Government execution causes" on separate elements.
- Missing cause endpoints/coverage remain explicit. A cause request failure leaves popularity usable with its own retry; older recordings retain unattributed census changes.
- Event pages refuse a continuation cursor that does not advance; an empty membership-filtered page may still advance its raw cursor. Pending reads keep paced status polling while preparation works through a block; an unchanged status after an instant wait ends with a retryable error.

## Files

| File | Responsibility |
|---|---|
| [RulingPartyWidget.tsx](RulingPartyWidget.tsx) | Leader header, bands and markers. |
| [partyData.ts](partyData.ts), [governmentData.ts](governmentData.ts) | Validation, local lookups and ledger/census matching; adjacent tests. |
| [preparedGovernments.ts](preparedGovernments.ts) | Native mark summaries, reached days, event pages and keyboard navigation. |
| [preparedParty.ts](preparedParty.ts), [PartyEventList.tsx](PartyEventList.tsx) | Viewport/reached queries, source ownership and complete event paging. |
| [partyParity.ts](partyParity.ts) | "Party history values" and "Stability party markers" browser models. |
| [definition.ts](definition.ts), [index.ts](index.ts), [rulingParty.css](rulingParty.css) | Options, registration and layout. |

`country-people` adds reached election interval/next date and governments in exile, including host, legitimacy, manpower pool/cap and leader resources. Exile cards retain their supplying dates and share the widget country and hour.

# Ruling party

## What it shows

Party-popularity history with the current leader/party header and optional government, election and leader-change markers.

## Products it reads

[`party-history`](../../../../crates/dashboard/src/products/party_history.rs) supplies popularity, leaders, names and elections. [`politics-ledger`](../../../../crates/dashboard/src/products/politics_ledger.rs) (`governments`) supplies captured causes; shared dated country facts supply labels/flags.

## Settings

Country, campaign/follow/pin/range, playback style, Show after the playhead, government/election markers and leader markers.

## Notable behaviour

- Ideology groups/names/colours come from content. The legend uses ideology names; the header uses the game’s party-name rules. Unknown leader, absent leader and unknown name stay distinct.
- Party names use a stored key, else the party's own country tag, its positive original-tag reference (own tag otherwise), then ideology name. Missing operands refuse only reached fallbacks. Names are plain text; scripted brackets remain glyphs.
- Only proven recorded portraits draw; missing generated-portrait inputs keep the silhouette.
- Native government causes appear at their supplying read. Only an exact matching before/after pair/read replaces a census marker; election dates and unrelated actions do not prove a cause.
- Missing cause endpoints/coverage remain explicit. A cause request failure leaves popularity usable with its own retry; older recordings retain unattributed census changes.

## Files

| File | Responsibility |
|---|---|
| [RulingPartyWidget.tsx](RulingPartyWidget.tsx) | Leader header, bands and markers. |
| [partyData.ts](partyData.ts), [governmentData.ts](governmentData.ts) | Validation, local lookups and ledger/census matching; adjacent tests. |
| [definition.ts](definition.ts), [index.ts](index.ts), [rulingParty.css](rulingParty.css) | Options, registration and layout. |

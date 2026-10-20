# Decisions

## What it shows

Every decision and mission the chosen countries took, as Construction log's rows: newest first by the date selected, each with its art (behind **Game artwork**), the in-game name over how it ran or ended ("70-day timer · 12 days left", "Selected 3 Mar 1939 → completed 12 May 1939", "Cancelled after 14 days", "Removed by the game"), the country unless exactly one is shown, a status pill and its cost. The header reads "N active" once every page is known.

## Products it reads

[`decisions`](../../../../crates/dashboard/src/products/event_feed/prepared/script.rs) supplies backward pages of occurrences from the event feed's prepared facts: a selection (or an activation with no open selection) with its activation, duration writes, effect executions and end. Paging and pending states are the event feed's ([useScriptOccurrences.ts](../event-feed/useScriptOccurrences.ts)).

## Settings

The dashboard's country by default, or any number of countries/groups; campaign/follow/pin/range; the **Active**, **Completed**, **Cancelled**, **Timed out** and **Removed** toggles, also the filter chips; artwork and shared log options.

## Notable behaviour

- A row appears at the read that shows its selection and keeps that place; its status changes in place as later facts are read. Each selection is its own row.
- Status follows recorded facts only: an end (completed, cancelled, timed out, removed) once read; Active while a timer, activation or duration change is recorded; otherwise **Taken**, listed with Completed. A decision still running when a break in the recording begins reads "Not recorded" with "no end recorded by <date>", listed with Active.
- Remaining days count down to the playback date from the latest recorded duration, only when one is recorded.
- A cost is shown only where the backend proves the payment, written as Political power spending writes amounts ("−50 PP", "−10 CP"); otherwise an em dash reads "Not recorded". Authored prices are never shown as charges.
- Effects that ran are listed in the tooltip, not as statuses. Missions and timed decisions are not told apart, since the recording does not say which a decision is; there is no category picker yet.
- A name the content does not resolve reads "Decision name unavailable", named and counted in the diagnostics.

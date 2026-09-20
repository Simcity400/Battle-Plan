# desktop-protocol

The shared contract between the Tauri desktop host and native runtime. It carries control messages, recording revisions and compact projected analytics results. It never transports map images or raw replay tables.

## What crosses the boundary

The host sends geometry, visibility, open-recording, seek and dashboard-view commands over a bounded line-oriented pipe. Dashboard geometry contains full body rectangles and separate visible clips. Map acknowledgements identify the actual displayed source, branch, hour and request sequence, separately from the requested target. Replaceable updates may be coalesced; preserve ordering where it changes behavior.

The revision marker uses file length and modification time. It detects the recording revision expected by a query result; it is not a cryptographic content identity. An open dashboard source retains an immutable snapshot at that revision. A standalone map seek that reopens the file must refuse an outdated analytics revision after a file change.

`dashboard_data.rs` defines source leases, independently cancellable query requests and numbered UTF-8 response fragments. Fragment and aggregate response limits are separate. Reassembly rejects changed lengths, missing/reordered fragments and premature completion. Endpoints must also bound aggregate buffers across requests, not just individual frames.

The native process performs asset-independent analytics and shares recording sources with map preparation. Only the resulting chart/value DTOs cross this boundary. This avoids duplicating recording buffers in the Tauri host; it does not eliminate the cost of encoding and delivering projected results.

## Changing the contract

`MapAction` routes the shell's existing map controls, folder configuration and export request to the native viewer. Actions have a bounded queue and run while the map is hidden by a pop-down. `MapDetails` returns an on-demand, byte-budgeted diagnostic report with its recording and displayed date; raw tables and map pixels remain native. Opening a save dialog chooses the PNG path before the host requests the viewer's existing capture path.

Land-owner discovery requests carry a source lease and revision; replies echo that identity with a compact country catalogue and coverage warnings. This background request uses the same cancellation and projected-response budgets as other dashboard data. It does not change the recording format or transport raw ownership history.

Start in [src](src/), then update [desktop](../desktop/README.md) and [viewer](../viewer/README.md) together. Keep the types small, validate malformed input through existing tests, and follow the [shared checks](../../AGENTS.md). Window ownership, DPI, shutdown and child-process cleanup remain endpoint responsibilities.

Country-flag requests identify a source lease/revision and an exact branch/hour, plus a bounded batch of recorded tags and a full/small asset class. Replies echo the class and carry bounded RGBA images, resolved asset names and provenance/unavailability warnings through the existing cancellable, byte-budgeted response channel. Consumers reject a mismatching asset class. Map pixels still stay native.

Map preferences use the cancellable data service with an explicit global, campaign or widget target. Campaign and widget requests validate the source lease and revision; global requests require empty source identity and no recording. Missing targets retain legacy campaign scope. Reads return the installed-mod catalogue, saved inheritance markers and resolved parent settings; writes validate the target before saving. Preferences are separate from recording identity and dashboard layout storage. Graphics controls gate existing native passes; selected cosmetic layers are applied only after original content verification. The removed owner/controller option and manual Fit action are no longer accepted by the protocol.

Map view targets carry optional viewing-country/fog options, defaulting to the recorded country with fog enabled for older callers. Export actions may include a validated viewId to address a dashboard map; omission retains standalone export. Host and viewer must be rebuilt together for these additions.

Unloading a campaign sends its canonical path to retire any matching standalone map, including a hidden cached map. It cancels earlier queued opens/seeks for that path. A later open is still allowed; other recordings and dashboard views retain their own ownership.

National-focus history requests carry a source/revision, branch and country tag. Replies echo that identity and carry compact disjoint history segments and a bounded localised artwork catalogue through the same cancellable response channel. Focus icons use PNG data URLs; country flags retain their separate pixel contract. Playback selects an exact covered hour locally and never carries a segment over a gap. The generated analytics contract owns the completion data types.

Focus entries carry nullable first-observed completion bounds; initial completions have no invented date. Artwork entries include the game-localisation description and nullable base duration in days. Consumers display date-only bounds and expandable inline details, distinguishing base duration from elapsed campaign time.

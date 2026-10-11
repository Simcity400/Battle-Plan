# National focus

## What it shows

Ordered focus completions plus the current/unfinished focus, with recorded progress and conditional game artwork.

## Products it reads

[`national-focus`](../../../../crates/dashboard/src/products/national_focus.rs) supplies order/progress and country-independent localized text; [`focus-artwork`](../../../../crates/dashboard/src/products/focus_artwork.rs) supplies dated names, descriptions and conditional icons. Known static titles, including empty titles and text equal to its key, need no dated title request. Absent or null static names still use dated localisation. Titles, images and full descriptions use independent request queues and backend locks. Title requests exclude descriptions. Image requests use `iconsOnly` and exclude both titles and descriptions, so only authored icon conditions require scoped image reads. Full descriptions load only for opened completion rows; known static descriptions remain available immediately. Title and image queues prioritise the current card in Follow and Pin. Follow then requests completions newest first, matching its automatic scroll to the latest completion; Pin and Whole campaign request completion rows in their displayed ordinal order. Scheduling preserves every row and its recorded dates.

The queues share recent decoded branch/hour frames for the source revision. Frame reads validate the containing block's checksum, then stream only through the exact requested hour. A least-recently-used cache keeps up to 64 MiB of row allocations, conservatively counting shared allocations for each cached entry plus frame/table ownership and unavailable reasons. Nested strings and lists owned by rows are outside this memory measure. Eviction limits reuse rather than recordings, countries or replies; oversized frames return complete without retention. Scoped reads upgrade the selected tables when needed and satisfy later plain-text reads. Exact recorded hours, gaps and cancellation remain distinct; text evaluation and image rendering run after releasing the shared frame lock.

The selected-country history scan seeds that same cache at each new order segment's first hour, after the complete recording block and campaign have passed validation. Only Campaign, Countries and CountryLinks views are retained, including their supplying-read provenance. Static completion images can reuse their confirming or undated segment hour immediately; scoped titles and icon conditions still load the required additional tables. Existing scoped entries stay scoped, and there is no second snapshot cache. Cancelled history requests expose no partial reply; independently verified frames already retained remain internal reusable inputs.

Cache hits and verified history seeding use short locks and proceed while another request reconstructs a missing hour. Reconstruction misses remain serialized and recheck the cache after admission, so identical reads share verified frames. A cancelled waiting request never cancels another request's reconstruction.

Order history borrows the selected raw focus owner's contiguous completion rows from the complete decoded table's validated country/ordinal key order. Country links still determine canonical identity; the recorded completion count and every ordinal must match before the history accepts that hour. Consecutive hours with the same focus, completion and country-link allocations and unchanged country identity keys extend the accepted runs without rebuilding their lookup or comparing tokens. Changed inputs and gaps take the complete validation path.

First-use history loading prepares the shared focus content while scanning the recording. Both tasks finish before a reply or error leaves the product; scan refusals keep their own reason. Pre-cancelled requests start neither task. Later requests reuse the focus context and check it against the current shared catalogue.

## Settings

Country and campaign/follow/pin; Whole campaign is the default. There are no widget-specific options.

## Notable behaviour

- Newly observed completions say By their confirming read; entries already in the first readable snapshot have no known date. Repeats retain separate ordinals. No future outcome fills a gap.
- Progress uses recorded work/base duration. Days left require a previous daily gain of the same focus; do not estimate through gaps. Progress-budget refusal leaves ordered completions usable.
- Missing pictures use a target icon; artwork failures are diagnostics, with no visible disclosure.
- Artwork is dated to the shown or confirming hour, scoped to client/source/branch/country/content. Pending conditional art never keeps an older choice; defined empty names stay empty.
- Titles show localized text, loading or unavailable wording. Hover reveals the focus key and any refusal reason.
- Opening a completed row requests its description at that row's recorded hour. A refused description retains successful titles and icons and contributes its named, counted diagnostic. Client, source, branch, country or content changes close previous detail demand.
- Diagnostics update when their text changes and clear on unmount; equal warnings do not restart a render cycle while text or images load.
- Dated completed rows offer Jump to their confirming read; undated rows do not. Enter/Space toggle details. Batch all wanted artwork without truncating the list.

## Files

| File | Responsibility |
|---|---|
| [NationalFocusWidget.tsx](NationalFocusWidget.tsx) | Current card, completion rows and interactions. |
| [focusData.ts](focusData.ts), [useFocusArtwork.ts](useFocusArtwork.ts) | Validation/local lookups and dated artwork requests; adjacent tests. |
| [definition.ts](definition.ts), [index.ts](index.ts), [nationalFocus.css](nationalFocus.css) | Metadata, registration and layout. |

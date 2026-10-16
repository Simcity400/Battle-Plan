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

## Prepared queries

Clients with prepared queries read the selected country's hour metadata and one
completion page. They read dated artwork pages for that page and the active
focus, using each completion's confirming hour or its undated order segment's
first hour. A repeated token completed and active at different hours keeps both
presentations. Current progress and its known daily gain remain exact decimal
strings until formatting.

The completion list uses the legacy disclosure rows and shared measured ordered
list, including independent expanded descriptions and the final unfinished row.
It renders only the viewport; First, Previous, Next and Last controls appear when
more than one completion page makes them necessary, exposing every ordinal without
retaining whole-branch histories. Follow initially opens the newest page; Pin
and Whole campaign open the first. Counts and page positions use exact decimal
strings and BigInt. Requests are cancelled on changed client, source, revision,
country, content, page or hour. Pending display belongs to its original hour;
known gaps and completed missing reads do not retain it. Native slot and country
id bind that carry: a reused tag never carries a replacement holder's data. A
null point while preparation is active stays pending until accepted coverage
grows; null after readiness is genuine missing data, with exact table reasons
reported separately. Readiness polling keeps its 500 ms cadence through repeated
status while a block is preparing; an instant wait with unchanged status stops
with a retryable error. Numeric order remains
available when dated artwork refuses. Readiness polls only queued or preparing
work, starts an idle source once, and stops on ready or terminal failures.
Focus-hour replies wait for accepted coverage because confirming dates, order
segments and daily gain require preceding observations. Native raw progress alone
does not provide the ordered, dated presentation shown here.

Opened-row descriptions read prepared text-only pages at their confirming or
undated order hour. Icon failures do not contaminate those text requests. An
absent prepared detail section retains the existing dated oracle for opened rows. Clients without prepared queries use the historical
product renderer, including its virtual list and keyboard interactions.

## Settings

Country and campaign/follow/pin; Whole campaign is the default. There are no widget-specific options.

## Notable behaviour

- Newly observed completions show the date of their confirming read; its hint says the focus completed by then and, since focuses are read once a game day, may have finished a day or so earlier. Entries already in the first readable snapshot have no known date. Repeats retain separate ordinals. No future outcome fills a gap.
- Whole campaign includes the unfinished focus as the final noninteractive item in the same measured ordered list as completions. Its list position and size include every completed and unfinished row. Keyboard destinations and entry boundaries include only completed rows; Tab leaves after the final completion and backward entry returns to its final control, including from a viewport showing only unfinished content. The completion eligibility predicate stays stable across content renders so unchanged lists reuse their keyboard index.
- Measured history rows preserve fractional layout heights and the authored space between entries, including after expanded descriptions and across offscreen rows. Measurements remain in layout pixels under content zoom. There is no extra gap below the final entry.
- Progress uses recorded work/base duration. Days left require a previous daily gain of the same focus; do not estimate through gaps. Progress-budget refusal leaves ordered completions usable.
- Missing pictures use a target icon; artwork failures are diagnostics, with no visible disclosure.
- Artwork is dated to the shown or confirming hour, scoped to client/source/branch/country/content. Pending conditional art never keeps an older choice; defined empty names stay empty.
- Titles show localized text, loading or unavailable wording. Hover reveals the focus key and any refusal reason.
- Opening a completed row requests its description at that row's recorded hour; closing releases the request. While its read has no answer, a row shows the description it already showed, so it closes with its text and reopens without waiting; a refusal shows none. A refused description retains successful titles and icons and contributes its named, counted diagnostic. Client, source, branch, country or content changes close previous detail demand and forget shown descriptions.
- Completed rows open and close by revealing their height while fading, clipped to the row so the rows below slide rather than jump or show through. A description that arrives after its row opened grows in the same way. Reduced motion makes both instant.
- Diagnostics update when their text changes and clear on unmount; equal warnings do not restart a render cycle while text or images load.
- Dated completed rows offer Jump to their confirming read; undated rows do not. Enter/Space toggle details. Batch all wanted artwork without truncating the list.
- Parity builds expose the complete ordered completion view model, including the unfinished row, exact ordinals and supplying hours. Reading the model starts no additional artwork or description requests. Ordinary builds expose no provider.

## Files

| File | Responsibility |
|---|---|
| [NationalFocusWidget.tsx](NationalFocusWidget.tsx) | Current card, completion rows and interactions. |
| [PreparedFocusWidget.tsx](PreparedFocusWidget.tsx), [preparedFocusData.ts](preparedFocusData.ts) | Prepared hour/page requests, exact validation and bounded completion display; adjacent tests. |
| [focusData.ts](focusData.ts), [useFocusArtwork.ts](useFocusArtwork.ts) | Validation/local lookups and dated artwork requests; adjacent tests. |
| [definition.ts](definition.ts), [index.ts](index.ts), [nationalFocus.css](nationalFocus.css) | Metadata, registration, layout and row motion; [nationalFocusMotion.test.ts](nationalFocusMotion.test.ts) checks the motion. |

The parity build registers prepared focus rows during loading and gathers every
completion page only after the capture preparation handshake. It preserves exact
ordinals, completion dates, progress integers and actual dated artwork evidence.
Missing numeric pages fail preparation. Production continues to read one selected
completion page. Independent static text is stored per observed token.

Accepted hour reads bypass readiness polling. Full-model preparation reuses the
settled visible page, its independent static text and opened descriptions,
fetching only unseen pages. Visible static descriptions remain available before
an opened row's dated text arrives. Each static-text batch validates inputs and
reads the store manifest once; the full parent model owns parity for its list.
Different supplying hours that reference the same immutable presentation page
share one read while retaining both dated artwork keys.

Focus parity uses one shown-hour model on both paths. The historical product
includes static metadata for future focus tokens too; the comparison selects all
completed, current and continuous focus tokens at the shown hour and retains every
static Artwork field. Unreferenced future metadata cannot affect rendering, Jump,
progress or opened details. Ordered dated observations retain resolved titles,
icons, duration, static descriptions, explicitly demanded descriptions, pending
state and named refusals. Reading the baseline model calls its ordinary presentation
getter and starts no additional content demand. Prepared parity reads independently
stored static text in requested token pages and Text pages for an opened row.

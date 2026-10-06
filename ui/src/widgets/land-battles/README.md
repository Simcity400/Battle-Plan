# Land battles

Land battles uses the shared `widgets/battles/BattleLog`: style-5's Event feed
and Scoreboard through the log and table kits. Battle log is the default view.
Biggest battles ranks by **Casualties** (default), **Peak size** or **Length**,
and shows all three figures in every table row and log tooltip. With no recorded casualty figures it
ranks by peak size and says so. Top N defaults to 30; its numeric setting has no
upper limit beyond JavaScript's finite representation. Header sorting applies
within that selected leaderboard. Live pins ongoing rows first and uses the
kit's pulse; unknown endings remain visible as **Ending not recorded**.

The Event feed has one primary line per battle: date, native battle location,
attacker flags/count, defender flags/count and result. Month dividers use the log
kit when enabled. Counts pluralise divisions; reserves and missing casualties are
explained once above the rows. Peak, length and bounds are explained in the row
tooltip. The Scoreboard columns are Battle, Date, Casualties, Peak divisions,
Length, Sides and Winner, after the rank. Unknown casualties/results are dashes
with reasons in tooltips; peak dates stay in tooltips. Measured flag summaries
show `+n` for countries that do not fit, retaining every participant in tooltips.

Place names follow combat-history `CCombatItem::UpdateDisplay@0x1416E07C0` ->
province display resolver `0x140E69320`: recorded rename, owner-qualified/generic
victory-point name (`0x140E68E30`), then state display name (`0x1409CC380`).
The shared place-names product resolves this through the recorded content stack.
It returns one native location name; no numeric province prefix or appended state.
An unresolved name displays **Unnamed province**, with its ID only in the tooltip.

`land-battles-manifest` supplies recording-block windows and checks availability
only through the first complete hourly LandCombats read (including an empty
census). It does not build the branch census. `land-battles` supplies one block's
histories through the existing `useProductWindows`. The log requests the
playhead block, the next in Live, and blocks reached by drawn rows, with the
preceding block ready for scrolling. Unwanted windows are released. Earlier /
Latest battles navigation also reaches older windows when a country filter has
no rows there. No block duration, page size or country limit is chosen here.
The shared product cache still caches visited replies under main's policy.

Biggest battles uses `land-battles-ranking`: one exact whole-range server ranking
and at most the user-selected top N summaries, with one last supplying read per
battle. Its normal reply grows with N and participant lists, not campaign hours
or number of episodes. Top N has no imposed upper limit; oversized replies
(including diagnostics) refuse at `MAX_PRODUCT_REPLY_BYTES`, never silently omit
rows. Casualties falls back to peak size while unavailable. Header sorting is
within the chosen leaderboard. Peak size and length include the whole observed
battle, including pre-range reads, and the widget says so.

The log filters either side locally with `countryMatch`. Exact server top N must
filter before selecting leaders, so `groups.countryMatchFilter` serializes the
same country/group predicate for ranking only. There is no country/request cap
added by this product. Unknown participant lists are marked **Countries not
recorded** and kept under any country filter because they may involve the chosen
country. Shared selection/storage caps remain the Economy lead's work.

The census stores exact affine counter runs (first hour and native duration;
every included read increments by one); any different step splits the run.
There is no interpolation over unread hours. It retains the two most recently
used branch censuses, evicting older branches as a retention policy. An ending
at a window boundary carries only its final recorded read.

Side counts are the latest/last complete recorded roster, counting distinct
handles including reserves, labelled **Divisions (incl. reserves)**.
`readHour` dates those counts; each country has its own supplying hour.
`place.hour` is the stable start read, used for cached place names. Peak size is the largest total
simultaneous recorded roster in the episode, dated to its earliest peak hour.
A missing beginning or ending makes that peak a minimum. End events are shown
only once their observation hour is reached; prefetched future windows cannot
supply later peaks, names, warnings or counts.

A complete adjacent absent census proves exact observation starts/ends; only a
hidden preceding hour makes a start `by`. Gaps split episodes even when the
same native handle/province reappears. Missing endings stay visible. At the
recording end, an open episode at the latest complete census remains ongoing
(`end: null`) and pulses in Live. **Ending not recorded** applies after an
unread/incomplete census or block closes the episode, or while showing beyond
its last complete present read without a later complete census. An absent
adjacent read instead establishes an exact end. We never claim continuation
through a hidden read. No playhead or absent read extends
a duration. Length is a minimum from the present-read span and the native
resolved-round counter; after a gap its counter baseline resets to the segment's
first read, including the recording's first read. Day counts continued rounds, not calendar days.

The casualty adapter is prepared for hourly cumulative reads and post-flush
terminal reads. `LandBattleHistory.losses` contains chronological
`LandBattleLossRead { hour, terminal, sides: [BattleValue[], BattleValue[]] }`
entries, attacker then defender. Each side's free-text loss labels include
**Manpower lost** and **Equipment lost**; per-variant equipment values can be
supplied alongside the total, with distinct recorded-content display labels.
They project directly into `BattleSide.losses`; the shared row shape is unchanged.
Casualties ranks the sum of both sides' **Manpower lost**. Equipment quantities
are shown separately, never added to people or counted twice through their
variant breakdown. Known zero is ranked; an unknown side makes the total unknown.
Unknown totals follow known totals. Only when no eligible row has a known total
does ranking fall back to peak size with the existing explanation.

Live reads show **so far**, with tooltips saying **<value> so far, as of <read
hour>**. Biggest battles explains that running battles rank by their losses so
far. Only a recorded `terminal: true` post-flush read is displayed as final;
disappearance alone cannot finalize losses. An ended battle with only an earlier
live read still says **so far** and reports that terminal losses are not recorded.
Prefetched terminal values never appear before their read hour. Window replies
carry one preceding loss read plus changes in that window; ranking carries the
latest supplying read and any terminal read, keeping ordinary top-N replies
independent of campaign length. The transport budget still refuses oversize data.

Capture integration is intentionally left to the lead at merge. The fetched
`origin/capture/mil-land` at **c2aeef32a656ed8f402779e93ac821b2768610cb** still
has no casualty row: `crates/replay/src/land_rows.rs` defines
`LandCombatResultsRow` (combat/province/end hour, sides and winner data),
`LandResultSide` (countries/commander/outcome/progress) and
`DestroyedDivisionsRow` (division/owner/combat references/hour). None contains
manpower or equipment loss amounts. Its `crates/collector/README.md` explicitly
defers those reads. Main therefore supplies an empty loss-read list, preserving
**Not in this recording** and the explained peak-size fallback. No capture
branch was merged or compiled and no provisional raw replay schema was invented.

At merge, add the eventual capture tables to the selected decode and join loss
reads to the existing branch/combat/province episode. Normalize deltas within
complete coverage, or take changed-only cumulative values from reconstructed
complete reads; preserve their supplying hour. A missed loss read or invalid
coverage must supply explicit unknown values, not carry an old known total
through the gap. Mark terminal only after FinalizeCombat's pending-damage flush,
and use the result's exact end hour. Resolve variant names through the existing
recorded-content equipment helpers, preserving/refusing each unsupported value
by name. The product notes cite native counter widths/writers and the hourly
round dispatcher. War-wide totals and snapshot differences cannot substitute.
Every winner remains **Result not recorded** until the result adapter is wired.

Only rows drawn by `onVisibleRows` request names and flags. Stable province/start
pairs use the existing product cache, so unchanged places do not request again
on Live ticks. `place-names` is a
generic province/hour product: province and state names, or strategic-region
names for state-less provinces, through recorded content layers. The shared
`scripted_text::state_display_name` serves places, Occupied states and Raids;
its state-content cache also serves scripted text. One text context is built per
decoded frame. A known province survives a missing state name. Country names
and flags use their own supplied read hours, with the ordinary minimum cache
allowance or the number of drawn hours, whichever is larger. Right-click Jump targets the start, or supplying
hour if no start exists. Diagnostics stop at the selected hour. The census-read
notice appears only when the latest complete census precedes that moment.

Manifest, window and ranking replies carry plain `contentWarnings` for the
shared product boundary's content diagnostics. Window/ranking `warnings` remain
typed, dated battle diagnostics. The widget reports content diagnostics through
the shared frame's notices, deduplicating across loaded windows, the manifest
and place-name replies; battle diagnostics still stop at the selected hour.

## Cost

The ignored Rust benchmark (`cargo test -p dashboard land_battles_nine_years_5000_countries_cost_and_reply_sizes -- --ignored --nocapture`) prints census and wire scan p50/p95 per built hour and the manifest, window and top-30 reply sizes, for 5,000 countries over nine years with one concurrent combat per 100 content provinces (`LAND_BATTLES_BENCH_PROVINCES`, default 20,000). Replies are windowed, so none grows with campaign length; the only bound is `MAX_PRODUCT_REPLY_BYTES`, refused rather than truncated.

# Occupied states

The `occupied-states` product supplies one reply per branch; playback is a local
lookup. Live follows the playback hour (at most ten table refreshes/second), and
Live off shows the campaign end. The table sorts by exact resistance descending
initially; clicking a header saves its column and direction with the widget.
The shared [table kit](../../charts/table/) owns density, cell bars, rank markers,
windowing, keyboard access, motion, scrolling and Jump to.
The widget supplies its row menu with the shared `jumpItem` helper. Its name,
country, law and Garrison columns opt into custom missing-value rendering to
retain the explicit state-id, Unknown, Not in this recording and native dash
fallbacks. Equal sort values order by the stable state row key.

Columns are State, Owner, Occupier, Resistance, Compliance, Garrison and
Occupation law. The countries picker and its recorded groups filter occupiers
by default; the gear can instead match the original occupied country. That
country may differ from the current legal owner. Country names and flags are
shown independently of human/AI status. They are
requested at the shown hour through the shared country-label hooks; state names
reuse the native custom-name / stored owner-name / content-template precedence.
Tags remain opaque UTF-8 strings, including spaces. Empty, NUL-containing or
undecodable recorded tags are named and counted as unresolved; the dashboard's
string contract cannot safely identify those countries.
Content names and icons resolve through the recorded vanilla/DLC/mod stack.
Missing names show their state id or law token; missing art has an empty frame.
Plain law sprites follow their native horizontal or vertical frame direction.
Animated, masked, custom-effect and nonstandard geometry forms are named,
counted refusals because a static icon cannot reproduce their native rendering.

Occupation membership follows the game's active-resistance test, **not**
controller different from owner. `StartResistance@140F85ED0` stores the original
occupied tag in `CResistance+0x50`; `RefreshGarrisons@140FE9420` requires that link
to be positive and `CheckActivation(...,1)@140F86330` to return the stored active
byte. Annexed non-core states can remain occupied. Owner uses the shown hour's
States link. Occupier, the occupied country,
garrison/law/default keys and their reason texts use the controller and
CountryLinks snapshot at the daily read supplying the values. A liberation or
change of controller between reads never joins yesterday's values to today's
controller. Country labels still resolve at the shown hour.

Resistance, compliance and their daily speeds are the exact stored fixed5
values. Nothing is interpolated; sort, bars and tooltips retain the exact inputs.
Cell text follows the native status control: `14155CC50` → `14155BE80` rounds
half away from zero (`1424D8760`), narrows to i32 and clamps to 0–100, then
`141554BD0` → `142259590` formats whole percentages. Thus 41.3354% shows as 41%
and 9.40099% as 9%; the tooltip retains both exact values and daily speeds.
Their read hours come from
`metadata::slot_read`, with `readNote(row.hour, shownHour)` in the tooltip.
Daily joins run only on read frames; other hours extend runs and re-check only
hourly owner and stored name inputs. Unchanged table identities skip that
projection too. Equal consecutive reads compress into state runs while
retaining separate read dates. Missing
hours end those runs; carried data after a gap cannot restart them without a
fresh read. Known empty inventories and incomplete inventories remain distinct.
An incomplete hourly state or country catalogue also ends coverage, since
the row identities and controllers cannot be established safely.
The context menu jumps to the read supplying the displayed occupation values.

The native presentation and recording limitations are explicit:

- The native law lookup (`140F84BB0` → `140FE2FE0`) first takes a state override,
  then the occupied-country law at `CCountryOccupationData+0xA0`, then the
  occupier's default and content fallback. The group pointer is not captured.
  Explicit recorded state overrides can be shown; without one the widget says
  **Not in this recording**, even when OccupationSettings records a default.
  A raw state key zero is never interpreted as that group pointer. The native
  default branch also validates scope (`140A665A0`); it cannot be reached safely
  by skipping the missing input.
- The Garrison column ports the occupied-state row (`14155CC50`). Its text is
  the recorded required template amount at +0xF8, formatted with two fixed
  decimals, truncated toward zero (`1424B6200` → `1424B89F0`, mode 1), without
  grouping. Zero or missing garrison data shows **-** with no bar. A nonzero
  +0x60 flag shows a red **!** and an empty strength bar. `140FE78B0` writes
  that requirement failure flag from a
  negative amount and preserves the negative raw +0xF8 value. The exact raw
  amount stays available in the tooltip.
  The shared `formatNativeFixed5` also preserves the formatter's signed i32
  whole-part conversion (`1424B7A80`), including its overflow behavior; sorting
  and tooltip values keep the original fixed5 integer.
- Only the bar uses displayed strength (`140FE01E0(...,true)`). It is established
  as zero for the failure flag, or zero assigned manpower from a complete,
  resolved `StateAssignedManpower` read. `140C5A0F0` sums that container with
  wrapping i32 arithmetic; a complete empty container is zero, an incomplete
  or unresolved one is unknown. Known strength is compared with the stack's
  `NInterface.GARRISON_STRENGTH_TO_SHOW_RED` (`14331F240`); an unknown threshold
  never invents red styling. A positive amount's bar is hidden when strength
  is unknown. Equipment allocation/requirements (+0x90/+0xD0,
  `140B8A4E0`) are the remaining unrecorded input. Assigned manpower and
  `GARRISON_STR_POW_*` defines are available. The stored +0xF0 ratio is never
  substituted. The tooltip shows exact values, signed daily speeds, assigned
  and required manpower, the occupied country's name at the shown hour, and
  the read note.

Payload validation and lookups have node tests; native membership, law
precedence, link aliases, 5,000 countries, gaps and modded content have Rust
tests. The table benchmark measures CPU sort/motion/window preparation for
5,000 rows; browser layout and paint need the lead's visual/performance check.

For that check, use [the browser development setup](../../../README.md#real-recordings-in-the-browser):
the development adapter is `crates/dashboard/src/bin/dashboard-bridge.rs`.
Choose a recording folder and matching game/user folders in `ui/.env.local` or
Settings, run `npm.cmd run dev:ui`, load the recording, enter layout edit mode,
add **Occupied states**, and save. Choose all countries to inspect the whole
inventory; test Live, both densities, both sort directions, scrolling, keyboard
and context menus. Compare the table with `style-5.html` in the planning mockups.
`npm.cmd run dev` starts the desktop setup as well.

# Scoreboard

The moment table follows playback through the shared ten-refresh-per-second
table clock. Settings choose the columns, countries, exact sort, density,
cell bars and rank markers. The default is the numerical top ten, re-ranked
when the recorded sort measure changes. There is no combined score.

The `scoreboard` product owns the single column registry and reuses the
single-stat products' shared measures. The UI retains every recorded decimal,
shows missing amounts as a dash and dates held values through `dashboard/reads`.
Country names and flags are requested at each row's shown hour; country keys
retain recorded native identity across tag changes, while ties use current tags.

Army size, casualties, planes and ships stay in the default set, with their
shared Military measures' pending reasons in header hints and row tooltips.
They each have one backend adapter ready for the Military branches. Right-click
has no app actions; the browser menu remains available for copying selectable
table text. Sorting Country alphabetises the numerical top N; its measure
can be changed under Countries settings and defaults to factories.
Sorting an unavailable measure retains that available Top N measure, descending,
while displayed rows tie by current tags.

Country requests are not capped or batched. Manual picks, groups and Top N use
one request; only the shared `MAX_PRODUCT_REPLY_BYTES` budget refuses an oversized
reply by name. Country and column choices have no shared count or token-length
cap, and large selections survive saved-layout reloads.

`charts/table` owns the style-5 header, faint scaled bars, exact missing-last
sorting, rank markers, moving rows and viewport windowing. Node fixtures cover
missing values, exact values, reads, identity changes and 5,000 countries.
Browser layout/paint and live recordings remain separate checks.

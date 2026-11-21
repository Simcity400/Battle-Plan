# Read power spending by fixed chunks and project it at the shown hour

Read aligned chunks independent of the view and join a country's chunks in supplying-read order. Each chunk contributes only entries read in its own span, and coverage continuations join across chunk boundaries. Requested range bounds use the selected period while a pin needs its branch prefix. A payment has its own date, a supplying read, and an ordering witness. Filter group membership by the payment's date and expose the entry only after the supplying read. Use native hour, action sequence and ordinal for stable row order.

Keep one-off rows and fold recurring days per country into monthly category or item rows. A recurring cost arrives as its day ends. A month visible during playback uses only its days supplied by then, showing the latest included date. A category filter changes which rows display while preserving each row's amount. As a followed block advances, reuse rows whose payments and figures are still the same. Request target names at their dated payment rows and comparative country names at their own row dates.

Keep exact recorded amounts, count unpriced payments, and identify uncovered stretches. Missing coverage is not proof that a country spent nothing. Carry partial status through grouped slices and preserve category amounts when item identity is missing. A running sum lets a moving frame look up gains and spending at a read by binary search. Track payment release boundaries separately from recurring-row and coverage updates so the view can stage and reveal payments at the right time.

Preserve legacy sort order and filter choices while a saved layout migrates. Wait for the category catalogue to interpret old Hide choices. The hidden marker disambiguates already migrated choices, and the former explicit no-category option is now treated as showing all categories.

[Power spending product projection](../../ui/src/dashboard/powerSpending.ts) · [Legacy settings migration](../../ui/src/dashboard/powerSpendingOptions.ts)

# Espionage operations log

The `espionage-log` product covers every country's operations in the chosen
branch. The shared log supplies style-5 rows, density, month headers, virtual
scrolling, row motion, tooltips and Jump to. Chips select operation types from
the campaign's recorded content stack, with their localised names and artwork.

Live puts preparing and running operations first, then completed or disappeared
operations newest first. Campaign mode shows the branch-end history newest
first. The row date is the observation supplying the row; an ending says **by**
its confirming read. Flags and country names use that row date. Execution start
is shown only once a recorded read establishes it, and Jump to offers that
start and a known ending, falling back to the row date or first read when
neither is known. Preparation observed before execution has no invented start
date. Missing reads leave progress, phases and assignments unknown.
Consecutive off-slot hours retain a successfully supplied daily read and its
date; phase, preparation and execution progress are never recomputed between
reads. A gap row's
date remains its last actual read, rather than the last hour carrying it.

While Live, preparing operations show recorded overall civilian preparation;
running operations show execution elapsed / duration at the supplying read.
Elapsed days use the native helper at `1413EC160`: subtract the executable epoch
in signed i32 arithmetic, divide each date separately by 24 with truncation
toward zero, then subtract the days. Its -1 result (including zero duration,
terminal state or elapsed equal to duration) remains unknown. The tooltip names
the elapsed / duration days and supplying read; the meter never interpolates.
The preparation ratio does not establish equipment readiness. The tooltip
shows its native truncated percent and the recorded-read phase index.
Phase names and operative names are not recorded. Only ended operations show
an outcome, through one capture adapter that currently says Outcome not recorded.
Operatives retain their recorded handles. Completed means the recorded terminal
state; disappearance says ended by the next complete read. Neither implies
success, failure or capture.

Choices where the specification is silent: default matching countries as owner,
with target and either available; ongoing rows newest start first, with highest
preparation available; type chips rather than outcome chips; the shared log's
comfortable density, chips and Live entry motion on, month headers off. Identity
and unknown-country rows remain visible under All countries. Content choices
too long for saved settings say why they are unavailable.
The widget starts in Campaign mode, with Live and a date range available; a
range narrows the displayed read and ending dates. Its minimum is 4×6 grid
units and default is 9×14. Category tags say Espionage, so the operation type
appears once in the sentence; tags use the shared accent.
Rows without a Live progress meter use date, artwork, sentence and category
tracks, with the status inline after the sentence. Preparing and running rows
while Live keep their separate recorded-progress track beside the sentence.

Episode invariants (type, countries, selected-phase count, repeat, duration and
slot count) are stored once; a change starts another episode. Daily runs retain
only dated changing values and the preparation ratio as a fixed5 integer.
The product accounts for its complete serialized history reply, leaving space
inside the transport limit and refusing oversized histories.

Synthetic Node tests cover mod names/art, execution progress and sentinels, missing reads, future start knowledge,
renamed countries, dated disappearance and 5,000 country operations. Browser
layout and live-recording capture correctness require separate verification.

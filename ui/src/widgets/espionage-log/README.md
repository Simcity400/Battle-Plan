# Espionage operations log

The `espionage-log` product covers every country's operations in the chosen
branch. The shared log supplies style-5 rows, density, month headers, virtual
scrolling, row motion, tooltips and Jump to. Chips select operation types from
the campaign's recorded content stack, with their localised names and artwork.

Live puts preparing and running operations first, then completed or disappeared
operations newest first. Campaign mode shows the branch-end history newest
first. The row date is the observation supplying the row; an ending says **by**
its confirming read unless a completion event supplies an exact date. Flags and
country names use that row date. Execution start
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
an outcome, through one capture adapter that currently says **Outcome not recorded**
and names the missing stored bonus, detection and effect description.
Operatives retain their recorded handles. Completed means the recorded terminal
state; disappearance says ended by the next complete read. Neither implies
success, failure or capture.

Choices where the specification is silent: default matching countries as owner,
with target and either available; ongoing rows newest start first, with highest
preparation available; type chips rather than outcome chips; the shared log's
comfortable density, chips and Live entry motion on, month headers off. Identity
and unknown-country rows remain visible under All countries. Type settings
retain every chosen content token, with no shared count or length cap.
The widget starts in Campaign mode, with Live and a date range available; a
range narrows the displayed read and ending dates. Its minimum is 4×6 grid
units and default is 9×14. Category tags say Espionage, so the operation type
appears once in the sentence; tags use the shared accent.
Rows without a Live progress meter use date, artwork, sentence and category
tracks, with the status inline after the sentence. Preparing and running rows
while Live keep their separate recorded-progress track beside the sentence.

An operation keeps its native handle and raw owner, target and definition-token
identity. Preparation and launch stay in the same operation. A finished operation
that reopens starts another episode; changing repeat, duration, selected phases,
slots or tags changes the dated run, never ends the operation. Failed country or
link reads leave unknown tags at that read, without reusing an older tag. A
definition name missing from the recording leaves the operation type unknown;
later lexical data is never backfilled. This recording gap is distinct from a
named content miss. Daily runs retain the preparation ratio as a fixed5 integer.
The product sizes its history from the recording and refuses a reply beyond the
shared transport budget; it never truncates countries, operations or runs.

Completion-event coverage has its own availability and **Not recorded** reason
on main. The contract can also carry a completion-only operation, its exact
completion hour and decimal sequence, and the independent stored bonus and
detection flags. Effect-description text carries either its captured value or a
named refusal reason, with its exact required byte length and recorded client
language. Non-UTF-8 description bytes are refused by name rather than decoded
with replacement characters. Refusing text does not lose the flags or language.
These fields remain unrecorded until the capture tables
supply them. Raw bonus or detection bytes outside 0 and 1 refuse the outcome
by field name and value, with counted warnings.

Synthetic Node tests cover mod names/art, preparation through launch, dated
mutable values and tags, execution progress and sentinels, missing reads and
lexical names, late start knowledge, dated disappearance, completion-only
outcomes and 5,000 country operations. Browser
layout and live-recording capture correctness require separate verification.

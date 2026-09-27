# replay

The shared recording contract. This crate owns tables, rows, frames, cadence, sample provenance, spool records, replay containers, branches and codecs. Recording and playback must agree here; consumers must not invent their own interpretation of the bytes.

## Sources of truth

Start with [src](src/), especially [format.rs](src/format.rs) for format support and [cadence.rs](src/cadence.rs) for scheduling. Keep version numbers, table counts, IDs and slot assignments in code rather than copying catalogs into READMEs.

Rows preserve engine-native identifiers and numeric representations. Encoding is explicit and lossless. Reject malformed, out-of-range or noncanonical inputs rather than silently coercing them. Compression and deltas may save space; reduced precision may not.

## Values, availability and freshness

Country references in rows are not necessarily canonical slot numbers. Resolve raw references through the frame's `CountryLinks`; physical country slots can be reused. Preserve the recorded raw ID and do not turn an unresolved link into a guess about the current country.

A frame belongs to one hour on one branch. Tables can be snapshots, deltas, unchanged, truncated or unavailable. Missing data is not an empty successful table. After losing a complete baseline, a later successful table needs a snapshot before deltas can resume.

Value equality and sample freshness are separate:

- A fresh successful acquisition may equal the previous values and encode as `Unchanged`, while advancing its sample origin.
- A carried table retains the original sample hour, even though it appears in a later frame.
- Failed, truncated or skipped acquisition must not be marked as a fresh complete sample.
- Sliced tables (`Cadence::Sliced`) are read one owner slice per hour. Each hour replaces only the rows of the owners read at that hour, so one complete table mixes rows read at different hours. SliceReads holds each owner's read hour per table (`sliced::read_hour`); rows carry no hour, so an owner's unchanged rows stay unchanged when it is read again. The owner's read hour, never the frame's hour, is when its values were true. TechnologyDefinitions is the one keyed sliced table: it has no owner or read hour; a complete read replaces it, and an incomplete read replaces only the rows whose keys it returned.

Keep those distinctions through encoding, decoding, inspection, analytics and oracle joins. Do not infer freshness from a sibling table or the current frame's hour. Startup and branch/load transitions retain the full-refresh behavior required by the format. Keyframes stay self-contained, but a recorder may build them from the complete rows it already holds instead of reading every table again: a carried table in a keyframe is a snapshot of its last read, not a new sample, and its table cost is zero.

`Replay::read_block_parallel` decodes a block's frame payloads on the caller's worker threads once their boundaries are known. It returns the same frames and the same first error as `read_block`. Only a caller that owns spare cores uses it, such as the viewer's playback read-ahead; the collector and the recorder read sequentially.

Consumers that need only a few tables can use `FrameDecoder::for_tables`. It applies the same baseline and recovery rules to those tables and reports omitted tables as unavailable. The selection is fixed for the decoder lifetime; ordinary `FrameDecoder::new` still reconstructs every table. `Replay::read_block_selected` complements this by retaining only selected updates during parsing. All updates still go through the same decoder and validation before unselected ones are discarded; frame ordering, bounds, keyframe and compression checks remain shared with the full reader. The decompression buffer reserves the bounded size from the block index once. Neither selection API changes the recording format.

## Compatibility and recovery

A format change updates all affected writers, readers and consumers together. This release reads and writes only the current format. Reject other replay versions at the header and incompatible spool records explicitly; do not infer or silently translate an older layout.

Every spool record carries the current recording version; recovery checks it before decoding the payload and keeps only valid CRC-checked prefixes. Replay finalization and append must not destroy an existing good file when interrupted. Preserve session/branch identity and never join branches through merely probable parentage.

File spools keep record writes ordered while allowing one periodic durability request per segment to run in the background. Pending errors are checked before subsequent writes and on idle polls; a failed segment is abandoned through the existing recovery path. Segment creation, forced boundaries and detach remain synchronous durability barriers. Every outstanding worker is joined before its sink is destroyed. On Windows the durability handle reopens the existing file through its handle, separately from the synchronous writer; no path-based replacement or extra capture-buffer queue is involved.

The replay stores content identity, ordered layer hashes and the coverage policy, not game assets or machine-specific paths. Asset resolution belongs in [installed-content](../installed-content/README.md).

## Checking changes

[fixed_layout.rs](src/fixed_layout.rs) lists which recorded tables and playback calculations use each fixed modifier key and static token, and checks playback keys against complete ModifierDefinitions and LexicalTokens. A missing row or dictionary, a registered gap or a different script spelling produces a named refusal. Expected names live only in engine; collector refusals use the existing unavailable-table encoding, so the format is unchanged.

Use existing round-trip, delta/baseline, malformed-input, version-rejection, provenance, branch and recovery tests. Validate affected collector, recorder, viewer and analytics consumers in the same change. See [AGENTS.md](../../AGENTS.md) for commands.

CountryAppearance keeps each country's stored map colour and name operands, and the ruling party's first country leader: its ideology token and CCharacter id. The id is missing, never zero, when there is no such leader or its character could not be read.

ModifierDefinitions keeps each registered modifier key's native metadata by index, with its script name as a lexer token resolved through LexicalTokens; consumers decode the flag words. HiredAdvisors keeps each country's appointed advisors in native order, keyed by raw country reference and ordinal; resolve the country through CountryLinks like any other raw reference.

CountryStabilityInputs keeps one row per country of the stability, war support and maximum command power operands in native representation (fixed5, Q15 penalty words, raw predicates, loaded defines, tracked modifier totals with their resolved key indices, the command power reserved by allocators, and the game language); consumers compute the displayed values. The maximum command power the game shows and clamps the balance to is `GetMaxCommandPower` minus that reserved amount. CountryStabilityLines keeps each modifier tree child's value per tracked key, keyed by country, tree position and native key, with the source kind and identity the recorder proved; only an unpaired line keeps the game's display text. Both are sliced by country; use each row's SliceReads hour.

The hourly research tables (ResearchInputs, ResearchAppliedBonuses and ResearchModifiers) keep slot-local inputs, applied-bonus attribution and modifier operands as native values. Country-wide modifier rows share one reserved slot index; slot rows identify the exact technology and retain speed components, source text, sharing witnesses, category filters, eligibility metadata and define values. The codec owns its kind tags.

# store

Windows prepared-data files in `%LOCALAPPDATA%\HOI4Replay\prepared\<campaign-id>\`.
`Store::local` selects a campaign; `SourceKey::capture` records the recording and
the ordered files supplied by the resolved content stack, including mod archives.
Reuse compares paths, sizes and modification times only. Missing or inaccessible
inputs report an error; a changed key is a cache miss. Capture validates inputs
when the recording/content binding is established. Cloned keys share that
binding and immutable stamps; page reads check only recording metadata and the
manifest reference. Content changes require a new binding. Commit reads the
recording's metadata every time and all input sizes and modification times once a
minute has passed since the key last found them current (capture counts), so a
content stack of thousands of files is not read again at each of a preparation's
commits; content edited within that minute is published under the old stamps,
which the edited content's next binding refuses. A failed input check
invalidates the shared binding. Input checks use scoped threads bounded by the
available CPU count, retaining input order and the first input error; unavailable
CPU information uses one thread. Ordinary metadata reads follow links as usual.

`is_reusable` binds the key to the manifest's separate stamp file once.
`read(key, section, range)` returns a `Range` for a zero-based range ordinal: an
owned handle on the range's pack, positioned at the range and bounded to its
length, with `Read`, `Seek`, `len` and `id`. It stays valid while held, through
replacement, Clear and later commits. Producers define range boundaries and
encode their own data; each saved range is immutable, and a replaced range
always gets a new `RangeId`. Opening and lookup read a header and at most one
entry, independent of campaign length. Memory holds input/section metadata, a
copy buffer and newly added entries.

`replace(key)` starts a full rebuild; `append(previous, key)` retains the prepared
prefix and adds ranges. Append requires recording growth, unchanged content and
the producer's recording-index reuse decision. The store checks growth and
content metadata; it does not prove byte identity. `Writer::write` streams one whole range;
`commit` rechecks source metadata as above and publishes the manifest last. Unequal keys
are never accepted by ordinary reopen.

`Writer::bytes_written` reports the actual new range payload bytes in a
transaction, so background producers can persist their disk totals without
walking the directory on progress polls.

Each writer appends the ranges it writes to its own pack file,
`pack_<random-id>.bin`, created by its first write. Bytes are buffered (1 MiB),
then written in order; nothing in a pack is rewritten. An empty range lives in no pack:
its entry names pack 0, which no pack file has, and reading it opens nothing, so
removing a pack never removes an empty range. A range whose producer
fails partway leaves no bytes a later range keeps, and commit trims any tail it
left. `write_at` replaces a partial tail page at its existing logical range, or
appends exactly the next range. It appends a new range and swaps only that
manifest entry at commit; held readers retain the old bytes. A transaction
keeps replacement entries only for changed pages and streams unchanged entries.
`payload_bytes_change` subtracts superseded payloads from newly written bytes.
`Writer::read_at` opens the latest staged or published range in its transaction,
writing out the buffer first when it still holds the range, allowing producers
to evict and resume partial page buffers before commit.

`Writer::fork` gives another thread a writer in the same transaction, with its
own pack. It reads the transaction as it stood when forked, plus its own ranges.
`end_batch` closes a batch and `into_staged` returns the batches in order, with
the fork's written-out pack. `Writer::adopt` lists one batch in the parent
exactly as direct writes would have: the same section numbers, ranges, order and
byte counts, each entry naming the fork's pack. Adoption rechecks every range the
fork read or replaced and refuses a batch whose ranges another adoption changed
(`ErrorKind::Other`), so forks must use disjoint sections; the transaction then
refuses to commit, since part of the batch may be listed. A fork holds a
duplicate of the writer's lock handle and cannot commit; ranges of batches never
adopted stay unlisted, and a pack that holds no listed range is removed at the
next writer open. Manifest entries are read at their offsets, so a writer and
its forks share one manifest handle.

`extend(key)` adds more accepted ranges for the same unchanged recording, so
cancelled preparation can resume from its saved accumulators. `range_count`
reads a section's published count from the header, without scanning its ranges.
`saved_key` returns published input metadata even after an input changes; the
producer still compares the recording's accepted index before choosing `append`.

After accepted growth, `read_prefix` and `prefix_range_count` read the old
published accumulator ranges against the new input key. They check recording
path/size and unchanged content metadata; the caller applies its recording-index
reuse policy. Ordinary reads continue to reject the stale input key.

The flat `manifest` contains a little-endian header length, a versioned JSON
header (layout version 3, source-file reference and section names/counts), then
consecutive 24-byte entries per section (pack id, offset and length,
little-endian), then the pack table: one 24-byte record per listed pack (id,
listed bytes, length). Offset arithmetic selects a range entry without scanning
earlier ranges; only writers read the pack table. A manifest of an older layout,
such as one file per range, reads as absent: reuse fails, the dashboard prepares
again, and the next writer open removes its files. Appends rewrite only the
manifest, streaming its old entries; old packs stay intact.
The ordered input stamps occupy an immutable `source_<id>.json`, written once
per source key and retained across commits for that key. Reopen reads this list
once; queries do not load it again. IDs identify files, not content hashes.
`Listing` opens a manifest, or a copy of one, read-only for measurement tools.

`session(key)` returns a `Session`, a verified view of the published manifest:
one recording metadata check and one manifest read, after which its reads and
range counts repeat neither, and ranges in one pack share a handle. A pack a
later writer removed before the session opened it is read through the
then-current manifest, as unscoped reads are. `scope(key)` installs a session on
the calling thread until its guard drops: every read, range count and reuse
check of that key through a store of this directory uses it, so a query checks
its key and reads the manifest once whatever its readers do. Other keys and
other threads stay unscoped. With `test-support`, `counters` reports each
thread's metadata checks, manifest reads and pack opens.

`read_many` opens requested ranges in order after one recording metadata check
and manifest read; ranges in one pack share its handle. Missing ranges remain
missing, duplicates remain ordered, and each returned range keeps its own bytes
across publication or Clear. `read_many_counted` also returns each section's
published range count, letting a series reader reuse its first opened range
without repeating those checks.

`commit` writes the new manifest under a sibling temporary name, flushes each
pack the transaction lists once, on scoped threads bounded by the CPU count,
flushes the temporary manifest and the directory, POSIX-replaces the manifest,
then flushes it and the directory again. A commit of any number of ranges thus
creates one file per writer that wrote, plus the manifest's temporary, and
issues one flush per listed pack plus the manifest's. Source stamps are
published the same way. A publication error after replacement can leave the
complete new file visible. Unsupported POSIX volumes are refused. The
`test-support` feature, for test builds only, skips these flushes; the
dashboard's tests enable it through their dev-dependency.
One exclusive-open `writer.lock` serializes writers and Clear. Writer opening
deletes every name other than the lock, the manifest, its stamps and the packs
its table lists, in one directory pass; memory holds the listed pack ids.

Garbage is bounded. The pack table records each pack's listed bytes, and a
commit subtracts the bytes of the ranges it replaces from their packs. Every pack
left with less than half its length listed, published or written in the
transaction, is emptied at that commit: its listed ranges are copied, in manifest
order, into one new pack, their entries are rewritten, and the old pack is no
longer listed (nor flushed). A pack no range lists leaves the table. So after
every commit each listed pack holds at least as many listed bytes as dead ones,
and the listed packs total at most twice the listed range bytes; packs of
earlier transactions that no manifest lists remain only until the next writer
open. Every copied byte was preceded by at least as many bytes made dead, so
copying costs at most the bytes written. Readers never see the move: held ranges
keep their old pack, and new reads see the same bytes under new entries.

`clear` removes the manifest first, then packs and temporary names with POSIX
delete. Open handles keep their bytes until close; new reads see the current
manifest or a cache miss. Reads racing name removal retry the current manifest.
The empty directory and zero-byte lock file remain. There is no campaign snapshot
handle: every returned range is independently held by its reader.

Synthetic tests cover replacement, held readers, Clear, abandoned publication,
metadata changes, immutable append, writer exclusion, forks adopted in order
against the same writes made directly, refused overlapping forks, an older
layout read as absent, failed ranges, range bounds and seeking, emptied packs
under held readers, and manifest-only opening with 1,756,800 hourly entries
representing two sections over 100 years. A counted test commits 10 and 1,000
ranges through a writer and two forks and checks that both create and flush the
same files. Empty ranges stay readable after the pack they were written beside is
removed, and through an open, uncommitted append. A scope test counts one metadata check, one manifest read and one
pack open for 150 reads, and checks held handles and removed packs. Ignored
timing tests measure reopen and page reads with 40,000
physical input files, and the commit of 5,000 ranges of 2 KB as packs against
one file per range, with real flushes.

# store

Windows prepared-data files in `%LOCALAPPDATA%\HOI4Replay\prepared\<campaign-id>\`.
`Store::local` selects a campaign under `local_root`, and `local_space` reports the
free and total bytes of the volume holding them; `SourceKey::capture` records the recording and
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

`is_reusable` binds the key to the manifest's separate stamp file once. `manifest_identity` reports the manifest file's length and write and creation
times (none before the first commit); a commit replaces the manifest and Clear
removes it, so callers compare identities to notice another writer.
`read(key, section, range)` returns a `Range` for a zero-based range ordinal: an
owned handle on the range's pack, positioned at the range and bounded to its
length, with `Read`, `Seek`, `len` and `id`; reading it to the end sizes the
buffer for the rest of the range at once (a bounded step at a time), so the
bytes arrive in one read. It stays valid while held, through
replacement, Clear and later commits. Producers define range boundaries and
encode their own data; each saved range is immutable, and a replaced range
always gets a new `RangeId`. `RangeId::cache_key` exposes its three numeric
identity operands as a serializable array for transient caches, independent of
display formatting and pack handles. Opening reads the manifest root and its names;
a lookup then reads one entry from one entry log, whatever the section's range
count. Memory holds input/section metadata, each section's runs, a copy buffer
and newly added entries.

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
keeps replacement entries only for changed pages; unchanged entries stay where
they are.
`payload_bytes_change` subtracts superseded payloads from newly written bytes.
`Writer::range_id` returns the current staged or published range identity
without opening its pack or flushing buffered bytes. A fork records this as a
read dependency, so adoption validates identity-based reuse just as it
validates payload reads.
`Writer::read_at` opens the latest staged or published range in its transaction,
writing out the buffer first when it still holds the range, allowing producers
to evict and resume partial page buffers before commit.

`Writer::fork` gives another thread a writer in the same transaction, with its
own pack. It reads the transaction as it stood when forked, plus its own ranges.
`end_batch` closes a batch and `into_staged` returns the batches in order, with
the fork's pack written out and flushed on the calling thread: a flush failure
is that fork's, forks flush at once rather than in their parent's commit, and
the commit does not flush the pack again. `Writer::adopt` lists one batch in the parent
exactly as direct writes would have: the same section numbers, ranges, order and
byte counts, each entry naming the fork's pack. Adoption rechecks every range the
fork read or replaced and refuses a batch whose ranges another adoption changed
(`ErrorKind::Other`), so forks must use disjoint sections; the transaction then
refuses to commit, since part of the batch may be listed. `Writer::adopt_all`
lists a fork's batches in order, or none of them: a refused batch takes back
every step of the batches before it (sections, ranges and byte counts), so the
transaction stays exactly as it was and publishable. Several producers'
batches thus share one commit (one root, one entry log and one flush set),
each whole or not at all, and a refusal stays its producer's. A fork holds a
duplicate of the writer's lock handle and cannot commit; ranges of batches never
adopted stay unlisted, and a pack that holds no listed range is removed at the
next writer open. Entries are read at their offsets, so a writer and its forks
share one catalog of the published manifest.

`Writer::stage` detaches a long batch from transaction publication. It reads the
snapshot at creation and its own writes, including ordinary forks adopted into
it for parallel flushing. The batch's reads and replaced ranges record immutable
range identities and its sections are resolved by name. `into_staged` returns
batches that `adopt_all` can join to a later transaction of the same directory
and source key, checking every dependency and taking back the whole batch on a
conflict. Other producers can commit meanwhile. The stage and its returned
batches retain the exclusive lock, and pin snapshot files and range identities:
cleanup retains its snapshot's entry logs and packs and the packs written by
the stage and its nested forks. Those packs keep their range identities;
unrelated packs and logs are cleaned up and compacted normally. Names and source
stamps are already loaded in the snapshot. Ordinary nested batches can join
only their own stage; ordinary sibling batches stay in their publishing
transaction. A stage's batches may join separate commits: each
commit adds its ranges' live bytes to any earlier publication of the same pack.
Compacting such a pack moves both earlier and newly adopted ranges. No payload
is copied to rebase a batch. `commit_continue` publishes and opens the next
transaction under a duplicate of the same lock handle, so detached stages do
not prevent continuation or leave an ownership gap. Ordinary forks remain tied
to their original transaction.

`extend(key)` adds more accepted ranges for the same unchanged recording, so
cancelled preparation can resume from its saved accumulators. `range_count`
reads a section's published count from the root, without scanning its ranges.
`saved_key` returns published input metadata even after an input changes; the
producer still compares the recording's accepted index before choosing `append`.

After accepted growth, `read_prefix` and `prefix_range_count` read the old
published accumulator ranges against the new input key. They check recording
path/size and unchanged content metadata; the caller applies its recording-index
reuse policy. Ordinary reads continue to reject the stale input key.

The `manifest` is a small root: a little-endian header length and a versioned
JSON header (layout version 4, the source-file reference, the names file and how
many of its bytes the root lists, and its table lengths), then one 16-byte
record per section (range count, run count), the sections' runs (32 bytes each:
first range, range count, entry log id, offset of the first entry), the log
table and the pack table (24 bytes per listed log or pack: id, listed bytes,
length). Entries (24 bytes: pack id, offset and length) live in entry logs,
`entries_<random-id>.bin`, each written whole by one commit and never changed;
a run names consecutive ranges whose entries lie consecutively in one log, so a
lookup finds its run by binary search and reads one entry at offset arithmetic.
Section names occupy `names_<random-id>.bin` (each a 4-byte length and its UTF-8
bytes, in section order); a commit that creates sections appends their names
past the bytes the published root lists, writing over any that an unpublished
transaction left there, so every published root reads an unchanged prefix. A
replacement starts new logs and a new names file. A manifest of an older
layout, such as the flat manifest of every entry or one file per range, reads
as absent: reuse fails, the dashboard prepares again, and the next writer open
removes its files. Appends write only new entries; old logs and packs stay
intact.
The ordered input stamps occupy an immutable `source_<id>.json`, written once
per source key and retained across commits for that key. Reopen reads this list
once; queries do not load it again. IDs identify files, not content hashes.
`Listing` opens a manifest read-only for measurement tools, and
`Listing::write_snapshot` writes a self-contained copy (its sections, then every
entry in manifest order) that `Listing::open` reads without the store's names
or logs.

A commit writes, into one new log, only the entries it changed: each section's
replaced and moved ranges as runs of consecutive ranges, laid over the runs
they cover in one ordered pass that visits each run and run of changes once,
and its appended ranges as one run; unchanged entries keep their runs.
A new last run absorbs the run before it while that one is no longer, so an
appended section's runs halve in length towards its end and each entry is
copied once per doubling of its section. A section with more than 32 runs is
rewritten as one, within that same pass, reading each published run once. A log left under half live is emptied into the commit's log,
as packs are, and the commit's log holds each run it lists once, so every
listed log holds at least as many live entries as dead ones. The root grows
with the sections, runs, logs and packs it lists, not with range counts.

`session(key)` returns a `Session`, a verified view of the published manifest:
one recording metadata check and one manifest read, after which its reads and
range counts repeat neither, and ranges in one pack share a handle, as entries
in one log do. A pack or log a later writer removed before the session opened
it is read through the then-current manifest, as unscoped reads are. `scope(key)` installs a session on
the calling thread until its guard drops: every read, range count and reuse
check of that key through a store of this directory uses it, so a query checks
its key and reads the manifest once whatever its readers do. Other keys and
other threads stay unscoped. With `test-support`, `counters` reports each
thread's metadata checks, manifest reads, pack opens and entry log opens.

`read_many` opens requested ranges in order after one recording metadata check
and manifest read; ranges in one pack share its handle. Missing ranges remain
missing, duplicates remain ordered, and each returned range keeps its own bytes
across publication or Clear. `read_many_counted` also returns each section's
published range count, letting a series reader reuse its first opened range
without repeating those checks.

`commit` writes its entry log and appends its names, flushes once each pack the
transaction lists that no fork flushed, the log and the names file, on scoped
threads bounded by the CPU count, then writes the root under a sibling
temporary name, flushes it and the directory, POSIX-replaces the manifest, and
flushes it and the directory again. A commit is all or nothing: until the root
is replaced no reader or writer sees its log, names or packs, and a process
ending before then leaves the previous root, whose listed names prefix and logs
are unchanged, so the store opens at the previous commit and the next writer
open removes the rest. A commit of any number of ranges thus creates one file
per writer that wrote, its entry log and the root's temporary, and issues one
flush per pack no fork flushed, its log and its names, plus the root's. Source
stamps are published the same way. A publication error after replacement can
leave the complete new root visible. Unsupported POSIX volumes are refused. The
`test-support` feature, for test builds only, skips these flushes; the
dashboard's tests enable it through their dev-dependency.
One exclusive-open `writer.lock` serializes writers and Clear. Writer opening
deletes every name other than the lock, the manifest, its stamps, its names
file, the packs and logs its tables list, and live stages' pinned files, in one
directory pass. Pack creation registers a stage's pin under the same lock as
cleanup, so a concurrent directory pass cannot remove a new stage pack.

Garbage is bounded. The pack table records each pack's listed bytes, and a
commit subtracts the bytes of the ranges it replaces from their packs. A pack
the transaction wrote that is left with less than half its length listed is
emptied at that commit: only the commit's own entries name it. Finding the
entries that name a published pack reads every entry, so published packs under
half live are emptied together, at a commit after which the listed packs would
hold more than a quarter dead bytes. An emptied pack's listed ranges are copied,
in manifest order, into one new pack, their entries are rewritten in the
commit's log, and the old pack is no longer listed (nor flushed). A pack no
range lists leaves the table. Unpinned listed packs
total after each commit at most twice their listed range bytes, and packs of earlier transactions that no
manifest lists remain only until the next writer open. Every copied byte was
preceded by at least as many bytes made dead, so copying costs at most the
bytes written. Readers never see the move: held ranges keep their old pack, and
new reads see the same bytes under new entries.

`clear` removes the manifest first, then packs, logs, names and temporary names
with POSIX delete. Open handles keep their bytes until close; new reads see the current
manifest or a cache miss. Reads racing name removal retry the current manifest.
The empty directory and zero-byte lock file remain. There is no campaign snapshot
handle: every returned range is independently held by its reader.

Synthetic tests cover replacement, held readers, Clear, abandoned publication,
metadata changes, immutable append, writer exclusion, forks adopted in order
against the same writes made directly, refused overlapping forks, producers
adopted into one commit against each committing alone (the same sections and
bytes, with one root, log and flush set instead of one each), a refused
producer taken back whole while the others commit, detached batches crossing
successive publications with nested flushes, their snapshot identities and lock
retained, rejected cross-source batches and atomic rollback of conflicting
reads or replacements, shared stage packs adopted across separate commits and
kept readable through later replacement and cleanup, compaction of their earlier
published ranges, bounded disk use during repeated page replacements beside a
live stage and its nested batch, rejected ordinary batches crossing stage
boundaries, an older layout read as
absent, failed ranges, range bounds and seeking, emptied packs under held
readers, and opening a root with 1,756,800 hourly entries representing two
sections over 100 years, which reads the root and one log entry. Reads after
240 commits of appends, tail, middle and first-range replacements, empty
ranges, new sections, forks, extensions and appends return every range's last
write, with every section within 32 runs, every listed log at least half live
and held ranges unchanged, and a snapshot lists the same entries. Scattered
replacements rewrite a section as one run and empty the log they left under
half live, keeping every entry. Replacing every odd ordinal of 100,000
published ranges in one transaction visits each of its 100,000 pieces once and
writes each entry once. A commit stopped after its log and names are
durable, before its root, leaves the previous root and reads; the next writer
removes the log and writes its names over the ones left. Over 300 commits that
each append a range and replace a page beside 100,000 untouched ranges, the
untouched entries keep their run and each commit writes under 32 KB of
manifest. A counted test commits 10 and 1,000 ranges through a writer and two
forks and checks that both create and flush the same files. Empty ranges stay readable after the pack they were written beside is
removed, and through an open, uncommitted append. A scope test counts one
metadata check, one manifest read, one pack open and one log open for 150
reads, and checks held handles and removed packs. Ignored
timing tests measure reopen and page reads with 40,000
physical input files, and the commit of 5,000 ranges of 2 KB as packs against
one file per range, with real flushes.

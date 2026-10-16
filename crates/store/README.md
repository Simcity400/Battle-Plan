# store

Windows prepared-data files in `%LOCALAPPDATA%\HOI4Replay\prepared\<campaign-id>\`.
`Store::local` selects a campaign; `SourceKey::capture` records the recording and
the ordered files supplied by the resolved content stack, including mod archives.
Reuse compares paths, sizes and modification times only. Missing or inaccessible
inputs report an error; a changed key is a cache miss. Capture validates inputs
when the recording/content binding is established. Cloned keys share that
binding and immutable stamps; page reads check only recording metadata and the
manifest reference. Content changes require a new binding, and commit validates
all input sizes and modification times before publication. A failed input check
invalidates the shared binding. Input checks use scoped threads bounded by the
available CPU count, retaining input order and the first input error; unavailable
CPU information uses one thread. Ordinary metadata reads follow links as usual.

`is_reusable` binds the key to the manifest's separate stamp file once.
`read(key, section, range)` returns
an ordinary file handle for a zero-based range ordinal. Producers define range
boundaries and encode their own data; each saved range is immutable. Opening and
lookup read a header and at most one file entry, independent of campaign length.
Memory holds input/section metadata, a copy buffer and newly added file names.

`replace(key)` starts a full rebuild; `append(previous, key)` retains the prepared
prefix and adds ranges. Append requires recording growth, unchanged content and
the producer's recording-index reuse decision. The store checks growth and
content metadata; it does not prove byte identity. `Writer::write` streams one whole range;
`commit` rechecks source metadata and publishes the manifest last. Unequal keys
are never accepted by ordinary reopen.

`Writer::bytes_written` reports the actual new range payload bytes in a
transaction, so background producers can persist their disk totals without
walking the directory on progress polls.

`write_at` replaces a partial tail page at its existing logical range, or appends
exactly the next range. It writes a new immutable file and swaps only that
manifest entry at commit; held readers retain the old bytes. A transaction
keeps replacement IDs only for changed pages and streams unchanged entries.
`payload_bytes_change` subtracts superseded payloads from newly written bytes.
`Writer::read_at` opens the latest staged or published range in its transaction,
allowing producers to evict and resume partial page buffers before commit.

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
header (source-file reference and section names/counts), then consecutive 16-byte file names
per section. A name identifies `<section-index>_<range-ordinal>_<random-id>.bin`.
Offset arithmetic selects a range entry without scanning earlier ranges. Appends
rewrite only the manifest, streaming its old entries; old data files stay intact.
The ordered input stamps occupy an immutable `source_<id>.json`, written once
per source key and retained across commits for that key. Reopen reads this list
once; queries do not load it again. IDs identify files, not content hashes.

`read_many` opens requested ranges in order after one recording metadata check and
manifest read. Missing ranges remain missing, duplicates remain ordered, and each
returned handle keeps its own bytes across publication or Clear.
`read_many_counted` also returns each section's published range count, letting a
series reader reuse its first opened file without repeating those checks.

Whole files use sibling temporary names, `publication::sync_file`, POSIX replace,
then another file/directory flush. A publication error after replacement can
leave the complete new file visible. Unsupported POSIX volumes are refused.
The `test-support` feature, for test builds only, skips both flushes; the
dashboard's tests enable it through their dev-dependency.
One exclusive-open `writer.lock` serializes writers and Clear. Writer opening
deletes unlisted files in one directory pass with bounded metadata memory.

`clear` removes the manifest first, then data and temporary names with POSIX
delete. Open file handles keep their bytes until close; new reads see the current
manifest or a cache miss. Reads racing name removal retry the current manifest.
The empty directory and zero-byte lock file remain. There is no campaign snapshot
handle: every returned range file is independently held by its reader.

Synthetic tests cover replacement, held readers, Clear, abandoned publication,
metadata changes, immutable append, writer exclusion, and manifest-only opening
with 1,756,800 hourly file entries representing two sections over 100 years.
An ignored timing test measures reopen and page reads with 40,000 physical input
files, counting input metadata checks and stamp-file reads.

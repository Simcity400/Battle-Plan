# store

Windows prepared-data files in `%LOCALAPPDATA%\HOI4Replay\prepared\<campaign-id>\`.
`Store::local` selects a campaign; `SourceKey::capture` records the recording and
the ordered files supplied by the resolved content stack, including mod archives.
Reuse compares paths, sizes and modification times only. Missing or inaccessible
inputs report an error; a changed key is a cache miss.

`is_reusable` reads only the manifest header. `read(key, section, range)` returns
an ordinary file handle for a zero-based range ordinal. Producers define range
boundaries and encode their own data; each saved range is immutable. Opening and
lookup read a header and at most one file entry, independent of campaign length.
Memory holds input/section metadata, a copy buffer and newly added file names.

`replace(key)` starts a full rebuild; `append(previous, key)` retains the prepared
prefix and adds ranges. Append requires recording growth, unchanged content and
the producer's knowledge that the old recording prefix is unchanged. Metadata
alone cannot prove that fact. `Writer::write` streams one whole range;
`commit` rechecks source metadata and publishes the manifest last. Unequal keys
are never accepted by ordinary reopen.

The flat `manifest` contains a little-endian header length, a versioned JSON
header (source key and section names/counts), then consecutive 16-byte file names
per section. A name identifies `<section-index>_<range-ordinal>_<random-id>.bin`.
Offset arithmetic selects a range entry without scanning earlier ranges. Appends
rewrite only the manifest, streaming its old entries; old data files stay intact.

Whole files use sibling temporary names, `publication::sync_file`, POSIX replace,
then another file/directory flush. A publication error after replacement can
leave the complete new file visible. Unsupported POSIX volumes are refused.
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

# dashboard-keyed

Keyed observation families: the shared way a widget's history is prepared when
each hour reads as a set of named exact values (a region's planes on one
mission, an owner's mines in one sea, one channel of a region's air
statistics) and the widget looks the playback hour up locally.

A product crate declares an [`Observer`](src/lib.rs): the tables it reads, the
fields whose changes make an hour worth observing, the table whose supplying
read dates its values, and how one frame turns into `(name, value)` pairs. This
crate does the rest the same way for every such family:

- [block.rs](src/block.rs): the family's block pre-pass entry
  (`block::family`). A pre-pass worker reads each block alone, only at the hours
  the watched fields change, and keeps those frames and, for the hours between,
  whether the dating table was read. The fold observes the kept hours on the
  carried scan and holds the values through the quiet hours; when the carried
  scan does not continue into a quiet hour it observes the rest of the block
  hour by hour.
- [lib.rs](src/lib.rs): the carried [`Scan`]. It numbers each name once, keeps
  the last observed values, and publishes runs through the shared series
  accumulator: equal values extend their run without a new observation, a name
  no longer read closes its run, and an hour whose inputs are not recorded
  (`observe` returns false) closes every run. Two reserved keys hold the hours
  the inputs were read (`KNOWN`) and the supplying read hour of the values
  (`READ`). After consecutive refused hours, an observation that resumes with
  no fresh read is dated by the read held before them only when the observer
  vouches for it (`Observer::supplying_read`); a gap drops it. The catalogue is
  the scan itself, so a branch continues from its
  last flush.
- [query.rs](src/query.rs): reading the pages for a query. `Query` carries the
  request, the prepared files and the prefix of the family's gate; `open` reads
  the catalogue, and `Opened::window` returns a `KeyedWindow`: the known spans,
  the read stretches and the runs of the names a predicate selects, clipped to
  the accepted prefix.
- [countries.rs](src/countries.rs): country name parts. A raw reference resolves
  through the hour's links to its canonical slot and through the hour's
  countries to its tag, spelled `TAG@slot`, so a reused tag never merges two
  countries.

What a window says, for every name: a run holds the exact value read from its
first to its last hour; a known hour without a run holds no value for the name
(its inputs were read and it was not among them); an hour outside the known
spans is not recorded. Values are signed 128-bit integers on the native scale
and travel as decimal text.

## Use it

1. Implement `Observer` for a serializable observer (its carried state; runtime
   caches `#[serde(skip)]`) with a `static FAMILY = block::family::<_>(...)`.
   Keep names compact: every distinct name is a series key for the branch.
   An observer that needs content (geography, defines) implements `bind`, which
   the dashboard calls with the block's source and content before every fold.
2. Write the product's query over `Query` and `Opened::window`, selecting only
   the names the request needs.
3. In the dashboard, add a `Keyed<YourObserver>` to the registry of families
   prepared on their own (`prepared/pipeline/independent/keyed.rs`), its refusal
   family and section label, and route its product key in `prepared/query.rs`,
   which marks the gate's prefix (`coverage`, `pending`) on the reply.
4. In the frontend, read the window with `useProduct` and look hours up with
   [keyedHistory.ts](../../ui/src/dashboard/keyedHistory.ts).

`cargo test -p dashboard-keyed` runs this crate's tests.

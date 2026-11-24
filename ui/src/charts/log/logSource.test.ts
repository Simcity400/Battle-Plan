import { gatedHour } from '../../test-support/gated.ts';
import assert from 'node:assert/strict';
import test from 'node:test';
import { aheadThrough, askedPages, binPages, countsAhead, cumulativeCounts, drawnAhead, failedPage, logPageRequest, missingNote, missingThrough, noPages, pageFirstHour, pageInSpan, pagesOf, rowsAhead, shownPages, shownThrough, validateLogCounts, validateLogPage } from './logSource.ts';
import type { LogRow } from './logData';
import { tailFollowParams } from '../../families/table/logHelpers.ts';

const hours = [100, 101, 104, 106, 106, 110, 111, 112, 115, 117];
const counts = validateLogCounts({ branchId: '1', log: 'test', firstHour: 100, binHours: 6, missing: [], warnings: [],
  lines: [{ before: '20', total: '10', bins: [[0, 3], [1, 4], [2, 3]] }] }, 100, 117);
const before = cumulativeCounts(counts);
const pageOf = (size: number, held: ReadonlySet<number>) => (index: number) =>
  held.has(index) ? hours.slice(index * size, (index + 1) * size).map(hour => ({ hour })) : undefined;

test('the cut counts every row through its hour, from the bins before it and the held rows of its own bin', () => {
  const every = new Set([0, 1, 2, 3]);
  const expected = (until: number) => hours.filter(hour => hour <= until).length;
  for (let until = 95; until <= 125; until++) {
    assert.deepEqual(shownThrough(counts, before, until, 3, pageOf(3, every)), { shown: expected(until), known: true }, `through ${until}`);
  }
  assert.deepEqual(shownThrough(counts, before, null, 3, pageOf(3, new Set())), { shown: 10, known: true }, 'no cut shows the span');
});

test('a cut inside a bin whose page is not held shows only the bins before it, and says so', () => {
  assert.deepEqual(shownThrough(counts, before, 108, 3, pageOf(3, new Set([1]))), { shown: 3, known: false });
  assert.deepEqual(shownThrough(counts, before, 108, 3, pageOf(3, new Set([1, 2]))), { shown: 5, known: true });
  assert.deepEqual(shownThrough(counts, before, 111, 3, pageOf(3, new Set([0]))), { shown: 3, known: false }, 'other pages do not stand in');
});

test('pages are fixed ordinal windows from the oldest row, the newest cut at the rows shown', () => {
  const rows = (index: number): LogRow<number>[] => hours.slice(index * 4, (index + 1) * 4).map((hour, at) => ({ key: `${index * 4 + at}`, hour, item: hour }));
  const loaded = new Map([[0, rows(0)], [1, rows(1)]]);
  const failure = new Error('The page could not be read.');
  const pages = shownPages(7, 4, loaded, new Map([[1, failure]]));
  assert.deepEqual(pages.map(page => [page.key, page.start, page.end, page.count]), [['0', 0, 3, 4], ['1', 4, 6, 3]]);
  assert.equal(pages[0].rows, loaded.get(0), 'a whole page shown keeps its rows');
  assert.deepEqual(pages[1].rows?.map(row => row.hour), [106, 110, 111], 'the newest page holds only the rows shown');
  assert.equal(pages[1].error, failure);
  assert.deepEqual(shownPages(9, 4, new Map(), new Map()).map(page => [page.key, page.count, page.rows]), [['0', 4, undefined], ['1', 4, undefined], ['2', 1, undefined]],
    'a page not held keeps its count');
  assert.deepEqual(shownPages(0, 4, loaded, new Map()), []);
});

test('a counts reply sums its groups into the bins of its span and keeps the rows before it', () => {
  assert.deepEqual(counts, { firstHour: 100, binHours: 6, counts: [3, 4, 3], total: 10, before: 20, missing: [] });
  const tagged = validateLogCounts({ firstHour: 96, binHours: 12, lines: [{ tag: 'GER', before: '2', total: '3', bins: [[0, 1], [2, 2]] },
    { tag: 'ENG', before: '1', total: '1', bins: [[1, 1]] }] }, 100, 130);
  assert.deepEqual(tagged, { firstHour: 96, binHours: 12, counts: [1, 1, 2], total: 4, before: 3, missing: [] }, 'bins without rows are zero, groups summed');
});

test('counts and pages that do not add up, run out of order or start elsewhere are refused', () => {
  const line = { before: '0', total: '3', bins: [[0, 1], [1, 2]] };
  assert.throws(() => validateLogCounts({ firstHour: 0, binHours: 6, lines: [{ ...line, total: '4' }] }, 0, 11), /malformed/, 'bins short of the total');
  assert.throws(() => validateLogCounts({ firstHour: 0, binHours: 0, lines: [] }, 0, 11), /malformed/);
  assert.throws(() => validateLogCounts({ firstHour: 0, binHours: 6, lines: [line] }, 0, 5), /malformed/, 'a bin past the span');
  assert.throws(() => validateLogCounts({ firstHour: 6, binHours: 6, lines: [line] }, 0, 11), /malformed/, 'bins from another hour');
  const cell = (value: string) => ({ kind: 'number', value });
  const page = { branchId: '1', log: 'test', columns: ['kind'], firstOrdinal: '24', total: '10', cursor: '26', missing: [], warnings: [],
    rows: [{ key: '7', hour: 110, ordinal: '24', cells: [cell('1')] }, { key: '9', hour: 111, ordinal: '25', cells: [{ kind: 'notRecorded' }] }] };
  assert.deepEqual(validateLogPage(page, 4, 4, 20), { firstOrdinal: 4, total: 10,
    rows: [{ key: '7', hour: 110, cells: { kind: cell('1') }, fields: page.rows[0] }, { key: '9', hour: 111, cells: { kind: { kind: 'notRecorded' } }, fields: page.rows[1] }] });
  assert.throws(() => validateLogPage(page, 0, 4, 20), /malformed/, 'a page from another cursor');
  assert.throws(() => validateLogPage({ ...page, rows: [...page.rows].reverse() }, 4, 4, 20), /malformed/, 'rows out of order');
  assert.throws(() => validateLogPage(page, 4, 1, 20), /malformed/, 'more rows than asked');
  assert.throws(() => validateLogPage({ ...page, total: '5' }, 4, 4, 20), /malformed/, 'rows past the total');
  assert.throws(() => validateLogPage({ ...page, rows: [{ ...page.rows[0], cells: [] }, page.rows[1]] }, 4, 4, 20), /malformed/, 'a row without its cells');
});

test('the hours a log was not recorded whole are kept with their reasons, cut at the list\'s head, and noted as not empty', () => {
  const line = { before: '0', total: '1', bins: [[0, 1]] };
  const missing = [{ firstHour: 100, lastHour: 119, reason: 'The market was not read' }, { firstHour: 130, lastHour: 131, reason: 'The market-events log was read in part here (2 rows not read)' }];
  const held = validateLogCounts({ firstHour: 96, binHours: 6, lines: [line], missing }, 100, 140);
  assert.deepEqual(held.missing, missing);
  assert.deepEqual(missingThrough(held.missing, 110), [{ ...missing[0], lastHour: 110 }], 'cut at the head; a later span not reached');
  assert.deepEqual(missingThrough(held.missing, null), missing);
  const note = missingNote(missingThrough(held.missing, 140), hour => `h${hour}`, 'Market events are not recorded whole');
  assert.deepEqual(note?.spans, [{ text: 'Market events are not recorded whole from h100 to h119.', detail: 'The market was not read' },
    { text: 'Market events are not recorded whole from h130 to h131.', detail: 'The market-events log was read in part here (2 rows not read)' }]);
  assert.match(note!.counted, /recorded hours only/);
  assert.deepEqual(missingNote([{ firstHour: 0, lastHour: 3, reason: 'x: not available (y)' }], hour => `day ${Math.floor(hour / 24)}`)?.spans,
    [{ text: 'Not recorded on day 0.', detail: 'x: not available (y)' }], 'hours of one day are one date; the recording’s wording is the detail only');
  assert.equal(missingNote([], String), null);
  assert.throws(() => validateLogCounts({ firstHour: 96, binHours: 6, lines: [line], missing: [{ firstHour: 120, lastHour: 110, reason: 'x' }] }, 100, 140), /malformed/);
  assert.throws(() => validateLogCounts({ firstHour: 96, binHours: 6, lines: [line], missing: [missing[1], missing[0]] }, 100, 140), /malformed/, 'out of hour order');
});

test('the page after the head is asked only while it holds rows: past the span the service answers none, which no page can be', () => {
  const head = Math.floor(counts.total / 4);
  assert.deepEqual([head, head + 1].filter(index => pageInSpan(index, 4, counts.total)), [2]);
  assert.equal(pageInSpan(0, 4, 0), false, 'an empty span asks no page');
  assert.equal(pageInSpan(-1, 4, 10), false);
  const past = { columns: [], firstOrdinal: String(20 + 12), total: '10', cursor: null, rows: [] };
  assert.throws(() => validateLogPage(past, 12, 4, 20), /malformed/, 'what the service gives for a page past the span');
});

test('a family\'s own page is asked without the log it fixes, as its strict params require; log-page is asked with it', () => {
  const span = { branchId: '1', log: 'market-events', facets: [{ name: 'kind', values: [20], countries: [] }], firstHour: gatedHour(100), lastHour: gatedHour(117) };
  const ledger = logPageRequest({ ...span, product: 'construction-log' }, 24, 500);
  assert.equal(ledger.product, 'construction-log');
  assert.deepEqual(ledger.params, { branchId: '1', facets: span.facets, from: 100, to: 117, cursor: '24', pageSize: 500 });
  const generic = logPageRequest(span, 24, 500);
  assert.equal(generic.product, 'log-page');
  assert.equal(generic.params.log, 'market-events');
  const tail = tailFollowParams({
    product: 'event-feed-tail',
    branchId: '1',
    from: gatedHour(100),
    limit: 256,
    ask: { categories: ['politics'], group: { type: 'faction', id: '42', membership: 'hour' } },
  });
  assert.deepEqual(tail, { panel: 'event-feed-tail', branchId: '1', from: 100, limit: 256, categories: ['politics'], group: { type: 'faction', id: '42', membership: 'hour' } },
    'a tail panel carries its real follow parameters and the group, never a member list');
  assert.equal('scope' in generic.params, false, 'no scope asks every row');
});

test('after a seek the cut waits on every page of its bin, and the list asking for the pages it shows releases none of them', () => {
  const waiting = binPages(counts, before, 108, 2);
  assert.deepEqual(waiting, [1, 2, 3]);
  const at = shownThrough(counts, before, 108, 2, pageOf(2, new Set([1])));
  assert.deepEqual(at, { shown: 3, known: false });
  assert.deepEqual([...askedPages(['1'], at.shown, 2, true, waiting)].sort(), [1, 2, 3]);
  assert.deepEqual([...askedPages(['1', 'family'], at.shown, 2, false, [])], [1], 'a pinned list asks only what it shows; a key not a page is no page');
  assert.deepEqual(binPages(counts, before, null, 2), [], 'no cut waits on nothing');
  assert.deepEqual(binPages(counts, before, 95, 2), [], 'nor does a cut before the span');
});

test('playing, the pages of the next two seconds of playback are asked with the block; paused, the next page', () => {
  assert.equal(aheadThrough(counts, before, 3, 104, 1, true), 7);
  assert.deepEqual([...askedPages([], 3, 2, true, [], aheadThrough(counts, before, 3, 104, 1, true))].sort(), [1, 2, 3], 'pages of rows 3 to 6');
  assert.deepEqual([...askedPages([], 3, 2, true, [], aheadThrough(counts, before, 3, 104, 1, false))].sort(), [1, 2], 'paused: the head page and the next');
  assert.deepEqual([...askedPages([], 3, 2, true, [], aheadThrough(counts, before, 3, 104, 1440, true))].sort(), [1, 2, 3, 4], 'fast: through the last row');
  const busy = validateLogCounts({ branchId: '1', log: 'test', firstHour: 100, binHours: 6, missing: [], warnings: [],
    lines: [{ before: '0', total: '100000', bins: [[0, 100000]] }] }, 100, 105);
  assert.equal(aheadThrough(busy, cumulativeCounts(busy), 10, 100, 1440, true), 100000);
  const feed = validateLogCounts({ branchId: '1', log: 'test', firstHour: 0, binHours: 30, missing: [], warnings: [],
    lines: [{ before: '0', total: String(1000 * 8), bins: Array.from({ length: 1000 }, (_, bin) => [bin, 8]) }] }, 0, 29_999);
  assert.equal(aheadThrough(feed, cumulativeCounts(feed), 800, 3_000, 1440, true), 8 * Math.floor((3_000 + 2880) / 30 + 1), 'every row through the bin two seconds on');
});

test('a page asks from the first hour its first row can hold: the start of the bin that row is counted in', () => {
  assert.deepEqual([0, 2, 3, 6, 7, 9].map(at => pageFirstHour(counts, before, at)), [100, 100, 106, 106, 112, 112]);
  const sparse = validateLogCounts({ branchId: '1', log: 'test', firstHour: 100, binHours: 6, missing: [], warnings: [],
    lines: [{ before: '0', total: '3', bins: [[0, 1], [3, 2]] }] }, 100, 125);
  assert.deepEqual([0, 1, 2].map(at => pageFirstHour(sparse, cumulativeCounts(sparse), at)), [100, 118, 118], 'empty bins between are passed over');
});

test('the rows a list draws ahead of its cut come from the pages held, without a gap and at most so many', () => {
  const pages = new Map([[0, hours.slice(0, 4).map(hour => ({ hour }))], [1, hours.slice(4, 8).map(hour => ({ hour }))]]);
  const page = (index: number) => pages.get(index);
  assert.deepEqual(rowsAhead(3, hours.length, 4, 10, page), { hours: [106, 106, 110, 111, 112], complete: false }, 'up to the third page, not held');
  assert.deepEqual(rowsAhead(3, hours.length, 4, 2, page), { hours: [106, 106], complete: false });
  pages.set(2, hours.slice(8).map(hour => ({ hour })));
  assert.deepEqual(rowsAhead(7, hours.length, 4, 10, page), { hours: [112, 115, 117], complete: true }, 'through the last row of the span');
});

test('a count painted each frame adds the rows drawn past the cut as the clock reaches them, each under its key', () => {
  const rows: LogRow<'even' | 'odd'>[] = hours.map((hour, index) => ({ key: String(index), hour, item: index % 2 ? 'odd' : 'even' }));
  const held = (indices: number[]) => new Map(indices.map(index => [index, rows.slice(index * 4, (index + 1) * 4)]));
  const ahead = drawnAhead(shownPages(10, 4, held([0, 1, 2]), new Map()), 6);
  assert.deepEqual(ahead.map(row => row.hour), [111, 112, 115, 117], 'the rows past the first six, oldest first');
  const at = countsAhead(new Map([['even', 3], ['odd', 3]]), ahead, row => row.item);
  assert.deepEqual([...at(110)], [['even', 3], ['odd', 3]], 'before the first row drawn ahead, the count at the cut');
  assert.deepEqual([...at(112)], [['even', 4], ['odd', 4]]);
  assert.deepEqual([...at(200)], [['even', 5], ['odd', 5]]);
  assert.deepEqual([...countsAhead(new Map([['odd', 0]]), ahead, row => row.item)(200)], [['odd', 2]], 'a key not counted stays out');
  assert.deepEqual(drawnAhead(shownPages(10, 4, held([0, 2]), new Map()), 2).map(row => row.hour), [104, 106, 115, 117], 'a page not held adds nothing');
});

test('failed pages bind to the shown spec and ignore answers for superseded specs', () => {
  const failure = new Error('Refused');
  const failed = failedPage(noPages<string[], string>(null), 'spec', 'spec', 5, failure);
  assert.equal(failed.lane, 'spec');
  assert.equal(failed.errors.get(5), failure, 'a failed page shows its failure, never loading');
  assert.equal(failedPage(failed, 'shown', 'spec', 2, new Error('Refused')), failed);
  assert.equal(pagesOf(failed, 'shown').rows.size, 0);
  assert.equal(pagesOf(failed, 'spec'), failed);
});

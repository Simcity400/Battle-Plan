import assert from 'node:assert/strict';
import test from 'node:test';
import { engineHour } from '../../dashboard/calendar.ts';
import { logCut, logHeldRange, logItemState, logShownOffsets, logShownOrdinals, logSpans, logStateAt } from './logAhead.ts';
import { logRowTarget } from './logPages.ts';
import { anchoredScrollTop, logItems, logOffsets, logSectionItems, MONTH_HEIGHT, ROW_HEIGHT, stableLogSections, visibleRange } from './logData.ts';
import { logPageEntries } from './logPages.ts';
import type { LogItem, LogRow } from './logData';

const at = (hours: number) => engineHour({ year: 1937, month: 3, day: 10, hour: 1 })! + hours;
const MAY = at(62 * 24);
const row = (key: string, hour: number, extra: Partial<LogRow<string>> = {}): LogRow<string> => ({ key, hour, item: key, ...extra });
const rows = [row('d', MAY), row('c', at(101)), row('b', at(100)), row('a', at(90)), row('old', at(80), { leftHour: at(101) })];

test('rows held ahead show from their reveal, a month from its first row\'s, a replaced row until its successor', () => {
  const items = logItems(rows, true);
  const spans = logSpans(items);
  const states = (hour: number) => items.map((item, slot) => `${item.kind === 'month' ? 'month' : item.key}:${logItemState(spans, slot, hour)}`);
  assert.deepEqual(states(at(100)), ['month:ahead', 'd:ahead', 'month:shown', 'c:ahead', 'b:shown', 'a:shown', 'old:shown']);
  assert.deepEqual(states(at(101)), ['month:ahead', 'd:ahead', 'month:shown', 'c:shown', 'b:shown', 'a:shown', 'old:gone']);
  assert.deepEqual(states(MAY), ['month:shown', 'd:shown', 'month:shown', 'c:shown', 'b:shown', 'a:shown', 'old:gone']);
  const late = logSpans(logItems([row('late', at(95), { revealedHour: at(105) })], false));
  assert.equal(logItemState(late, 0, at(100)), 'ahead', 'a row dated before its read waits for the read');
  assert.equal(logItemState(late, 0, at(105)), 'shown');
});

test('one row drawn with its states shows the state its hour reaches, its first before it, the last it showed after or between', () => {
  const items = logItems([row('x#ended', at(104), { id: 'x', leftHour: at(110) }), row('y', at(100)), row('x#under way', at(90), { id: 'x', leftHour: at(104) }),
    row('x#again', at(95), { id: 'x', revealedHour: at(120) })], false);
  const spans = logSpans(items);
  const slots = items.flatMap((item, slot) => item.kind === 'row' && item.row.id === 'x' ? [slot] : []);
  const stateAt = (hour: number) => { const shown = logStateAt(spans, slots, hour); return [items[shown.slot].key, shown.state]; };
  assert.deepEqual(stateAt(at(80)), ['x#under way', 'ahead'], 'before its first state: that state, held');
  assert.deepEqual(stateAt(at(90)), ['x#under way', 'shown']);
  assert.deepEqual(stateAt(at(103)), ['x#under way', 'shown']);
  assert.deepEqual(stateAt(at(104)), ['x#ended', 'shown'], 'the change applies at its hour, in the same row');
  assert.deepEqual(stateAt(at(115)), ['x#ended', 'gone'], 'between: the state it last showed, faded');
  assert.deepEqual(stateAt(at(120)), ['x#again', 'shown']);
  const lone = logSpans(logItems([row('z', at(100), { id: 'z', leftHour: at(105) })], false));
  assert.deepEqual(logStateAt(lone, [0], at(106)), { slot: 0, state: 'gone' });
});

test('the keyboard steps over the rows the clock shows only, never onto a row or state held ahead or gone', () => {
  const stops = (items: ReadonlyArray<LogItem<string>>, hour: number) => {
    const ordinals = logShownOrdinals(items, logSpans(items), hour);
    return Array.from({ length: ordinals[items.length] }, (_, ordinal) => logRowTarget(items, ordinals, ordinal)?.row?.key);
  };
  const items = logItems(rows, true);
  assert.deepEqual(stops(items, at(100)), ['b', 'a', 'old'], 'the rows waiting above are never reached; the first stop is the newest shown');
  assert.deepEqual(stops(items, at(101)), ['c', 'b', 'a'], 'a row revealed is reached, a row replaced is not');
  assert.deepEqual(stops(items, MAY), ['d', 'c', 'b', 'a']);
  const states = logItems([row('x#ended', at(104), { id: 'x', leftHour: at(110) }), row('y', at(100)), row('x#under way', at(90), { id: 'x', leftHour: at(104) })], false);
  assert.deepEqual(stops(states, at(100)), ['y', 'x#under way'], 'never the state held ahead');
  assert.deepEqual(stops(states, at(104)), ['x#ended', 'y'], 'the state reached, in its own place');
  assert.deepEqual(stops(states, at(110)), ['y']);
});

test('the layout at an hour closes up over the items not shown, and changes only where an item comes or goes', () => {
  const items = logItems(rows, true);
  const spans = logSpans(items);
  const full = logOffsets(items, 'comfortable');
  const shown = logShownOffsets(full, spans, at(100));
  assert.equal(shown[2], 0, 'the waiting month and its row take no height');
  assert.equal(shown[4], shown[3], 'nor the waiting row of the month shown');
  assert.equal(shown[shown.length - 1], full[full.length - 1] - (full[2] - full[0]) - (full[4] - full[3]));
  assert.equal(logShownOffsets(full, spans, MAY)[2], full[2] - full[0], 'later, only the gone row closes up');
  assert.equal(logCut(spans, at(100)), logCut(spans, at(100) + 0.5), 'no item comes or goes inside an hour');
  assert.notEqual(logCut(spans, at(101)), logCut(spans, at(100)), 'a row revealed and one gone at the same hour');
  const plain = logItems(rows.slice(1, 4), false), laid = logOffsets(plain, 'comfortable');
  assert.equal(logShownOffsets(laid, logSpans(plain), MAY), laid, 'every item shown: the layout as laid');
});

test('a block drawn once holds every row the view reaches until the clock leaves it, however many rows leave or arrive', () => {
  const held = [...Array.from({ length: 5 }, (_, at) => row(`ahead ${at}`, 0, { revealedHour: 190 - at * 40 })),
    ...Array.from({ length: 30 }, (_, at) => row(`shown ${at}`, -1 - at, at % 2 ? {} : { leftHour: 5 + at * 6 }))];
  const items = logItems(held, false);
  const spans = logSpans(items);
  const full = logOffsets(items, 'comfortable');
  const height = 8 * ROW_HEIGHT.comfortable;
  const laid = logShownOffsets(full, spans, 0);
  const within = ([from, to]: [number, number], [start, end]: [number, number]) => from >= start && to <= end;
  const hours = Array.from({ length: 200 }, (_, hour) => hour);
  for (const [top, anchored] of [[0, true], [0, false], [3 * ROW_HEIGHT.comfortable, true], [3 * ROW_HEIGHT.comfortable, false]] as const) {
    const drawn = logHeldRange(laid, full, spans, 0, top, height, 6, anchored);
    for (const hour of hours) {
      const now = logShownOffsets(full, spans, hour);
      const at = anchored ? anchoredScrollTop(items, laid, items, now, top) : top;
      assert.ok(within(visibleRange(now, at, height, 3), drawn), `top ${top}${anchored ? ', anchored' : ''}: the view at ${hour} is drawn`);
    }
  }
});

test('at the top, a block draws every row held ahead above the first shown, however many arrive, with no month divider above them', () => {
  const held = [...Array.from({ length: 20 }, (_, at) => row(`beginning ${at}`, 200 - at * 10)),
    ...Array.from({ length: 30 }, (_, at) => row(`under way ${at}`, -1 - at))];
  const items = logItems(held, false);
  const spans = logSpans(items);
  const full = logOffsets(items, 'comfortable');
  const height = 8 * ROW_HEIGHT.comfortable;
  const laid = logShownOffsets(full, spans, 0);
  const within = ([from, to]: [number, number], [start, end]: [number, number]) => from >= start && to <= end;
  const hours = Array.from({ length: 201 }, (_, hour) => hour);
  for (const anchored of [true, false]) {
    const drawn = logHeldRange(laid, full, spans, 0, 0, height, 6, anchored);
    assert.equal(drawn[0], 0, 'every row held ahead at the top is drawn');
    for (const hour of hours) {
      const now = logShownOffsets(full, spans, hour);
      const at = anchored ? anchoredScrollTop(items, laid, items, now, 0) : 0;
      assert.ok(within(visibleRange(now, at, height, 3), drawn), `${anchored ? 'anchored' : 'unanchored'}: the view at ${hour} is drawn`);
    }
  }
  const firstShown = 20;
  const before: [number, number] = [firstShown - 6, logHeldRange(laid, full, spans, 0, 0, height, 6, true)[1]];
  assert.ok(hours.some(hour => !within(visibleRange(logShownOffsets(full, spans, hour), 0, height, 3), before)), 'the old window misses the rows arriving at the top');
});

test('sections are one list: each under its heading, its note while it has nothing to draw, held ahead until its first row shows', () => {
  const sections = [{ key: 'going', label: 'Under way', months: false, rows: [row('g', at(100))] }, { key: 'done', label: 'Completed', rows: [row('c', at(101)), row('b', at(90))] },
    { key: 'ended', label: 'Ended', rows: [] }, { key: 'paged', label: 'Paged', pages: [{ key: 'p0', start: 0, end: 4, count: 5 }] }];
  const shape = (items: readonly LogItem<string>[]) => items.map(item => item.kind === 'month' ? `${item.part ?? 'month'}:${item.label}` : item.kind === 'page' ? 'page' : item.key);
  const browsed = logSectionItems(sections, true, false, logPageEntries);
  assert.deepEqual(shape(browsed), ['heading:Under way', 'g', 'heading:Completed', 'month:Mar 1937', 'c', 'b', 'heading:Ended', 'note:', 'heading:Paged', 'page'],
    'a section of rows has no note; an empty one has; a section may keep its rows without month dividers');
  assert.equal(new Set(browsed.map(item => item.key)).size, browsed.length, 'every item keyed apart, month dividers within their section');
  const offsets = logOffsets(browsed, 'comfortable');
  assert.equal(offsets[1] - offsets[0], MONTH_HEIGHT, 'a heading is as tall as a divider');
  assert.equal(offsets[8] - offsets[7], ROW_HEIGHT.comfortable, 'a note is as tall as a row');

  const held = logSectionItems(sections, true, true, logPageEntries);
  assert.deepEqual(shape(held), ['heading:Under way', 'note:', 'g', 'heading:Completed', 'note:', 'month:Mar 1937', 'c', 'b', 'heading:Ended', 'note:', 'heading:Paged', 'note:', 'page']);
  const spans = logSpans(held);
  const states = (hour: number) => held.map((item, slot) => `${shape([item])[0]}=${logItemState(spans, slot, hour)}`);
  assert.deepEqual(states(at(95)), ['heading:Under way=shown', 'note:=shown', 'g=ahead', 'heading:Completed=shown', 'note:=gone', 'month:Mar 1937=shown', 'c=ahead', 'b=shown',
    'heading:Ended=shown', 'note:=shown', 'heading:Paged=shown', 'note:=gone', 'page=shown'], 'a heading always shows; a note until any row of its section does');
  assert.deepEqual(states(at(100)).slice(0, 3), ['heading:Under way=shown', 'note:=gone', 'g=shown'], 'the note gives way to the first row as the clock reveals it');

  assert.equal(stableLogSections(sections, sections.map(section => ({ ...section }))), sections, 'the same rows under the same headings keep the items');
  const changed = sections.map((section, index) => index === 1 ? { ...section, rows: [row('b', at(90))] } : section);
  assert.equal(stableLogSections(sections, changed), changed);
});

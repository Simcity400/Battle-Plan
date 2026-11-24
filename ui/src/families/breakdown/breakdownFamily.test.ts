import assert from 'node:assert/strict';
import test from 'node:test';
import { engineHour } from '../../dashboard/calendar.ts';
import type { Breakdown, BreakdownPart as WirePart, PowerSpendingBreakdownReply } from '../../generated/dashboard';
import { ppSpendingProjection } from '../../widgets/political-power-spending-breakdown/ppSpendingProjection.ts';
import { createBoundedReadView, displayedHour, FutureDataError } from '../boundedView.ts';
import type { FamilyTheme, InstalledAction } from '../contract.ts';
import { BACK, parseBreakdownHit, reduceBreakdown } from './hostAdapter.ts';
import { NO_INTERACTION } from './laneView.ts';
import { layoutBreakdown, type PartsViewLayout, type RingViewLayout, type MosaicViewLayout } from './layout.ts';
import { deriveBreakdownAudit, deriveBreakdownNavAndMirror, partColour, resolveDrillLevel, type BreakdownModel } from './model.ts';
import { breakdownParts, breakdownTotal, type BreakdownWords } from './product.ts';

const THEME: FamilyTheme = {
  surface: '#16181d',
  raised: '#1e2128',
  dark: true,
  text: '#eef1f6',
  muted: '#8c95a6',
  line: '#2e3440',
  lineSoft: '#222630',
  accent: '#4f86f7',
  blue: '#4f86f7',
  red: '#e5534b',
  shortage: '#ff6b6f',
  amber: '#d29922',
  green: '#3fb950',
  grey: '#8c95a6',
  hatch: '#6e7681',
  hatchEdge: '#484f58',
  font: 'Inter',
  fontSize: { small: 11, body: 12 },
  lineWidth: 1,
  hatchSpacing: 6,
};

const START = engineHour({ year: 1936, month: 1, day: 1, hour: 1 })!;
const WORDS: BreakdownWords = { unit: 'PP', verb: 'Spent', format: raw => raw.toLocaleString('en-US') };
const measure = (text: string) => text.length * 7;

function wire(key: string, ordinal: number, raw: string | null, extra: Partial<WirePart> = {}): WirePart {
  return {
    key,
    name: key,
    ordinal,
    other: false,
    raw,
    state: raw === null ? 'notRecorded' : 'known',
    reason: raw === null ? `${key} was not recorded` : null,
    share: null,
    count: null,
    children: [],
    ...extra,
  };
}

function partlyKnown(): Breakdown {
  return {
    decimals: 0,
    total: { raw: '211', state: 'atLeast', reason: 'New ships were not recorded' },
    parts: [wire('Repairs', 2, '211'), wire('New ships', 0, null), wire('Refits', 1, null)],
    unrecordedHours: 0,
  };
}

function spent(): Breakdown {
  return {
    decimals: 0,
    total: { raw: '5972', state: 'partial', reason: 'Spending was not recorded' },
    parts: [
      wire('Daily costs', 3, '2071', { share: 0.35 }),
      wire('Decisions', 0, '1415', {
        share: 0.24,
        children: [wire('War measures', 0, '640', { share: 0.11 }), wire('Economy', 1, '352', { share: 0.059 })],
      }),
      wire('Other', 1, '63', { share: 0.01, other: true }),
    ],
    unrecordedHours: 2,
  };
}

function model(breakdown: Breakdown): BreakdownModel {
  return { hour: 10, rootLabel: 'Spent', total: breakdownTotal(breakdown, WORDS), parts: breakdownParts(breakdown, WORDS) };
}

function action(subject: string, meta: unknown = null): InstalledAction {
  return { installedId: 1, owner: 'o', subject, hour: displayedHour(10), meta } as InstalledAction;
}

test('the product decides the whole, its order and its shares; the family only reads them', () => {
  const read = model(spent());
  assert.deepEqual(read.parts.map(part => part.key), ['Daily costs', 'Decisions', 'Other'], 'drawn in the order the product gave');
  assert.equal(read.total.text, '5,972');
  assert.equal(read.total.partial, true);
  assert.equal(read.total.known, false);
  assert.equal(read.total.shortReason, '2 hours not recorded');
  assert.equal(read.parts[0]!.share, 0.35, 'a partial whole keeps its shares');
  assert.equal(read.parts[1]!.children![0]!.share, 0.11);
  assert.equal(read.parts[2]!.neutral, true);
  assert.equal(partColour(read.parts[2]!, THEME), THEME.grey, 'other is the reserved grey');
});

test('colours follow the recorded ordinal, so a reordered breakdown keeps them', () => {
  const read = model(spent());
  const reordered = model({ ...spent(), parts: [...spent().parts].reverse() });
  for (const part of read.parts) {
    const same = reordered.parts.find(item => item.key === part.key)!;
    assert.equal(partColour(same, THEME), partColour(part, THEME));
  }
});

test('a partly known whole is drawn as at least, with parts not recorded never as zero', () => {
  const read = model(partlyKnown());
  assert.equal(read.total.atLeast, true);
  assert.equal(read.total.text, '211');
  assert.equal(read.parts[1]!.notRecorded, true);
  assert.equal(read.parts[1]!.value, null);
  assert.equal(read.parts[1]!.missingReason, 'New ships was not recorded');

  const parts = layoutBreakdown({ view: 'parts', level: resolveDrillLevel(read, []), width: 1111, height: 346, theme: THEME, namesInRing: true, userExpanded: {}, ringOpenKey: undefined, measure }) as PartsViewLayout;
  assert.ok(parts.totalLine.partialTabRect, 'the at-least total carries the partial tab');
  assert.ok(parts.rows.every(row => row.shareText === ''), 'no shares beside a lower bound');

  const ring = layoutBreakdown({ view: 'ring', level: resolveDrillLevel(read, []), width: 1111, height: 346, theme: THEME, namesInRing: true, userExpanded: {}, ringOpenKey: undefined, measure }) as RingViewLayout;
  assert.equal(ring.center.verb, 'At least');
  assert.ok(ring.innerSlices.every(slice => slice.label === null), 'no names on a hatched ring');
  assert.deepEqual(deriveBreakdownAudit(read).map(row => row.value), ['atLeast:211', '211', 'notRecorded:New ships was not recorded', 'notRecorded:Refits was not recorded']);
});

test('the three views lay out the same parts', () => {
  const level = resolveDrillLevel(model(spent()), []);
  const base = { level, width: 1111, height: 346, theme: THEME, namesInRing: true, userExpanded: {}, ringOpenKey: undefined, measure };
  const parts = layoutBreakdown({ ...base, view: 'parts' }) as PartsViewLayout;
  assert.deepEqual(parts.rows.filter(row => row.depth === 0).map(row => row.key), ['Daily costs', 'Decisions', 'Other']);
  const ring = layoutBreakdown({ ...base, view: 'ring' }) as RingViewLayout;
  assert.equal(ring.openKey, 'Decisions', 'the first part with sub-parts opens in the list');
  assert.ok(ring.outerSlices.some(slice => slice.key === 'War measures'));
  const mosaic = layoutBreakdown({ ...base, view: 'mosaic' }) as MosaicViewLayout;
  assert.ok(mosaic.tiles.some(tile => tile.key === 'Decisions' && tile.isGroup));
  const narrow = layoutBreakdown({ ...base, view: 'ring', width: 320, height: 220 }) as RingViewLayout;
  assert.equal(narrow.sideList, null, 'at minimum size the ring has no list');
});

test('drill, open and expand are the family state; back returns', () => {
  const read = model(spent());
  const level = resolveDrillLevel(read, []);
  const { nav } = deriveBreakdownNavAndMirror(level, {}, 'mosaic');
  const decisions = nav.find(entry => entry[0] === 'Decisions')!;
  const drilled = reduceBreakdown(NO_INTERACTION, action('Decisions', decisions[5]))!;
  assert.deepEqual(drilled.drill, ['Decisions']);
  assert.deepEqual(resolveDrillLevel(read, drilled.drill).parts.map(part => part.key), ['War measures', 'Economy']);
  assert.deepEqual(reduceBreakdown(drilled, action(BACK))!.drill, []);

  const toggled = reduceBreakdown(NO_INTERACTION, action('Decisions', deriveBreakdownNavAndMirror(level, {}, 'parts').nav[1]![5]))!;
  assert.equal(toggled.expanded.Decisions, true);
  const ring = reduceBreakdown(NO_INTERACTION, action('Decisions', deriveBreakdownNavAndMirror(level, {}, 'ring').nav[1]![5]))!;
  assert.equal(ring.ringOpen, 'Decisions');
  assert.equal(reduceBreakdown(ring, action('Decisions', { act: 'ring', open: true }))!.ringOpen, '');
  assert.equal(reduceBreakdown(NO_INTERACTION, action('Daily costs')), null, 'a part without sub-parts is not the family\'s to handle');
  assert.deepEqual(parseBreakdownHit('toggle:Decisions'), { key: 'Decisions', subject: 'Decisions', kind: 'toggle', toggle: true, inspectToken: 'row:Decisions' });
});

function reply(breakdown: Breakdown, extra: Partial<PowerSpendingBreakdownReply> = {}): PowerSpendingBreakdownReply {
  return { from: START, hour: START + 10, through: START + 30, breakdown, gainedRaw: '5306', lostAtCapRaw: '1', gainedReason: null, warnings: [], ...extra };
}

function project(data: PowerSpendingBreakdownReply | null) {
  const readView = createBoundedReadView({ hour: START + 10, panels: new Map(data ? [['main', data]] : []), validInterval: [START + 10, START + 30] });
  return ppSpendingProjection.project(readView, { digits: 0 });
}

test('political power spending reads the product at the displayed hour', () => {
  const read = project(reply(spent()));
  assert.equal(read.periodLabel, 'Spent since 1 Jan 1936');
  assert.deepEqual(read.comparison, { label: 'Gained', value: '5,306 PP', note: '1 PP lost at the cap.' });
  assert.equal(read.total.text, '5,972');
  assert.deepEqual(read.parts.map(part => part.name), ['Daily costs', 'Decisions', 'Other']);

  const ungained = project(reply(spent(), { gainedRaw: null, gainedReason: 'Some gains were not recorded.' }));
  assert.deepEqual(ungained.comparison, { label: 'Gained', value: 'Not recorded', note: 'Some gains were not recorded.' });

  const none = project(reply({ decimals: 0, total: { raw: null, state: 'notRecorded', reason: 'Spending was not recorded' }, parts: [], unrecordedHours: 10 }));
  assert.equal(none.empty, 'Not recorded at this time.');
  assert.equal(none.emptyKind, 'unavailable');
  assert.equal(none.emptyDetail, 'Spending was not recorded');

  const zero = project(reply({ decimals: 0, total: { raw: '0', state: 'known', reason: null }, parts: [], unrecordedHours: 0 }));
  assert.equal(zero.empty, 'No spending at this date.');
  assert.equal(zero.total.text, '0', 'a known zero is 0');

  const unrecorded = project(reply({ decimals: 0, total: { raw: '0', state: 'partial', reason: 'Spending was not recorded' }, parts: [], unrecordedHours: 5 }));
  assert.equal(unrecorded.empty, 'No spending recorded at this date.', 'unrecorded hours never read as no spending');
  assert.equal(unrecorded.emptyDetail, 'Spending was not recorded');
});

test('nothing after the displayed hour reaches the projection', () => {
  const readView = createBoundedReadView({ hour: 10, panels: new Map([['main', reply(spent())]]) });
  assert.throws(() => readView.hover('main', 11), FutureDataError);
});

import assert from 'node:assert/strict';
import test from 'node:test';
import { assertBarsModelValid, formatBarsValueField, type BarsDataRow } from '../../families/bars/model.ts';
import { createBoundedReadView, displayedHour } from '../../families/boundedView.ts';
import type { IdentityProvider } from '../../families/plan.ts';
import type { SeriesStateReply } from '../../generated/dashboard';
import { casualtiesButterflyDeclaration, casualtiesButterflyProjection, casualtiesRaceProjection, type CasualtiesOptions } from './casualtiesProjection.ts';

const identity: IdentityProvider = {
  resolve(kind, key, atHour) {
    return {
      kind, key, atHour,
      holder: { status: 'ready', value: key },
      tag: { status: 'ready', value: key },
      name: { status: 'ready', value: `${key} name` },
      colour: { status: 'ready', value: '#c0392b' },
      flagChoice: { status: 'absent', reason: 'Flag not recorded' },
      validInterval: [0, 1000],
      nextChangeAt: Infinity,
    };
  },
};

const run = (value: number) => ({ firstHour: 400, lastHour: 600, valueRaw: String(value), slot: 1, countryId: 1, startsSegment: false, segmentFirstHour: 0 });

const reply = (measure: string, values: readonly (readonly [string, number | null])[]): SeriesStateReply => ({
  branchId: '1', measure, decimals: 0,
  states: values.map(([country, value]) => ({ country, hour: 500, run: value === null ? null : run(value) as never, drop: null, missing: value === null ? 'Casualties were not read' : null })),
});

const panels = (patch: Record<string, SeriesStateReply> = {}) => new Map<string, unknown>(Object.entries({
  rank: reply('dealtAndTaken', [['SOV', 900], ['GER', 700], ['ITA', null]]),
  dealt: reply('dealt', [['SOV', 300], ['GER', 500], ['ITA', null]]),
  taken: reply('taken', [['SOV', 600], ['GER', 200], ['ITA', null]]),
  unknown: reply('unknown', [['SOV', 40], ['GER', 0], ['ITA', null]]),
  partial: reply('partial', [['SOV', 0], ['GER', 3], ['ITA', null]]),
  ...patch,
}));

const view = (map = panels()) => createBoundedReadView({ hour: displayedHour(500), panels: map, identity });
const BUTTERFLY: CasualtiesOptions = { view: 'butterfly', includeUnknown: false, chosen: null, membership: null };

test('the butterfly keeps the product rank, puts casualties taken left and dealt right, and leaves the unrecorded last', () => {
  const model = casualtiesButterflyProjection.project(view(), BUTTERFLY);
  assertBarsModelValid(model, 500);
  assert.equal(model.leftHeader, 'Casualties taken');
  assert.equal(model.rightHeader, 'Casualties dealt');
  const rows = model.rows as BarsDataRow[];
  assert.deepEqual(rows.map(row => row.key), ['SOV', 'GER', 'ITA']);
  assert.deepEqual([formatBarsValueField(rows[0]!.left!.value).text, formatBarsValueField(rows[0]!.right!.value).text], ['600', '300']);
  assert.equal(rows[1]!.left?.value.value.kind, 'partial', 'casualties recorded without full detail are at least the figure');
  assert.equal(rows[2]!.left?.value.value.kind, 'notRecorded');
  assert.equal(rows[2]!.left?.numeric, null);
});

test('including unknown attackers adds them to casualties taken; an unread unknown makes taken not recorded', () => {
  const rows = casualtiesButterflyProjection.project(view(), { ...BUTTERFLY, includeUnknown: true }).rows as BarsDataRow[];
  assert.equal(formatBarsValueField(rows[0]!.left!.value).text, '640');
  const unreadUnknown = casualtiesButterflyProjection.project(view(panels({ unknown: reply('unknown', [['SOV', null], ['GER', 0]]) })), { ...BUTTERFLY, includeUnknown: true }).rows as BarsDataRow[];
  assert.equal(unreadUnknown[0]!.left?.value.value.kind, 'notRecorded');
});

test('the race view is ordered by the product\'s ranking of casualties taken', () => {
  const model = casualtiesRaceProjection.project(view(panels({ rank: reply('taken', [['SOV', 600], ['GER', 200]]) })), { ...BUTTERFLY, view: 'race' });
  assert.equal(model.form, 'ordered');
  assert.deepEqual((model.rows as BarsDataRow[]).map(row => [row.key, row.numeric]), [['SOV', 600], ['GER', 200]]);
});

test('a chosen country the product does not answer is drawn last as not recorded', () => {
  const rows = casualtiesButterflyProjection.project(view(), { ...BUTTERFLY, chosen: ['GER', 'FIN'] }).rows as BarsDataRow[];
  assert.equal(rows[rows.length - 1]!.key, 'FIN');
  assert.equal(rows[rows.length - 1]!.left?.value.value.kind, 'notRecorded');
});

type Ctx = Parameters<typeof casualtiesButterflyDeclaration.sources>[0];
const ctxOf = (countries: Record<string, unknown>): Ctx => ({
  track: { source: { id: 's', revision: 1 }, axisOffset: 0 },
  branch: { id: '1' },
  options: {},
  countries: { all: false, top: null, every: [], tags: [], membership: null, ...countries },
}) as unknown as Ctx;

function askedAt(ctx: Ctx, name: string, hour: number): Readonly<Record<string, unknown>> | null {
  const read = casualtiesButterflyDeclaration.sources(ctx, { following: false, staticSpan: null }).entries.find(entry => entry.name === name)!.read;
  assert.equal(read.shape, 'point');
  return read.shape === 'point' ? read.buildExactParams!(read.params!, hour as never, 'static') : null;
}

test('under the default Top selection the detail measures are read for every country, and an unread detail panel is loading, not missing', () => {
  assert.equal(casualtiesButterflyDeclaration.countries?.mode === 'multi' && casualtiesButterflyDeclaration.countries.everyOnTop, true,
    'the host lists every country when Top is chosen');
  const listed = ctxOf({ top: 10, every: ['GER', 'SOV'] });
  assert.deepEqual((askedAt(listed, 'dealt', 500) as { reads: unknown }).reads, [{ country: 'GER', hour: 500 }, { country: 'SOV', hour: 500 }]);
  assert.equal(askedAt(ctxOf({ top: 10 }), 'dealt', 500), null, 'nothing is asked with an empty country list');
  const rank = casualtiesButterflyDeclaration.sources(listed, { following: false, staticSpan: null }).entries.find(entry => entry.name === 'rank')!.read;
  assert.equal(rank.shape === 'point' && rank.provenInterval, undefined, 'a Top answer holds for its own hour until the product proves more');
  const rankOnly = view(new Map([['rank', reply('dealtAndTaken', [['SOV', 900], ['GER', 700]])]]));
  const model = casualtiesButterflyProjection.project(rankOnly, casualtiesButterflyDeclaration.projectionOptions(ctxOf({ top: 10 })));
  assert.deepEqual(model.rows, [], 'recorded casualties are not drawn as missing while their details load');
  assert.equal(model.empty, undefined);
});

test('a faction member that joins after the displayed hour is neither asked for nor drawn', () => {
  const ctx = ctxOf({ tags: ['GER', 'ITA'], membership: { GER: [[0, 900]], ITA: [[600, 900]] } });
  assert.deepEqual((askedAt(ctx, 'rank', 500) as { reads: unknown }).reads, [{ country: 'GER', hour: 500 }]);
  assert.deepEqual((askedAt(ctx, 'taken', 700) as { reads: unknown }).reads, [{ country: 'GER', hour: 700 }, { country: 'ITA', hour: 700 }]);
  const before = ctxOf({ tags: ['GER', 'FIN'], membership: { GER: [[0, 900]], FIN: [[600, 900]] } });
  const shown = (casualtiesButterflyProjection.project(view(), casualtiesButterflyDeclaration.projectionOptions(before)).rows as BarsDataRow[]).map(row => row.key);
  assert.equal(shown.includes('FIN'), false, 'a member from tomorrow is not drawn today');
  assert.equal(casualtiesButterflyProjection.nextChange?.(view(), casualtiesButterflyDeclaration.projectionOptions(before)), 600);
});

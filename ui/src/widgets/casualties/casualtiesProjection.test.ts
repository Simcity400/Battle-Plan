import assert from 'node:assert/strict';
import test from 'node:test';
import { assertBarsModelValid, formatBarsValueField, type BarsDataRow } from '../../families/bars/model.ts';
import { createBoundedReadView, displayedHour } from '../../families/boundedView.ts';
import type { IdentityProvider } from '../../families/plan.ts';
import type { SeriesRankReply } from '../../generated/dashboard';
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

const run = (value: number) => ({ firstHour: 400, lastHour: 600, valueRaw: String(value), slot: 1, countryId: 1, startsSegment: false, segmentFirstHour: 0, firstExact: true, lastExact: true });

type Row = readonly [string, ...(number | null)[]];

const reply = (measures: readonly string[], rows: readonly Row[], refused: string | null = null): SeriesRankReply => ({
  branchId: '1', hour: 500, from: 500, until: 500, refused,
  measures: measures.map(name => ({ name, decimals: 0 })),
  rows: rows.map(([country, ...values]) => ({
    country,
    readings: values.map(value => ({ country, hour: 500, run: value === null ? null : run(value), drop: null, missing: value === null ? 'Casualties were not read' : null })),
  })),
});

const BUTTERFLY_MEASURES = ['dealtAndTaken', 'dealt', 'taken', 'unknown', 'partial'];
const butterflyReply = (unknownSov: number | null = 40) => reply(BUTTERFLY_MEASURES, [['SOV', 900, 300, 600, unknownSov, 0], ['GER', 700, 500, 200, 0, 3], ['ITA', null, null, null, null, null]]);

const view = (panel: SeriesRankReply = butterflyReply()) => createBoundedReadView({ hour: displayedHour(500), panels: new Map<string, unknown>([['rank', panel]]), identity });
const BUTTERFLY: CasualtiesOptions = { view: 'butterfly', includeUnknown: false, group: false };

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
  const unreadUnknown = casualtiesButterflyProjection.project(view(butterflyReply(null)), { ...BUTTERFLY, includeUnknown: true }).rows as BarsDataRow[];
  assert.equal(unreadUnknown[0]!.left?.value.value.kind, 'notRecorded');
});

test('the race view is ordered by the product ranking of casualties taken', () => {
  const model = casualtiesRaceProjection.project(view(reply(['taken', 'unknown', 'partial'], [['SOV', 600, 0, 0], ['GER', 200, 0, 0]])), { ...BUTTERFLY, view: 'race' });
  assert.equal(model.form, 'ordered');
  assert.deepEqual((model.rows as BarsDataRow[]).map(row => [row.key, row.numeric]), [['SOV', 600], ['GER', 200]]);
});

test('a top the product refuses says why, never that there were no casualties', () => {
  const model = casualtiesButterflyProjection.project(view(reply(BUTTERFLY_MEASURES, [], 'Casualty totals not recorded at this hour')), BUTTERFLY);
  assert.deepEqual([model.rows.length, model.empty, model.emptyKind], [0, 'Casualty totals not recorded at this hour', 'unavailable']);
});

type Ctx = Parameters<typeof casualtiesButterflyDeclaration.sources>[0];
const ctxOf = (countries: Record<string, unknown>): Ctx => ({
  track: { source: { id: 's', revision: 1 }, axisOffset: 0 },
  branch: { id: '1' },
  options: {},
  countries: { all: false, top: null, every: [], tags: [], membership: null, ...countries },
  widget: { countries: { kind: 'tags', tags: [] } },
}) as unknown as Ctx;

function askedAt(ctx: Ctx, name: string, hour: number): Readonly<Record<string, unknown>> | null {
  const read = casualtiesButterflyDeclaration.sources(ctx, { following: false, staticSpan: null }).entries.find(entry => entry.name === name)!.read;
  assert.equal(read.shape, 'point');
  return read.shape === 'point' ? read.buildExactParams!(read.params!, hour as never, 'static') : null;
}

test('under the default Top selection one rank reply carries every displayed measure for the ranked countries, valid only where the product proves it', () => {
  const asked = askedAt(ctxOf({ top: 10 }), 'rank', 500) as { top?: number; measures?: string[]; countries?: unknown };
  assert.deepEqual([asked.top, asked.measures, asked.countries], [10, ['dealt', 'taken', 'unknown', 'partial'], undefined]);
  const rank = casualtiesButterflyDeclaration.sources(ctxOf({ top: 10 }), { following: false, staticSpan: null }).entries.find(entry => entry.name === 'rank')!.read;
  assert.deepEqual(rank.shape === 'point' ? rank.provenInterval?.({ ...butterflyReply(), from: 500, until: 500 }, 500) : null, [500, 500],
    'a Top answer holds for its own hour until the product proves more');
  const model = casualtiesButterflyProjection.project(view(), casualtiesButterflyDeclaration.projectionOptions(ctxOf({ top: 10 })));
  assert.deepEqual((model.rows as BarsDataRow[]).map(row => formatBarsValueField(row.right!.value).text).slice(0, 2), ['300', '500'], 'recorded casualties are drawn, not missing');
});

test('a faction is sent as the group, resolved at the hour in Rust, and a known empty group says so', () => {
  const faction = { kind: 'group', group: { type: 'faction', id: '42' }, membership: 'hour', aggregate: false };
  const ctx = { ...ctxOf({ tags: ['GER', 'ITA'] }), widget: { countries: faction } } as unknown as Ctx;
  const asked = askedAt(ctx, 'rank', 500) as { group?: unknown; countries?: unknown; top?: unknown };
  assert.deepEqual([asked.group, asked.countries, asked.top], [{ type: 'faction', id: '42', membership: 'hour' }, undefined, undefined],
    'the group, never the union of its members over time');
  const empty = casualtiesButterflyProjection.project(view(reply(BUTTERFLY_MEASURES, [])), casualtiesButterflyDeclaration.projectionOptions(ctx));
  assert.deepEqual([empty.empty, empty.emptyKind], ['No country of this group at this date.', 'empty']);
});

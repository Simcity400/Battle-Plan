import assert from 'node:assert/strict';
import test from 'node:test';
import { assertBarsModelValid, formatBarsValueField, type BarsDataRow, type BarsModel } from '../../families/bars/model.ts';
import type { BarsWidgetDeclaration } from '../../families/bars/declaration.ts';
import { createBoundedReadView, displayedHour } from '../../families/boundedView.ts';
import type { FamilyPlanContext, FamilyViewportRequest, PointSourceRead } from '../../families/plan.ts';
import { formatValueState } from '../../families/valueFormat.ts';
import { PENDING_ART, readyArt } from '../../families/art/types.ts';
import type { ResourceBalanceReply } from '../../generated/dashboard';
import { FAMILY_VIEWS } from '../familyManifest.ts';
import { validateResourceBalance } from './resourceReply.ts';

const ICON = 'http://artwork.localhost/0123456789abcdef0123456789abcdef.png';

const reply = (over: Partial<ResourceBalanceReply> = {}): ResourceBalanceReply => ({
  tag: 'ITA', decimals: 5, warnings: [], poolsMissing: null, dealsMissing: null, deals: [],
  resources: [
    { id: 1, token: 'oil', ordinal: 1, name: 'Oil', icon: ICON },
    { id: 2, token: 'steel', ordinal: 1, name: 'Steel', icon: null },
    { id: 3, token: 'coal', ordinal: 1, name: null, icon: null },
    { id: 4, token: 'chromium', ordinal: 1, name: 'Chromium', icon: null },
  ],
  pools: [1, 2, 3],
  rows: [
    { resource: 1, net: '-1300000', supply: '1100000', demand: '2400000', missing: null },
    { resource: 3, net: '400000', supply: '2000000', demand: '1600000', missing: null },
    { resource: 2, net: '0', supply: '500000', demand: '500000', missing: null },
    { resource: 4, net: null, supply: null, demand: null, missing: 'No row of this resource was recorded for this country at this hour' },
  ],
  short: 1,
  unrecorded: 1,
  ...over,
});

async function view() {
  const entry = FAMILY_VIEWS.find(item => item.kind === 'resource-balance' && item.view === 'bars');
  assert.equal(entry?.family, 'bars');
  assert.equal(entry?.viewOption, 'view');
  assert.deepEqual(entry?.options?.map(field => field.key), ['view', 'hideEmpty']);
  const spec = await entry!.loadSpec();
  return { projection: spec.projection as BarsWidgetDeclaration<{ country: string | null }>['projection'], declaration: spec.declaration as BarsWidgetDeclaration<{ country: string | null }> };
}

const project = async (data: ResourceBalanceReply): Promise<BarsModel> => {
  const { projection } = await view();
  const model = projection.project(createBoundedReadView({ hour: displayedHour(500), panels: new Map([['main', data]]), art: { demandOwner: 'view', resolve: handle => (handle.kind === 'url' ? readyArt(handle.url) : PENDING_ART) } }), { country: 'ITA' });
  assertBarsModelValid(model, 500);
  return model;
};

const text = (row: BarsDataRow) => formatBarsValueField(row.value!).text;

test('the Bars view keeps the product order and prints the exact signed net, In and Out of each resource', async () => {
  const model = await project(reply());
  assert.equal(model.form, 'net');
  const rows = model.rows as BarsDataRow[];
  assert.deepEqual(rows.map(row => row.key), ['1', '3', '2', '4'], 'no client reordering');
  assert.deepEqual(rows.map(text), ['−13', '+4', '0', 'Not recorded']);
  assert.deepEqual(rows.map(row => row.numeric), [-13, 4, 0, null], 'a known zero draws no bar; not recorded draws none either');
  assert.equal(rows[0]!.value?.tone, 'shortage');
  assert.equal(rows[2]!.value?.value.kind, 'zero');
  assert.equal(rows[3]!.value?.value.kind, 'notRecorded');
  assert.match(formatBarsValueField(rows[3]!.value!).missingReason ?? '', /No row of this resource/);
  const pair = rows[0]!.pair!;
  assert.deepEqual([pair.topLabel, pair.bottomLabel], ['In', 'Out']);
  assert.deepEqual([formatValueState(pair.topValue, pair.format), formatValueState(pair.bottomValue, pair.format)].map(item => 'text' in item ? item.text : item.label), ['11', '24']);
  assert.equal(rows[3]!.pair?.topValue.kind, 'notRecorded');
  assert.equal(model.summaryText, '1 short · 1 not recorded', 'a resource not recorded is never counted short and never makes "No shortages"');
});

test('the resource icon comes from the reply; a missing name or icon is said, never guessed', async () => {
  const rows = (await project(reply())).rows as BarsDataRow[];
  assert.equal(rows[0]!.media?.kind, 'art');
  const icon = rows[0]!.media?.art;
  assert.equal(icon?.status === 'ready' ? icon.url : null, ICON);
  assert.equal(rows[2]!.media?.art.status, 'unavailable');
  assert.equal(typeof rows[1]!.label !== 'string' && rows[1]!.label.kind, 'notRecorded');
  assert.notEqual(rows[0]!.color, rows[1]!.color, 'colours follow the recorded resource order');
});

test('no shortage says so; unrecorded resources are the body state with their reason, never a list of zeros', async () => {
  const none = await project(reply({ rows: reply().rows.slice(1, 3), short: 0, unrecorded: 0 }));
  assert.equal(none.summaryText, 'No shortages');
  const unread = await project(reply({ pools: [], rows: [], short: 0, unrecorded: 0, poolsMissing: 'Resources are not recorded at this hour: unreadable' }));
  assert.equal(unread.rows.length, 0);
  assert.equal(unread.emptyKind, 'unavailable');
  assert.match(unread.empty ?? '', /not recorded at this hour/);
});

test('the Bars view asks the product for the chosen country at the hour, hiding unused resources only when asked', async () => {
  const { declaration } = await view();
  const ctx = (options: Record<string, unknown>) => ({
    track: { source: { id: 's1', revision: 'r1' }, axisOffset: 0 },
    branch: { id: '1', firstHour: 100, lastHour: 900 },
    widget: { countries: { kind: 'tags', tags: ['ITA'] } },
    options,
  }) as unknown as FamilyPlanContext;
  const read = (options: Record<string, unknown>) => declaration.sources(ctx(options), {} as FamilyViewportRequest).entries[0]!.read as PointSourceRead;
  assert.deepEqual(read({}).params, { branchId: '1', country: 'ITA' });
  assert.deepEqual(read({ hideEmpty: true }).params, { branchId: '1', country: 'ITA', hideEmpty: true });
  assert.deepEqual(read({}).followQuery, { panel: 'resource-balance', branchId: '1', country: 'ITA' });
});

test('a malformed reply is refused', () => {
  assert.equal(validateResourceBalance(reply()).rows.length, 4);
  assert.throws(() => validateResourceBalance({ ...reply(), rows: [{ resource: 9, net: '1', supply: '1', demand: '0', missing: null }] }), /malformed/);
  assert.throws(() => validateResourceBalance({ ...reply(), rows: [{ resource: 1, net: null, supply: '1', demand: '0', missing: null }] }), /malformed/, 'a figure without its reason');
  assert.throws(() => validateResourceBalance({ ...reply(), pools: [9] }), /malformed/);
  assert.throws(() => validateResourceBalance({ ...reply(), resources: [...reply().resources].reverse() }), /malformed/);
});

test('resource colours follow recorded catalogue ids rather than filtered array positions', async () => {
  const full = (await project(reply())).rows as BarsDataRow[];
  const steelFull = full.find(r => r.key === '2')!.color;
  const chromiumFull = full.find(r => r.key === '4')!.color;
  const filtered = (await project(reply({
    resources: reply().resources.filter(info => info.id === 2 || info.id === 4),
    pools: [2],
    rows: reply().rows.filter(r => r.resource === 2 || r.resource === 4),
    short: 0,
    unrecorded: 1,
  }))).rows as BarsDataRow[];
  assert.equal(filtered.find(r => r.key === '2')!.color, steelFull);
  assert.equal(filtered.find(r => r.key === '4')!.color, chromiumFull);
});

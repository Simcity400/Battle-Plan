import assert from 'node:assert/strict';
import test from 'node:test';
import { createBoundedReadView } from '../boundedView.ts';
import type { CountryFlowReply, FlowEndpoint, FlowLink, FlowSide } from '../../generated/dashboard';
import { projectCountryFlow, type CountryFlowText } from './countryFlow.ts';
import { deriveFlowAudit, deriveFlowNav, findFlowTooltip } from './model.ts';
import { hitFlow, layoutFlow } from './painter.ts';
import { assertFreshPaint, assertQuietFrames, assertTooltipDated, LANE_THEME } from '../laneRuntime.fixture.ts';
import { registerFamilyProjection } from '../registry.ts';
import { flowLane } from './laneView.ts';
import { projectTradeMatrix, projectTradeTable } from '../../widgets/trade-flows/tradeFlowsProjection.ts';
import { createLaneView } from '../laneRuntime.ts';
import { flowPerSide } from './countryFlowPlan.ts';

const HOUR = 60_810_000;
const TEXT: CountryFlowText = {
  leftHeader: 'Imports from',
  rightHeader: 'Exports to',
  unitNote: 'per day',
  groupLeftHeader: 'Exporters',
  groupRightHeader: 'Importers',
  notRecorded: 'Trade was not recorded at this date.',
  none: 'No trade at this date.',
  others: count => `${count} other partners`,
  ribbonTitle: (from, to) => `${from} → ${to}`,
};
const parts = [
  { key: 'oil', name: 'Oil', nameMissing: null, ordinal: 0, icon: null },
  { key: 'steel', name: 'Steel', nameMissing: null, ordinal: 1, icon: null },
];
const side = (links: FlowLink[], total: string | null, missing: string | null = null, unranked: string | null = null): FlowSide => ({ links, total, missing, unranked });
const NONE = side([], '0');
const base: CountryFlowReply = {
  endpoints: [], subject: null, decimals: 5, parts, incoming: NONE, outgoing: NONE,
  links: [], senders: NONE, receivers: NONE, table: [], largest: '0', largestLink: '0', missing: null, warnings: [],
};
const amount = (value: number) => String(value * 100_000);
const endpoint = (slot: number | null, tag: string | null, unknown: string | null = null, raw: number | null = null): FlowEndpoint =>
  ({ key: slot !== null ? `s${slot}` : raw !== null ? `r${raw}` : 'collective', slot, tag, collective: slot === null && raw === null, unknown });
const link = (from: number | null, to: number | null, parts: [number, number | null][], others: number | null = null): FlowLink => {
  const known = parts.every(([, value]) => value !== null);
  return {
    from, to, others,
    amounts: parts.map(([part, value]) => ({ part, amount: value === null ? null : amount(value) })),
    total: known ? amount(parts.reduce((sum, [, value]) => sum + (value ?? 0), 0)) : null,
    missing: known ? null : 'A deal\'s daily delivery was not recorded at this hour',
  };
};
const view = (reply: unknown) => createBoundedReadView({ hour: HOUR, panels: new Map([['main', reply]]) });

const USA = 0;
const HOL = 1;
const RAJ = 2;
const ENG = 3;
const centred: CountryFlowReply = {
  ...base,
  endpoints: [endpoint(1, 'USA'), endpoint(2, 'HOL'), endpoint(3, 'RAJ'), endpoint(6, 'ENG')],
  subject: USA,
  incoming: side([
    link(HOL, USA, [[0, 20], [1, 6]]),
    link(null, USA, [[0, 14]], 14),
    link(RAJ, USA, [[1, null]]),
  ], null, 'A deal\'s daily delivery was not recorded at this hour'),
  outgoing: side([link(USA, ENG, [[0, 74]])], amount(74)),
  largest: amount(74),
  largestLink: amount(74),
};

test('a country-centred flow: partners in the product\'s order, an unrecorded partner hatched with its reason, the rest folded, never a zero', () => {
  const model = projectCountryFlow(view(centred), TEXT);
  assert.equal(model.mode, 'centred');
  assert.deepEqual(model.left.map(node => [node.key, node.numeric, node.figure]), [
    ['left:s2', 26, '26'],
    ['left:others', 14, '14'],
    ['left:s3', null, 'Not recorded'],
  ]);
  assert.equal(model.left[1]!.label, '14 other partners');
  assert.equal(model.right[0]!.numeric, 74);
  assert.equal(model.notRecordedKey, true, 'a hatched partner adds the not-recorded key');
  const ribbon = model.ribbons.find(item => item.from === 'left:s2')!;
  assert.deepEqual(ribbon.bands, [{ part: 0, numeric: 20 }, { part: 1, numeric: 6 }]);
  assert.equal(findFlowTooltip(model, 'ribbon:left:s3>centre:s1')?.missingReason, 'A deal\'s daily delivery was not recorded at this hour');
  assert.ok(deriveFlowAudit(model).some(row => row.key === 'node:left:s3' && (row.value ?? '').startsWith('Not recorded · Steel: Not recorded (')));
  assert.equal(model.left[0]!.audit, '26 · Oil: 20; Steel: 6', 'the audit carries each part, so swapped bands differ');
  assert.ok(deriveFlowNav(model).some(tuple => tuple[0] === 'ribbon:left:s2>centre:s1'), 'a ribbon takes keyboard focus');
  assert.deepEqual(model.notes, ['A deal\'s daily delivery was not recorded at this hour'], 'the side whose total is unproven says why');
  assert.match(model.centre!.audit, /Imports from Not recorded: .*Exports to 74/);
});

test('a partner on both sides has a node on each side: the incoming ribbon starts on the left and each node reads its own figures', () => {
  const reply: CountryFlowReply = {
    ...base,
    endpoints: [endpoint(1, 'USA'), endpoint(6, 'ENG')],
    subject: 0,
    incoming: side([link(1, 0, [[0, 5]])], amount(5)),
    outgoing: side([link(0, 1, [[0, 45]])], amount(45)),
    largest: amount(45),
    largestLink: amount(45),
  };
  const model = projectCountryFlow(view(reply), TEXT);
  assert.deepEqual([model.left[0]!.key, model.right[0]!.key], ['left:s6', 'right:s6']);
  const layout = layoutFlow(model, 846, 360);
  const left = layout.nodes.find(box => box.node.key === 'left:s6')!;
  const right = layout.nodes.find(box => box.node.key === 'right:s6')!;
  assert.ok(left.x < right.x, 'the two boxes are apart');
  const incoming = layout.ribbons.find(box => box.ribbon.to === 'centre:s1')!;
  assert.equal(incoming.x0, left.x + 6, 'the incoming ribbon starts at the left box');
  assert.equal(hitFlow(layout, right.x + 20, right.y + right.height / 2), 'node:right:s6');
  assert.equal(findFlowTooltip(model, 'node:right:s6')?.items?.[0]?.value, '45');
  assert.equal(findFlowTooltip(model, 'node:left:s6')?.items?.[0]?.value, '5');
  assert.equal(new Set(deriveFlowAudit(model).map(row => row.key)).size, deriveFlowAudit(model).length, 'audit keys are unique');
});

test('licences from distinct unlinked givers stay distinct partners, a combined amount says it is combined, and every identity reason reaches hover and audit', () => {
  const reason = 'The licence names a country the recording cannot link at this hour';
  const combined = 'The deal names a country the recording cannot link at this hour';
  const unnamed = 'Not a country at this hour';
  const one = (from: number) => ({ from, to: 0, others: null, amounts: [{ part: 0, amount: '1' }], total: '1', missing: null });
  const reply: CountryFlowReply = {
    ...base,
    decimals: 0,
    parts: [{ key: 'licences', name: 'Licences', nameMissing: null, ordinal: 0, icon: null }],
    endpoints: [endpoint(7, 'GER'), endpoint(8, 'ITA'), endpoint(null, null, reason, 77), endpoint(null, null, reason, 78), endpoint(null, null, combined), endpoint(9, null, unnamed)],
    subject: 0,
    incoming: side([one(1), one(2), one(3), one(4), one(5)], '5'),
    largest: '5',
    largestLink: '1',
  };
  const model = projectCountryFlow(view(reply), TEXT);
  assert.deepEqual(model.left.map(node => [node.key, node.label]), [
    ['left:s8', 'Loading name…'], ['left:r77', 'Unknown country'], ['left:r78', 'Unknown country'], ['left:collective', 'Unknown countries, combined'], ['left:s9', 'Loading name…'],
  ], '"Unknown country" is a label, not a key');
  assert.equal(model.left[1]!.unknown, reason);
  assert.equal(findFlowTooltip(model, 'node:left:r77')?.missingReason, reason);
  assert.equal(model.left[1]!.subject, undefined, 'an unknown partner is no country to open');
  assert.equal(model.left[4]!.unknown, unnamed, 'a recorded slot that cannot be named keeps its reason');
  const ribbon = model.ribbons.find(item => item.from === 'left:collective')!;
  assert.ok(ribbon.tooltip?.items?.some(item => item.label === 'Unknown countries, combined' && item.missingReason === combined), 'the ribbon says why its partner is unknown');
  assert.ok(ribbon.audit.endsWith(`Unknown countries, combined: ${combined}`));
});

test('a side whose order is not proven says the asked Top N is not applied', () => {
  const model = projectCountryFlow(view({ ...centred, incoming: { ...centred.incoming, unranked: 'not recorded' } }), TEXT);
  assert.ok(model.notes?.includes('Top countries not applied, every partner shown: not recorded'));
});

test('a partial reply never says there is none: emptiness needs both sides proven', () => {
  const unproven = 'The deal names a country the recording cannot link at this hour';
  const partial = projectCountryFlow(view({ ...base, endpoints: [endpoint(1, 'USA')], subject: 0, incoming: side([], null, unproven) }), TEXT);
  assert.equal(partial.emptyKind, 'unavailable');
  assert.notEqual(partial.empty, TEXT.none);
  assert.equal(partial.emptyDetail, unproven);
  const group = projectCountryFlow(view({ ...base, senders: side([], null, unproven), receivers: side([], null, unproven) }), TEXT);
  assert.equal(group.emptyKind, 'unavailable');
  const table = projectTradeTable(view({ ...base, endpoints: [endpoint(1, 'USA')], subject: 0, incoming: side([], null, unproven) }));
  assert.equal(table.emptyKind, 'unavailable');
  const partialTable = projectTradeTable(view({ ...centred, incoming: side(centred.incoming.links, null, unproven), table: [{ partner: 1, cells: ['8', '0'], net: '8' }] }));
  assert.equal(partialTable.titleSummaryPartial, unproven, 'trade table carries partial side reason in the header summary marker');
  assert.equal('notes' in partialTable, false);
  const matrix = projectTradeMatrix(view({ ...base, senders: side([], null, unproven), receivers: side([], null, unproven) }));
  assert.equal(matrix.emptyKind, 'unavailable');
  const none = projectCountryFlow(view({ ...base, endpoints: [endpoint(1, 'USA')], subject: 0 }), TEXT);
  assert.equal(none.empty, TEXT.none);
  assert.equal(none.emptyKind, undefined);
  assert.equal(projectCountryFlow(view({ ...base, missing: 'Trade flows were not read at this hour' }), TEXT).emptyKind, 'unavailable');
});

test('the layout gives every ribbon its width at both ends, keeps labels apart, and hit-tests what it draws', () => {
  const model = projectCountryFlow(view(centred), TEXT);
  const layout = layoutFlow(model, 846, 360);
  const hol = layout.nodes.find(box => box.node.key === 'left:s2')!;
  const holRibbon = layout.ribbons.find(box => box.ribbon.from === 'left:s2')!;
  assert.ok(Math.abs((holRibbon.a1 - holRibbon.a0) - (holRibbon.b1 - holRibbon.b0)) < 0.01, 'a ribbon keeps its width');
  assert.ok(Math.abs(hol.height - 26 * layout.scale) < 0.01);
  const lefts = layout.nodes.filter(box => box.node.side === 'left');
  for (let i = 1; i < lefts.length; i++) assert.ok(lefts[i]!.slotTop >= lefts[i - 1]!.slotTop + lefts[i - 1]!.slotHeight, 'label slots never overlap');
  const middleX = (holRibbon.x0 + holRibbon.x1) / 2;
  const middleY = (holRibbon.a0 + holRibbon.b0 + holRibbon.a1 + holRibbon.b1) / 4;
  assert.equal(hitFlow(layout, middleX, middleY), 'ribbon:left:s2>centre:s1');
  assert.equal(hitFlow(layout, hol.x - 20, hol.y + hol.height / 2), 'node:left:s2');
  assert.ok(layout.bodyTop + 26 * layout.scale <= 360);

  registerFamilyProjection({ kind: 'test-flow', family: 'flow', project: readView => projectCountryFlow(readView, TEXT) });
  const lane = (reply: CountryFlowReply) => ({ family: 'flow', kind: 'test-flow', options: {}, sources: {}, staticPanels: { main: reply }, scrollTop: 0, theme: LANE_THEME, width: 846, height: 360 });
  const held = lane(centred);
  const corrected: CountryFlowReply = { ...centred, outgoing: side([link(USA, ENG, [[0, 70]])], amount(70)) };
  const surface = { width: 846, height: 360, dpr: 1 };
  assertQuietFrames(() => createLaneView(flowLane, () => {}), { label: 'flow', binding: held, hour: HOUR }, 24, surface);
  assertFreshPaint(() => createLaneView(flowLane, () => {}), [
    { label: 'held', binding: held, hour: HOUR, check: true },
    { label: 'later', binding: held, hour: HOUR + 30, speed: 24 },
    { label: 'seek back', binding: held, hour: HOUR, check: true },
    { label: 'correction', binding: lane(corrected), hour: HOUR, check: true },
  ], surface);
  assertTooltipDated(() => createLaneView(flowLane, () => {}), { label: 'flow', binding: held, hour: HOUR }, 'node:left:s2', 5, surface);
});

test('a group subject is a bipartite flow of the product\'s senders and receivers, a country on both sides drawn on both', () => {
  const ITA = 0;
  const SOV = 1;
  const SPD = 2;
  const SPR = 3;
  const reply: CountryFlowReply = {
    ...base,
    endpoints: [endpoint(10, 'ITA'), endpoint(8, 'SOV'), endpoint(11, 'SPD'), endpoint(9, 'SPR')],
    senders: side([link(ITA, null, [[0, 9]]), link(SOV, null, [[0, 4]]), link(SPD, null, [[0, 1]])], amount(14)),
    receivers: side([link(null, SPD, [[0, 9]]), link(null, SPR, [[0, 4]]), link(null, ITA, [[0, 1]])], amount(14)),
    links: [link(ITA, SPD, [[0, 9]]), link(SOV, SPR, [[0, 4]]), link(SPD, ITA, [[0, 1]])],
    largest: amount(9),
    largestLink: amount(9),
  };
  const model = projectCountryFlow(view(reply), TEXT);
  assert.equal(model.mode, 'bipartite');
  assert.deepEqual(model.left.map(node => node.key), ['left:s10', 'left:s8', 'left:s11']);
  assert.deepEqual(model.right.map(node => node.key), ['right:s11', 'right:s9', 'right:s10']);
  assert.deepEqual(model.ribbons.map(ribbon => ribbon.key), ['left:s10>right:s11', 'left:s8>right:s9', 'left:s11>right:s10']);
  assert.equal(model.leftHeader, 'Exporters');
  const layout = layoutFlow(model, 846, 360);
  const back = layout.ribbons.find(box => box.ribbon.key === 'left:s11>right:s10')!;
  assert.ok(back.x0 < back.x1, 'a link back to a sender runs left to right, to its receiver box');
});

test('the trade matrix keys a partner by its recorded slot and an unproven population leaves absent cells unknown, never "No trade"', () => {
  const unproven = 'The deal names a country the recording cannot link at this hour';
  const reply: CountryFlowReply = {
    ...base,
    endpoints: [endpoint(1, 'USA'), endpoint(6, 'ENG'), endpoint(null, null, unproven)],
    senders: side([link(0, null, [[0, 3]]), link(2, null, [[0, 2]])], null, unproven),
    receivers: side([link(null, 1, [[0, 5]])], null, unproven),
    links: [link(0, 1, [[0, 3]]), link(2, 1, [[0, 2]])],
    largestLink: amount(3),
  };
  const model = projectTradeMatrix(view(reply));
  assert.deepEqual(model.rows.map(row => [row.id, row.label]), [['s1', 'Loading name…'], ['collective', 'Unknown countries, combined']]);
  assert.deepEqual(model.columns.map(column => column.id), ['s6']);
  assert.equal(model.cells.present.size, 2);
  const absent = model.cells.absent;
  assert.equal(typeof absent === 'function' ? absent(0).kind : absent.kind, 'missing');
});

test('a faction is sent as the group, resolved in Rust at the hour, never as the union of its members or a one-member centre', async () => {
  const { tradeFlowDeclaration } = await import('../../widgets/trade-flows/tradeFlowsProjection.ts');
  const ctx = (membership: 'hour' | 'ever') => ({
    track: { source: { id: 's', revision: '1' }, axisOffset: 0 },
    branch: { id: '1' },
    options: {},
    countryTag: null,
    countries: { all: false, top: null, tags: ['GER'] },
    widget: { countries: { kind: 'group', group: { type: 'faction', id: '42' }, membership, aggregate: false } },
  }) as never;
  const view = { following: true, staticSpan: null, height: 360 };
  const paramsOf = (membership: 'hour' | 'ever') => (tradeFlowDeclaration.sources(ctx(membership), view).entries[0]!.read as unknown as { params: Record<string, unknown> }).params;
  const params = paramsOf('hour');
  assert.deepEqual(params.group, { type: 'faction', id: '42', membership: 'hour' });
  assert.equal(params.subject, undefined, 'one member so far is not a centred view of it');
  assert.equal(params.countries, undefined, 'no member list leaves the client');
  assert.deepEqual(paramsOf('ever').group, { type: 'faction', id: '42', membership: 'ever' });
});

test('a recorded 0.04 a day is drawn as under one tenth and read exactly with its unit, never as 0', () => {
  const small: CountryFlowReply = {
    ...base,
    endpoints: [endpoint(1, 'USA'), endpoint(2, 'HOL')],
    subject: 0,
    incoming: side([{ from: 1, to: 0, others: null, amounts: [{ part: 0, amount: '4000' }], total: '4000', missing: null }], '4000'),
    largest: '4000',
    largestLink: '4000',
  };
  const model = projectCountryFlow(view(small), { ...TEXT, unit: 'per day' });
  const node = model.left[0]!;
  assert.equal(node.figure, '<0.1');
  assert.equal(node.audit, '0.04 per day · Oil: 0.04 per day');
  assert.equal(findFlowTooltip(model, 'node:left:s2')?.items?.[0]?.value, '0.04 per day');
});

test('a witnessed zero delivery keeps its partner but its ribbon, which paints nothing, takes no hit', () => {
  const zero: CountryFlowReply = {
    ...base,
    endpoints: [endpoint(1, 'USA'), endpoint(2, 'HOL'), endpoint(6, 'ENG')],
    subject: 0,
    incoming: side([link(1, 0, [[0, 0]])], '0'),
    outgoing: side([link(0, 2, [[0, 40]])], amount(40)),
    largest: amount(40),
    largestLink: amount(40),
  };
  const model = projectCountryFlow(view(zero), TEXT);
  assert.equal(model.left[0]!.figure, '0', 'the witnessed zero stays on the partner node');
  const layout = layoutFlow(model, 846, 360);
  const box = layout.ribbons.find(item => item.ribbon.from === 'left:s2')!;
  const x = (box.x0 + box.x1) / 2;
  for (let y = layout.bodyTop; y < layout.height; y++) assert.notEqual(hitFlow(layout, x, y), 'ribbon:left:s2>centre:s1');
});

test('the other partners band is one neutral band, its parts kept for the tooltip', () => {
  const model = projectCountryFlow(view(centred), TEXT);
  const others = model.ribbons.find(ribbon => ribbon.from === 'left:others')!;
  assert.equal(others.others, true);
  assert.equal(model.ribbons.filter(ribbon => ribbon.others).length, 1, 'named partners keep their resource colours');
  assert.equal(findFlowTooltip(model, `ribbon:${others.key}`)?.items?.[0]?.label, 'Oil');
});

test('every flow widget offers Top countries, and the choice reaches its product as the explicit top beside the viewport fold', async () => {
  const { createServer } = await import('vite');
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true }, server: { middlewareMode: true, watch: null, hmr: false, ws: false } });
  const { groupKinds } = await vite.ssrLoadModule('/ui/src/dashboard/settings/CountriesSection.tsx').finally(() => vite.close()) as typeof import('../../dashboard/settings/CountriesSection.tsx');
  const declarations = [
    [(await import('../../widgets/trade-flows/definition.ts')).definition, (await import('../../widgets/trade-flows/tradeFlowsProjection.ts')).tradeFlowDeclaration],
    [(await import('../../widgets/production-licences/definition.ts')).definition, (await import('../../widgets/production-licences/licencesProjection.ts')).licencesDeclaration],
    [(await import('../../widgets/volunteers/definition.ts')).definition, (await import('../../widgets/volunteers/volunteersProjection.ts')).volunteersDeclaration],
  ] as const;
  const ctx = (top: number | null) => ({
    track: { source: { id: 's', revision: '1' }, axisOffset: 0 },
    branch: { id: '1' },
    options: {},
    countryTag: null,
    countries: { all: top === null, top, tags: [] },
    widget: { countries: top === null ? { kind: 'all' } : { kind: 'group', group: { type: 'top', count: top }, membership: 'hour', aggregate: false } },
  }) as never;
  const view = { following: true, staticSpan: null, height: 360 };
  for (const [definition, declaration] of declarations) {
    assert.ok(groupKinds(undefined, definition.countries).some(kind => kind.value === 'top'), `${definition.kind} offers Top countries`);
    const plan = (top: number | null) => declaration.sources(ctx(top), view);
    const params = (top: number | null) => (plan(top).entries[0]!.read as unknown as { params: { limit?: number; top?: number } }).params;
    assert.deepEqual([params(10).top, params(10).limit], [10, flowPerSide(view)], `${definition.kind}: Top 10 is its own request field; the viewport fold stays`);
    assert.equal(params(3).top, 3);
    assert.equal(params(null).top, undefined);
    assert.notEqual(plan(10).owner, plan(null).owner, 'Top 10 and All are different requests');
  }
});

test('in a group, a sender whose total is unknown reserves the width of its known ribbons and its missing marks, so nothing runs into the next node', () => {
  const reply: CountryFlowReply = {
    ...base,
    endpoints: [endpoint(10, 'ITA'), endpoint(8, 'SOV'), endpoint(11, 'SPD'), endpoint(9, 'SPR')],
    senders: side([link(0, null, [[0, 9], [1, null]]), link(1, null, [[0, 4]])], null, 'A deal\'s daily delivery was not recorded at this hour'),
    receivers: side([link(null, 2, [[0, 9]]), link(null, 3, [[0, 4], [1, null]])], null, 'A deal\'s daily delivery was not recorded at this hour'),
    links: [link(0, 2, [[0, 9]]), link(0, 3, [[1, null]]), link(1, 3, [[0, 4]])],
    largest: amount(13),
    largestLink: amount(9),
  };
  const layout = layoutFlow(projectCountryFlow(view(reply), TEXT), 846, 360);
  const ita = layout.nodes.find(box => box.node.key === 'left:s10')!;
  const sov = layout.nodes.find(box => box.node.key === 'left:s8')!;
  const out = layout.ribbons.filter(box => box.ribbon.from === 'left:s10');
  const end = Math.max(...out.map(box => box.a1));
  assert.ok(ita.height >= end - ita.y - 0.01, 'the unknown node holds all its ribbons');
  assert.ok(sov.slotTop >= end, 'the next node starts below them');
});

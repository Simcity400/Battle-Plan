import assert from 'node:assert/strict';
import test from 'node:test';
import { createBoundedReadView } from '../boundedView.ts';
import type { CountryFlowReply, FlowEndpoint, FlowLink, FlowSide } from '../../generated/dashboard';
import { projectCountryFlow, type CountryFlowText } from './countryFlow.ts';
import { deriveFlowAudit, findFlowTooltip } from './model.ts';
import { hitFlow, layoutFlow } from './painter.ts';
import { assertFreshPaint, assertQuietFrames, LANE_THEME } from '../laneRuntime.fixture.ts';
import { registerFamilyProjection } from '../registry.ts';
import { familyFlowLaneView } from './laneView.ts';
import { projectTradeMatrix, projectTradeTable } from '../../widgets/trade-flows/tradeFlowsProjection.ts';

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
const side = (links: FlowLink[], total: string | null, missing: string | null = null): FlowSide => ({ links, total, missing });
const NONE = side([], '0');
const base: CountryFlowReply = {
  endpoints: [], subject: null, decimals: 5, parts, incoming: NONE, outgoing: NONE,
  links: [], senders: NONE, receivers: NONE, table: [], largest: '0', largestLink: '0', missing: null, warnings: [],
};
const amount = (value: number) => String(value * 100_000);
const endpoint = (slot: number | null, tag: string | null, unknown: string | null = null): FlowEndpoint => ({ slot, tag, unknown });
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
    ['left:2', 26, '26'],
    ['left:others', 14, '14'],
    ['left:3', null, 'Not recorded'],
  ]);
  assert.equal(model.left[1]!.label, '14 other partners');
  assert.equal(model.right[0]!.numeric, 74);
  assert.equal(model.notRecordedKey, true, 'a hatched partner adds the not-recorded key');
  const ribbon = model.ribbons.find(item => item.from === 'left:2')!;
  assert.deepEqual(ribbon.bands, [{ part: 0, numeric: 20 }, { part: 1, numeric: 6 }]);
  assert.equal(findFlowTooltip(model, 'ribbon:left:3>centre:1')?.missingReason, 'A deal\'s daily delivery was not recorded at this hour');
  assert.ok(deriveFlowAudit(model).some(row => row.key === 'node:left:3' && row.value === 'Not recorded'));
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
  assert.deepEqual([model.left[0]!.key, model.right[0]!.key], ['left:6', 'right:6']);
  const layout = layoutFlow(model, 846, 360);
  const left = layout.nodes.find(box => box.node.key === 'left:6')!;
  const right = layout.nodes.find(box => box.node.key === 'right:6')!;
  assert.ok(left.x < right.x, 'the two boxes are apart');
  const incoming = layout.ribbons.find(box => box.ribbon.to === 'centre:1')!;
  assert.equal(incoming.x0, left.x + 6, 'the incoming ribbon starts at the left box');
  assert.equal(hitFlow(layout, right.x + 20, right.y + right.height / 2), 'node:right:6');
  assert.equal(findFlowTooltip(model, 'node:right:6')?.items?.[0]?.value, '45');
  assert.equal(findFlowTooltip(model, 'node:left:6')?.items?.[0]?.value, '5');
  assert.equal(new Set(deriveFlowAudit(model).map(row => row.key)).size, deriveFlowAudit(model).length, 'audit keys are unique');
});

test('a licence with an unknown giver stays one licence, from a partner named as unknown with its reason', () => {
  const reason = 'The licence names a country the recording cannot link at this hour';
  const reply: CountryFlowReply = {
    ...base,
    decimals: 0,
    parts: [{ key: 'licences', name: 'Licences', nameMissing: null, ordinal: 0, icon: null }],
    endpoints: [endpoint(7, 'GER'), endpoint(8, 'ITA'), endpoint(null, null, reason)],
    subject: 0,
    incoming: side([{ from: 1, to: 0, others: null, amounts: [{ part: 0, amount: '1' }], total: '1', missing: null }, { from: 2, to: 0, others: null, amounts: [{ part: 0, amount: '1' }], total: '1', missing: null }], '2'),
    largest: '2',
    largestLink: '1',
  };
  const model = projectCountryFlow(view(reply), TEXT);
  assert.deepEqual(model.left.map(node => [node.key, node.label, node.numeric]), [['left:8', 'ITA', 1], ['left:unknown', 'Unknown country', 1]]);
  assert.equal(model.left[1]!.unknown, reason);
  assert.equal(findFlowTooltip(model, 'node:left:unknown')?.missingReason, reason);
  assert.equal(model.left[1]!.subject, undefined, 'an unknown partner is no country to open');
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
  const hol = layout.nodes.find(box => box.node.key === 'left:2')!;
  const holRibbon = layout.ribbons.find(box => box.ribbon.from === 'left:2')!;
  assert.ok(Math.abs((holRibbon.a1 - holRibbon.a0) - (holRibbon.b1 - holRibbon.b0)) < 0.01, 'a ribbon keeps its width');
  assert.ok(Math.abs(hol.height - 26 * layout.scale) < 0.01);
  const lefts = layout.nodes.filter(box => box.node.side === 'left');
  for (let i = 1; i < lefts.length; i++) assert.ok(lefts[i]!.slotTop >= lefts[i - 1]!.slotTop + lefts[i - 1]!.slotHeight, 'label slots never overlap');
  const middleX = (holRibbon.x0 + holRibbon.x1) / 2;
  const middleY = (holRibbon.a0 + holRibbon.b0 + holRibbon.a1 + holRibbon.b1) / 4;
  assert.equal(hitFlow(layout, middleX, middleY), 'ribbon:left:2>centre:1');
  assert.equal(hitFlow(layout, hol.x - 20, hol.y + hol.height / 2), 'node:left:2');
  assert.ok(layout.bodyTop + 26 * layout.scale <= 360);

  registerFamilyProjection({ kind: 'test-flow', family: 'flow', project: readView => projectCountryFlow(readView, TEXT) });
  const lane = (reply: CountryFlowReply) => ({ family: 'flow', kind: 'test-flow', options: {}, sources: {}, staticPanels: { main: reply }, scrollTop: 0, theme: LANE_THEME, width: 846, height: 360 });
  const held = lane(centred);
  const corrected: CountryFlowReply = { ...centred, outgoing: side([link(USA, ENG, [[0, 70]])], amount(70)) };
  const surface = { width: 846, height: 360, dpr: 1 };
  assertQuietFrames(() => familyFlowLaneView(() => {}), { label: 'flow', binding: held, hour: HOUR }, 24, surface);
  assertFreshPaint(() => familyFlowLaneView(() => {}), [
    { label: 'held', binding: held, hour: HOUR, check: true },
    { label: 'later', binding: held, hour: HOUR + 30, speed: 24 },
    { label: 'seek back', binding: held, hour: HOUR, check: true },
    { label: 'correction', binding: lane(corrected), hour: HOUR, check: true },
  ], surface);
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
  assert.deepEqual(model.left.map(node => node.key), ['left:10', 'left:8', 'left:11']);
  assert.deepEqual(model.right.map(node => node.key), ['right:11', 'right:9', 'right:10']);
  assert.deepEqual(model.ribbons.map(ribbon => ribbon.key), ['left:10>right:11', 'left:8>right:9', 'left:11>right:10']);
  assert.equal(model.leftHeader, 'Exporters');
  const layout = layoutFlow(model, 846, 360);
  const back = layout.ribbons.find(box => box.ribbon.key === 'left:11>right:10')!;
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
  assert.deepEqual(model.rows.map(row => [row.id, row.label]), [['1', 'USA'], ['unknown', 'Unknown country']]);
  assert.deepEqual(model.columns.map(column => column.id), ['6']);
  assert.equal(model.cells.present.size, 2);
  const absent = model.cells.absent;
  assert.equal(typeof absent === 'function' ? absent(0).kind : absent.kind, 'missing');
});

test('a faction is sent as the group, resolved in Rust at the hour, never as the union of its members or a one-member centre', async () => {
  const { tradeFlowDeclaration } = await import('../../widgets/trade-flows/tradeFlowsProjection.ts');
  const ctx = (membership: 'hour' | 'ever' | 'end') => ({
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
  assert.match(tradeFlowDeclaration.unavailable!(ctx('end'), 0) ?? '', /end of the recording/);
  assert.equal(tradeFlowDeclaration.unavailable!(ctx('hour'), 0), null);
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
  assert.equal(node.audit, '0.04 per day');
  assert.equal(findFlowTooltip(model, 'node:left:2')?.items?.[0]?.value, '0.04 per day');
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
  const box = layout.ribbons.find(item => item.ribbon.from === 'left:2')!;
  const x = (box.x0 + box.x1) / 2;
  for (let y = layout.bodyTop; y < layout.height; y++) assert.notEqual(hitFlow(layout, x, y), 'ribbon:left:2>centre:1');
});

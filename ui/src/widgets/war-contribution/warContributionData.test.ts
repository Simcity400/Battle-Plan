import assert from 'node:assert/strict';
import test from 'node:test';
import type { WarContributionGroup, WarMeasureSide, WarObservation, WarTimeline, WarsReply } from '../../generated/dashboard';
import { warContributions } from '../wars/warsData.ts';
import { chosenWar, createWarChipTracker, createWarMenuTracker, readHourFor, warChipLeaderHour, warChipStateAt, warChoicesAt, warEligibilityAt } from './warContributionData.ts';

const observation = (first: number, last: number, viewpointSide: string[], opposingSide: string[], name = 'Synthetic war'): WarObservation => ({
  firstHour: first, lastHour: last, viewpointTag: viewpointSide[0], viewpointSide, opposingSide, attackersAreViewpoint: true, representative: null,
  name: { text: name, known: true, reason: null, key: null },
} as WarObservation);
const war = (id: string, first: number, last: number, sides: [string[], string[]]): WarTimeline => ({
  id, firstHour: first, lastHour: last, startedBy: false, endedBy: false, observations: [observation(first, last, ...sides)],
  participants: [], capitulations: [],
} as unknown as WarTimeline);

test('the saved war wins while it exists, else the war with most participants going on at the hour', () => {
  const reply = { wars: [war('1', 0, 100, [['A'], ['B']]), war('2', 10, 500, [['A', 'C'], ['B', 'D', 'E']]), war('3', 600, 900, [['A'], ['F']])] } as unknown as WarsReply;
  assert.equal(chosenWar(reply, null, 50)?.id, '2');
  assert.equal(chosenWar(reply, '1', 50)?.id, '1');
  assert.equal(chosenWar(reply, 'gone', 700)?.id, '3', 'a war going on beats a bigger war that ended');
  assert.equal(chosenWar(reply, null, 550)?.id, '2', 'between wars, the largest one fought so far');
  assert.equal(chosenWar(reply, null, -1), null, 'no war has started');
  const tied = { ...reply, wars: [reply.wars[0]!, war('2', 10, 500, [['A'], ['D']])] };
  assert.equal(chosenWar(tied, null, 50)?.id, '1', 'a later end cannot break a tie between active wars');
  const renamed = { ...reply, wars: [{ ...reply.wars[0]!, observations: [
    observation(0, 49, ['A'], ['B'], 'First name'), observation(50, 100, ['A'], ['B'], 'Later name'),
  ] }, reply.wars[2]!] };
  assert.deepEqual(warChoicesAt(renamed, 49), [{ id: '1', firstHour: 0, name: 'First name' }]);
  assert.deepEqual(warChoicesAt(renamed, 49, '3')[1], { id: '3', firstHour: null, name: 'The selected war has not started at this date.' });
  assert.equal(warChoicesAt(renamed, 50)[0]?.name, 'Later name');
  assert.equal(readHourFor(reply.wars[0], 550), 100, 'an ended war reads its last observed hour');
  const sides = reply.wars[1].observations[0];
  assert.equal(warContributions([{ viewpointSide: ['A'], opposingSide: ['B'], participants: [], measures: [], reason: null },
    { viewpointSide: ['A', 'C'], opposingSide: ['B', 'D', 'E'], participants: [], measures: [], reason: null }], sides)?.viewpointSide.length, 2);
  assert.equal(warContributions([], sides), null);
  const joined = observation(0, 100, ['A'], ['B', 'C']);
  const groups = [{ viewpointSide: ['A'], opposingSide: ['B'], participants: [], measures: [], reason: null },
    { viewpointSide: ['A'], opposingSide: ['C', 'D'], participants: [], measures: [], reason: null },
    { viewpointSide: ['A', 'C'], opposingSide: ['B'], participants: [], measures: [], reason: null }];
  assert.equal(warContributions(groups, joined), groups[0]);
  const changing = { ...reply, wars: [{ ...reply.wars[0]!, observations: [
    observation(0, 49, ['A'], ['B']), { ...observation(50, 100, ['A'], ['B']), attackersAreViewpoint: false },
  ] }] };
  assert.equal(warChipStateAt(changing, '1', 49).observation?.attackersAreViewpoint, true);
  assert.deepEqual(warChipStateAt(changing, '1', 49).valid, [0, 49]);
  assert.equal(warChipStateAt(changing, '1', 50).observation?.attackersAreViewpoint, false, 'the chip changes sides while the same war keeps playing');
  assert.deepEqual(warChipStateAt(changing, '1', 50).valid, [50, 100]);
});

const knownValueOf = (value: number) => ({ kind: 'known' as const, value });

test('the Sides view draws the product’s shares in two panes: share of the war on top, then each side’s countries in the product’s order', async () => {
  const { warContributionSidesProjection } = await import('./sidesProjection.ts');
  const { createBoundedReadView } = await import('../../families/boundedView.ts');
  const member = (tag: string, value: string, shareOfSide: number | null, partial = false) => ({ tag, value, shareOfSide, shareOfWar: null, partial });
  const side = (total: string, shareOfWar: number | null, members: WarMeasureSide['members'], partial = false): WarMeasureSide => ({ total, shareOfWar, partial, members });
  const country = (tag: string, side: 'viewpoint' | 'opponent') => ({ tag, side, equipmentDamage: '0', occupation: '0', airDamage: '0', strategicBombing: '0', shipDamage: '0', convoyDamage: '0', casualties: '0', unknownCasualties: '0', lendLeaseSent: '0', lendLeaseReceived: '0', score: '0', participation: '0' });
  const group: WarContributionGroup = { viewpointSide: ['BBB'], opposingSide: ['AAA', 'CCC'], reason: null,
    participants: [country('BBB', 'viewpoint'), country('AAA', 'opponent'), country('CCC', 'opponent')],
    measures: [
      { key: 'participation', viewpoint: side('100000', null, [member('BBB', '100000', 10000)]), opposing: side('100000', null, [member('CCC', '70000', 7000), member('AAA', '30000', 3000)]) },
      { key: 'casualties', viewpoint: side('500000', 4100, [member('BBB', '500000', 10000)]), opposing: side('700000', 5900, [member('CCC', '700000', 10000, true), member('AAA', '0', 0)], true) },
    ] };
  const war = { id: '1', firstHour: 10, lastHour: 500, startedBy: false, endedBy: false, participants: [], capitulations: [],
    observations: [{ ...observation(10, 500, ['BBB'], ['AAA', 'CCC']), attackersAreViewpoint: false }] } as unknown as WarTimeline;
  const wars = { available: true, wars: [war] } as unknown as WarsReply;
  const reply = { branchId: '1', hour: 100, available: true, contributions: [group], warnings: [] };
  const model = warContributionSidesProjection.project(createBoundedReadView({ hour: 100, panels: new Map([['live', reply]]) }), { wars, saved: null });
  assert.deepEqual(model.panes?.map(pane => pane.key), ['opposing', 'viewpoint'], 'the attackers are the left pane');
  const [left, right] = model.panes!;
  assert.deepEqual(left!.rows.map(row => row.key), ['opposing:CCC', 'opposing:AAA'], 'members in the product’s order');
  assert.deepEqual(left!.totalRow!.cells.casualties!.value, { kind: 'partial', value: 0.59, reason: 'Part of this side’s casualties contribution is not attributed to a country' });
  assert.equal(left!.totalRow!.cells.participation!.value.kind, 'notApplicable', 'participation is already a share of the side');
  assert.equal(left!.rows[1]!.cells.casualties!.value.kind, 'zero', 'a part the product says is zero reads 0');
  assert.deepEqual(left!.rows[1]!.cells.casualties!.format, { decimals: 0 }, 'a zero part reads 0, not 0%');
  assert.equal(left!.rows[0]!.cells.casualties!.value.kind, 'partial');
  assert.deepEqual(left!.rows[0]!.cells.participation!.value, { kind: 'known', value: 0.7 });
  assert.equal(left!.rows[0]!.cells.participation!.format, 'share', 'every share follows the one share format');
  const { formatValueState } = await import('../../families/valueFormat.ts');
  assert.equal((formatValueState(knownValueOf(0.0042), 'share') as { text: string }).text, '0.4%', 'a small non-zero share keeps its decimal, never 0%');
  assert.equal(right!.rows[0]!.group?.label, 'Share of the side');
  assert.deepEqual(model.columns.map(column => column.key), ['country', 'participation', 'casualties'], 'only the measures the product answers are columns');
  const none = warContributionSidesProjection.project(createBoundedReadView({ hour: 5, panels: new Map() }), { wars, saved: null });
  assert.equal(none.empty, 'Choose a war to see who fought it.');
  const future = warContributionSidesProjection.project(createBoundedReadView({ hour: 5, panels: new Map() }), { wars, saved: '1' });
  assert.equal(future.empty, 'The selected war has not started at this date.');
  const ended = warContributionSidesProjection.project(createBoundedReadView({ hour: 900, panels: new Map([['ended', { ...reply, hour: 500 }]]) }), { wars, saved: null });
  assert.equal(ended.panes?.length, 2, 'an ended war is read at its last hour, never later');
  const { warContributionMeasureProjection } = await import('./measureProjection.ts');
  const warShares: Readonly<Record<string, number>> = { BBB: 4100, CCC: 5900, AAA: 0 };
  const withWar = { ...group, measures: group.measures.map(measure => (measure.key !== 'casualties' ? measure : {
    ...measure,
    viewpoint: { ...measure.viewpoint, members: measure.viewpoint.members.map(item => ({ ...item, shareOfWar: warShares[item.tag]! })) },
    opposing: { ...measure.opposing, members: measure.opposing.members.map(item => ({ ...item, shareOfWar: warShares[item.tag]! })) },
  })) };
  const measured = warContributionMeasureProjection.project(createBoundedReadView({ hour: 100, panels: new Map([['live', { ...reply, contributions: [withWar] }]]) }), { wars, saved: null, measure: 'casualties' });
  const bars = measured.rows.filter(row => row.kind === 'data');
  assert.deepEqual(bars.map(row => row.key), ['opposing:CCC', 'opposing:AAA'], 'the attackers on the left, in the product’s order');
  assert.deepEqual(bars[0]!.value?.value, { kind: 'partial', value: 0.59, reason: 'Part of this side’s casualties contribution is not attributed to a country' });
  assert.equal(bars[0]!.rightSide?.key, 'viewpoint:BBB');
  assert.deepEqual(bars[0]!.rightSide?.value.value, { kind: 'known', value: 0.41 }, 'every figure is a share of all of the measure in this war');
  assert.deepEqual(bars[0]!.rightSide?.tooltip?.items, [
    { label: 'Recorded amount', value: '5' }, { label: 'Share', value: '41%' }, { label: 'Flag', missingReason: 'Loading' },
  ]);
  assert.equal(bars[1]!.value?.value.kind, 'zero');
  assert.equal(bars[1]!.rightSide, undefined, 'a shorter side leaves its half empty');
  assert.deepEqual(measured.pickList?.rows.map(row => [row.key, row.left.fraction, row.right.fraction, row.divided === true]), [['casualties', 0.59, 0.41, false], ['participation', null, null, true]],
    'the list offers the measures the product answers, each side’s part of its total, war participation set apart');
  assert.equal(measured.pickList?.selectedKey, 'casualties');
  assert.deepEqual(measured.captionSides?.map(item => item.value), ['59% partial', '41%']);
  assert.match(measured.caption ?? '', /^Casualties inflicted · Each country’s share of all casualties inflicted in this war$/);

  const casualties = group.measures[1]!;
  const missing = 'Directional participation ledger not recorded';
  const partialZero = member('AAA', '0', 0, true);
  const oneSide = { ...group, viewpointSide: ['BBB', 'BBB'], reason: missing, participants: group.participants.filter(person => person.side === 'opponent'),
    measures: [{ ...casualties, viewpoint: { total: null, partial: false, members: [], shareOfWar: null },
      opposing: { ...casualties.opposing, shareOfWar: null, members: [casualties.opposing.members[0]!, partialZero] } }] };
  const read = createBoundedReadView({ hour: 100, panels: new Map([['live', { ...reply, contributions: [oneSide] }]]) });
  const table = warContributionSidesProjection.project(read, { wars, saved: null });
  assert.deepEqual(table.panes![1]!.rows.map(row => row.key), ['viewpoint:BBB']);
  assert.deepEqual(table.panes![1]!.rows[0]!.cells.casualties!.value, { kind: 'notRecorded', reason: missing });
  const witnessed = table.panes![0]!.rows[1]!;
  assert.equal(witnessed.cells.casualties!.value.kind, 'partial');
  const amounts = typeof witnessed.tooltipExtra === 'function' ? witnessed.tooltipExtra() : witnessed.tooltipExtra;
  assert.deepEqual(amounts?.[0]?.value, { kind: 'partial', value: 0n, reason: 'Part of this side’s casualties contribution is not attributed to a country' });
  const byMeasure = warContributionMeasureProjection.project(read, { wars, saved: null, measure: 'casualties' });
  const [first, second] = byMeasure.rows.filter(row => row.kind === 'data');
  assert.deepEqual(first!.rightSide!.value.value, { kind: 'notRecorded', reason: missing });
  assert.equal(second!.rightSide, undefined);
  assert.deepEqual(second!.value!.value, { kind: 'notRecorded', reason: missing });
  assert.equal(second!.tooltip?.items?.[0]?.value, '0');
  assert.ok(second!.tooltip?.note);
  const both = { ...withWar, measures: [{ ...casualties, opposing: { ...casualties.opposing, members: [{ ...partialZero, shareOfWar: 0 }] } }] };
  const zeroShare = warContributionMeasureProjection.project(createBoundedReadView({ hour: 100, panels: new Map([['live', { ...reply, contributions: [both] }]]) }), { wars, saved: null, measure: 'casualties' });
  const zero = zeroShare.rows[0];
  assert.equal(zero?.kind === 'data' && zero.value?.value.kind, 'partial');

  const { formatDateOnly } = await import('../../families/valueFormat.ts');
  const { tableLane } = await import('../../families/table/laneView.ts');
  const { deriveTableAudit } = await import('../../families/table/model.ts');
  const { buildColumnLayouts } = await import('../../families/table/layout.ts');
  const { tableDefaults } = await import('../../families/table/settings.ts');
  const { barsLane } = await import('../../families/bars/laneView.ts');
  const { deriveBarsAudit } = await import('../../families/bars/model.ts');

  const endedAt = 500;
  const shownAt = 900;
  const expectedDate = `As of ${formatDateOnly(endedAt)}`;
  const endedSides = warContributionSidesProjection.project(createBoundedReadView({ hour: shownAt, panels: new Map([['ended', { ...reply, hour: endedAt }]]) }), { wars, saved: null });
  assert.equal(endedSides.tooltipHour, endedAt, 'Sides carries the ended war read hour for tooltips');
  assert.equal(endedSides.titleSummary, expectedDate, 'Sides labels the evidence date in its summary');
  const sidesFlat = endedSides.panes!.flatMap(pane => pane.rows.map((row, i) => ({
    kind: 'data' as const, key: row.key, name: row.name, row, rank: i + 1, level: 1, parent: null, isTreeGroup: false, isOpen: false, childCount: 0,
  })));
  const sidesTip = tableLane.tooltip({ model: endedSides, layout: { shape: { flatRows: sidesFlat } } } as never, 'row:opposing:CCC', false);
  assert.equal(sidesTip?.hour, endedAt, 'Sides lane tooltip stamps the supplying read hour, not the display hour');
  const sidesAudit = deriveTableAudit({ model: endedSides, flatRows: sidesFlat, visibleColumns: buildColumnLayouts(endedSides.columns, tableDefaults(endedSides.columns, { density: 'normal', sort: { column: 'participation', direction: 'descending' } })) });
  const sidesMeta = JSON.parse(sidesAudit.find(r => r.key === '__table__:meta')!.value!) as { tooltipHour?: unknown; titleSummary?: string };
  assert.deepEqual(sidesMeta.tooltipHour, ['int', String(endedAt)], 'Sides audit records the supplying read hour');
  assert.deepEqual(sidesMeta.titleSummary, ['str', expectedDate]);

  const endedMeasure = warContributionMeasureProjection.project(createBoundedReadView({ hour: shownAt, panels: new Map([['ended', { ...reply, hour: endedAt, contributions: [withWar] }]]) }), { wars, saved: null, measure: 'casualties' });
  assert.equal(endedMeasure.summaryText, expectedDate, 'By measure puts the supplying read hour in summaryText (header, unaffected by narrow 714px caption width)');
  const { layoutFamilyBars } = await import('../../families/bars/layout.ts');
  const narrowLayout = layoutFamilyBars(endedMeasure, 714, 300, { font: 'sans-serif', fontSize: { small: 12, body: 13 } } as never, text => text.length * 7);
  const narrowStructure = barsLane.structure({ model: endedMeasure, layout: narrowLayout } as never);
  assert.equal((narrowStructure.meta as { summaryText?: string }).summaryText, expectedDate, 'summaryText stays intact in structure meta at the 714px picker threshold');
  const barLeftTip = barsLane.tooltip({ model: endedMeasure } as never, 'row:opposing:CCC', false);
  const barRightTip = barsLane.tooltip({ model: endedMeasure } as never, 'right:viewpoint:BBB', false);
  const barPickTip = barsLane.tooltip({ model: endedMeasure } as never, 'pick:casualties', false);
  assert.equal(barLeftTip?.hour, endedAt, 'By measure left-side tooltip stamps the supplying read hour');
  assert.equal(barLeftTip?.payload?.hour, endedAt);
  assert.equal(barRightTip?.hour, endedAt, 'By measure right-side tooltip stamps the supplying read hour');
  assert.equal(barRightTip?.payload?.hour, endedAt);
  assert.equal(barPickTip?.hour, endedAt, 'By measure measure-picker tooltip stamps the supplying read hour');
  assert.equal(barPickTip?.payload?.hour, endedAt);
  const barsAudit = deriveBarsAudit(endedMeasure);
  assert.equal(barsAudit.find(r => r.key === '__bars__:dataHours')?.value, JSON.stringify([endedAt]), 'By measure audit records the supplying read hour');

  const endedMissingSide = warContributionMeasureProjection.project(createBoundedReadView({ hour: shownAt, panels: new Map([['ended', { ...reply, hour: endedAt, contributions: [oneSide] }]]) }), { wars, saved: null, measure: 'casualties' });
  const missingRightTip = barsLane.tooltip({ model: endedMissingSide } as never, 'right:viewpoint:BBB', false);
  assert.equal(missingRightTip?.hour, endedAt, 'missing-member right-side tooltip stamps the supplying read hour');
  assert.equal(missingRightTip?.payload?.hour, endedAt);

  const leftMissingAndShorter = {
    ...group,
    viewpointSide: ['BBB', 'DDD'],
    opposingSide: ['AAA'],
    reason: missing,
    measures: [{
      ...casualties,
      opposing: { total: null, partial: false, members: [], shareOfWar: null },
      viewpoint: { total: '500000', partial: false, shareOfWar: null, members: [member('BBB', '300000', 6000), member('DDD', '200000', 4000)] },
    }],
  };
  const rightLongerWars = {
    available: true,
    wars: [{
      ...war,
      observations: [{ ...observation(10, 500, ['BBB', 'DDD'], ['AAA']), attackersAreViewpoint: false }],
    }],
  } as unknown as WarsReply;
  const rightLonger = warContributionMeasureProjection.project(createBoundedReadView({ hour: shownAt, panels: new Map([['ended', { ...reply, hour: endedAt, contributions: [leftMissingAndShorter] }]]) }), { wars: rightLongerWars, saved: null, measure: 'casualties' });
  const missingLeftTip = barsLane.tooltip({ model: rightLonger } as never, 'row:opposing:AAA', false);
  assert.equal(missingLeftTip?.hour, endedAt, 'missing-member left-side tooltip stamps the supplying read hour');
  assert.equal(missingLeftTip?.payload?.hour, endedAt);
  assert.deepEqual(missingLeftTip?.payload?.items, [{ label: 'Flag', missingReason: 'Loading' }], 'missing-member left-side tooltip preserves default art status items');
  assert.equal(barsLane.tooltip({ model: rightLonger } as never, 'row:opposing:none:1', false), null, 'empty synthetic left half on a right-longer roster emits no tooltip');
});

test('warContributionSources plans no contribution reads when no eligible war exists and keeps ended idle while live or prestart', async () => {
  const { warContributionSources } = await import('./warContributionPlan.ts');
  const { setToken } = await import('../../dashboard/options.ts');
  const { buildPlaybackBoundedRequest, playbackIdleInterval } = await import('../../families/broker.ts');
  const makeCtx = (aux: WarsReply | undefined, following: boolean, saved?: string, mode: 'follow' | 'pin' | 'campaign' = following ? 'follow' : 'campaign', pinOffset = 0) => ({
    track: { id: 't1', source: { id: 's1', revision: 1 }, branch: { id: 'b1', firstHour: 0, lastHour: 1000 }, axisOffset: 0 },
    branch: { id: 'b1', firstHour: 0, lastHour: 1000 },
    lastRecordedHour: 1000,
    widget: { id: 'w1', kind: 'war-contribution', trackIds: ['t1'], time: mode === 'pin' ? { mode: 'pin', windows: { t1: { firstOffset: pinOffset, lastOffset: pinOffset } } } : { mode }, countries: { kind: 'selected', tags: ['AAA'] } },
    options: saved ? { war: setToken(saved) } : {},
    following,
    aux,
    countryTag: 'AAA',
    countrySubject: null,
    countryName: 'Alpha',
    countries: {} as never,
    gateSpan: (first: number, last: number) => ({ firstHour: first, lastHour: last }),
    staticSpan: null,
  }) as unknown as Parameters<typeof warContributionSources>[0];

  const emptyWars = { available: true, wars: [] } as unknown as WarsReply;
  const unavailableWars = { available: false, wars: [] } as unknown as WarsReply;
  for (const aux of [undefined, emptyWars, unavailableWars]) {
    for (const following of [true, false]) {
      const plan = warContributionSources(makeCtx(aux, following));
      assert.equal(plan.entries.length, 0);
      assert.equal(plan.allowEmptyFollow, true);
    }
  }

  const withWars = { available: true, wars: [war('1', 100, 200, [['AAA'], ['BBB']]), war('2', 600, 900, [['AAA'], ['CCC']])] } as unknown as WarsReply;
  const livePlan = warContributionSources(makeCtx(withWars, true));
  const endedEntry = livePlan.entries.find(e => e.name === 'ended')!;
  const gate = { recorded: (h: number | null | undefined) => (h == null ? null : h), span: (a: number, b: number) => ({ firstHour: a, lastHour: b }) } as unknown as Parameters<typeof buildPlaybackBoundedRequest>[1]['gate'];
  const opts = (currentHour: number) => ({ branch: { firstHour: 0, lastHour: 1000 }, effectiveEndHour: 1000, currentHour, gate });
  assert.equal(buildPlaybackBoundedRequest(endedEntry, opts(50)), null, 'no ended read before any war starts');
  assert.deepEqual(playbackIdleInterval(endedEntry, opts(50)), [-Infinity, 99]);
  assert.equal(buildPlaybackBoundedRequest(endedEntry, opts(150)), null, 'no ended read while a war is live');
  assert.deepEqual(playbackIdleInterval(endedEntry, opts(150)), [100, 200]);
  const afterEnd = buildPlaybackBoundedRequest(endedEntry, opts(250));
  assert.equal((afterEnd?.request.params as { hour?: number } | undefined)?.hour, 200, 'ended war reads its last observed hour');

  assert.equal(warContributionSources(makeCtx(withWars, false, undefined, 'pin', 50)).entries.length, 0);
  assert.deepEqual(warContributionSources(makeCtx(withWars, false, undefined, 'pin', 150)).entries.map(e => e.name), ['live']);
  assert.deepEqual(warContributionSources(makeCtx(withWars, false, undefined, 'pin', 250)).entries.map(e => e.name), ['ended']);
  assert.equal(warContributionSources(makeCtx(withWars, false, '2', 'pin', 250)).entries.length, 0, 'prestart saved war plans no read');

  const gappedWar = {
    ...war('1', 100, 300, [['AAA'], ['BBB']]),
    observations: [observation(100, 150, ['AAA'], ['BBB']), observation(200, 300, ['AAA'], ['BBB'])],
  } as unknown as WarTimeline;
  const gappedWars = { available: true, wars: [gappedWar] } as unknown as WarsReply;
  const gappedPlan = warContributionSources(makeCtx(gappedWars, true, '1'));
  const gappedEnded = gappedPlan.entries.find(e => e.name === 'ended')!;
  assert.equal(buildPlaybackBoundedRequest(gappedEnded, opts(175)), null, 'observation gap does not fetch ended');
  assert.deepEqual(playbackIdleInterval(gappedEnded, opts(175)), [151, 199]);
  assert.equal(warContributionSources(makeCtx(gappedWars, false, '1', 'pin', 175)).entries.length, 0);
  const { chosenGroup } = await import('./warContributionPlan.ts');
  const { createBoundedReadView } = await import('../../families/boundedView.ts');
  assert.deepEqual(
    chosenGroup(createBoundedReadView({ hour: 175, panels: new Map() }), { wars: gappedWars, saved: '1' }),
    { kind: 'blank', empty: 'This war’s contributions were not recorded at this time.', emptyKind: 'unavailable' },
  );
  const savedEndedPlan = warContributionSources(makeCtx(withWars, true, '1'));
  const savedEnded = savedEndedPlan.entries.find(e => e.name === 'ended')!;
  assert.equal(savedEnded.read.shape, 'point');
  if (savedEnded.read.shape === 'point') {
    assert.deepEqual(savedEnded.read.provenInterval?.(undefined, 250), [201, Infinity], 'saved ended war stays valid even when another war starts');
  }

  const { askUnits, settleUnits, unitEntries } = await import('../../families/readUnits.ts');
  const { readPlanExact } = await import('../../families/broker.ts');
  const pinEndedPlan = warContributionSources(makeCtx(withWars, false, undefined, 'pin', 250));
  const pinUnits = askUnits(unitEntries(pinEndedPlan, false, []), {
    following: false,
    trackOffset: 0,
    displayHour: 250,
    pinned: { timeMode: 'pin', branch: { firstHour: 0, lastHour: 1000 }, window: { firstHour: 250, lastHour: 250 }, pinHour: 250, gate },
    content: 1,
  });
  assert.equal(pinUnits.length, 1);
  assert.equal((pinUnits[0]!.spec?.request.params as { hour?: number } | undefined)?.hour, 200);
  const endedReply = {
    branchId: 'b1',
    hour: 200,
    available: true,
    contributions: [{
      viewpointSide: ['AAA'],
      opposingSide: ['BBB'],
      reason: null,
      participants: [],
      measures: [{ key: 'participation', viewpoint: { total: '100000', shareOfWar: null, partial: false, members: [] }, opposing: { total: '100000', shareOfWar: null, partial: false, members: [] } }],
    }],
    warnings: [],
  };
  const settled = settleUnits(pinUnits, new Map([[pinUnits[0]!.name, { key: pinUnits[0]!.key!, data: endedReply }]]), {
    following: false,
    moving: false,
    hour: 250,
    subscribed: false,
  });
  assert.equal(settled.kind, 'complete');
  if (settled.kind === 'complete') {
    assert.equal(settled.bundle.hour, 250, 'static ended war preserves display hour 250 while using hour 200 contributions');
    const shown = chosenGroup(createBoundedReadView({ hour: settled.bundle.hour, panels: settled.bundle.panels }), { wars: withWars, saved: null });
    assert.equal(shown.kind, 'shown');
    if (shown.kind === 'shown') assert.equal(shown.reply.hour, 200);
  }

  let exactFetches = 0;
  const exactClient = { product: async () => { exactFetches++; return { data: null }; } } as unknown as Parameters<typeof readPlanExact>[2]['client'];
  for (const [emptyPlan, atHour, expectedEmpty, warsArg, savedArg] of [
    [warContributionSources(makeCtx(emptyWars, true)), 50, 'Choose a war to see who fought it.', emptyWars, null],
    [warContributionSources(makeCtx(withWars, false, '2', 'pin', 250)), 250, 'The selected war has not started at this date.', withWars, '2'],
    [warContributionSources(makeCtx(gappedWars, false, '1', 'pin', 175)), 175, 'This war’s contributions were not recorded at this time.', gappedWars, '1'],
  ] as const) {
    const exactView = await readPlanExact(emptyPlan, atHour, {
      client: exactClient,
      following: false,
      timeMode: 'pin',
      branch: { firstHour: 0, lastHour: 1000 },
      window: { firstHour: atHour, lastHour: atHour },
      staticSpan: null,
      gate,
    });
    assert.ok(exactView, 'allowed empty plan produces a dated empty exact read view');
    assert.equal(exactView.hour, atHour);
    const group = chosenGroup(exactView, { wars: warsArg, saved: savedArg });
    assert.equal(group.kind, 'blank');
    if (group.kind === 'blank') assert.equal(group.empty, expectedEmpty);
  }
  assert.equal(exactFetches, 0, 'allowed empty exact reads fetch no contribution products');

  const { PlaybackController } = await import('../../dashboard/playback.ts');
  const { watchPlanValid } = await import('../../families/host/planSources.ts');
  const splitWar = {
    ...war('1', 100, 200, [['AAA'], ['BBB']]),
    observations: [observation(100, 125, ['AAA'], ['BBB']), observation(175, 200, ['AAA'], ['BBB'])],
  } as unknown as WarTimeline;
  const splitWars = { available: true, wars: [splitWar] } as unknown as WarsReply;
  const followClock = new PlaybackController();
  followClock.setBounds({ firstHour: 0, lastHour: 300 }, true);
  followClock.seek(50);

  let replans = 0;
  let stopWatch: (() => void) | undefined;
  let activePlan = warContributionSources({ ...makeCtx(splitWars, true, '1'), planHour: followClock.getHour() });
  const arm = () => {
    stopWatch?.();
    stopWatch = watchPlanValid(followClock, 0, activePlan.valid, () => {
      replans++;
      activePlan = warContributionSources({ ...makeCtx(splitWars, true, '1'), planHour: followClock.getHour() });
      arm();
    });
  };
  arm();

  assert.deepEqual(activePlan.entries.map(e => e.name), [], 'prestart follow mode subscribes nothing');
  assert.deepEqual(activePlan.valid, [-Infinity, 99]);
  const askedHours: number[] = [];
  const trackingClient = {
    product: async (req: { params: { hour?: number } }) => {
      askedHours.push(req.params.hour ?? -1);
      return { data: { ...endedReply, hour: req.params.hour ?? 0 } };
    },
  } as unknown as Parameters<typeof readPlanExact>[2]['client'];
  const exactOpts = (at: number) => ({
    client: trackingClient,
    following: true,
    timeMode: 'follow' as const,
    branch: { firstHour: 0, lastHour: 300 },
    window: null,
    staticSpan: null,
    gate,
    ...(at >= 0 ? {} : {}),
  });
  assert.ok(await readPlanExact(activePlan, 50, exactOpts(50)));
  assert.deepEqual(askedHours, [], 'prestart follow exact read fetches no contributions');

  followClock.seek(99);
  assert.equal(replans, 0);
  followClock.step(1);
  assert.equal(replans, 1, 'wakes at war start (100) without remount');
  assert.deepEqual(activePlan.entries.map(e => e.name), ['live']);
  assert.deepEqual(activePlan.valid, [100, 125]);

  followClock.seek(150);
  assert.equal(replans, 2, 'wakes when entering observation gap (126..174)');
  assert.deepEqual(activePlan.entries.map(e => e.name), [], 'observation gap unsubscribes live');
  assert.deepEqual(activePlan.valid, [126, 174]);
  assert.ok(await readPlanExact(activePlan, 150, exactOpts(150)));
  assert.deepEqual(askedHours, [], 'gap follow exact read fetches no contributions');

  followClock.seek(175);
  assert.equal(replans, 3, 'wakes when war resumes observation (175..200)');
  assert.deepEqual(activePlan.entries.map(e => e.name), ['live']);
  assert.deepEqual(activePlan.valid, [175, 200]);

  followClock.seek(250);
  assert.equal(replans, 4, 'wakes after war ends (201..Infinity)');
  assert.deepEqual(activePlan.entries.map(e => e.name), ['ended'], 'ended war drops live subscription and plans only ended');
  assert.deepEqual(activePlan.valid, [201, Infinity]);
  const endedExact = await readPlanExact(activePlan, 250, exactOpts(250));
  assert.equal(endedExact?.hour, 250);
  assert.deepEqual(askedHours, [200], 'ended follow exact read fetches only hour 200, never 250');

  followClock.seek(50);
  assert.equal(replans, 5, 'seeking back before start returns to empty plan');
  assert.deepEqual(activePlan.entries.map(e => e.name), []);
  stopWatch?.();

  let obsReads = 0;
  const manyObs = Array.from({ length: 2000 }, (_, i) => observation(i, i, ['AAA'], [`B${i}`]));
  const countingObs = new Proxy(manyObs, {
    get(target, prop, receiver) {
      if (typeof prop === 'string' && /^\d+$/.test(prop)) obsReads++;
      return Reflect.get(target, prop, receiver);
    },
  });
  const hourlyWars = {
    available: true,
    wars: [{ ...war('1', 0, 1999, [['AAA'], ['B0']]), observations: countingObs } as unknown as WarTimeline],
  } as unknown as WarsReply;
  warChipStateAt(hourlyWars, null, 0);
  obsReads = 0;
  for (let h = 1; h <= 100; h++) {
    const state = warChipStateAt(hourlyWars, null, h);
    assert.deepEqual(state.valid, [h, h]);
  }
  assert.ok(obsReads < 4000, `cached edge index avoids rescanning all 2,000 observations per boundary (reads=${obsReads})`);

  warEligibilityAt(hourlyWars, null, 0);
  obsReads = 0;
  for (let h = 1; h <= 100; h++) {
    assert.deepEqual(warEligibilityAt(hourlyWars, null, h), { mode: 'live', valid: [0, 1999] });
  }
  assert.equal(obsReads, 0, 'warm eligibility lookup reuses cached coalesced segments with zero observation accesses');

  const splitByUnobserved = {
    available: true,
    wars: [war('1', 100, 125, [['AAA'], ['BBB']]), war('2', 175, 200, [['AAA'], ['BBB']])],
    unobserved: [{ firstHour: 126, lastHour: 174 }],
  } as unknown as WarsReply;
  assert.deepEqual(warEligibilityAt(splitByUnobserved, null, 150), { mode: 'none', valid: [126, 174] });
  const gapFollowPlan = warContributionSources({ ...makeCtx(splitByUnobserved, true), planHour: 150 });
  assert.deepEqual(gapFollowPlan.entries, [], 'WarsReply.unobserved gap plans no ended or live read');
  assert.deepEqual(gapFollowPlan.valid, [126, 174]);
  assert.deepEqual(
    chosenGroup(createBoundedReadView({ hour: 150, panels: new Map([['ended', { ...endedReply, hour: 125 }]]) }), { wars: splitByUnobserved, saved: null }),
    { kind: 'blank', empty: 'This war’s contributions were not recorded at this time.', emptyKind: 'unavailable' },
    'unobserved war gap shows unavailable instead of the previous war’s hour-125 contributions',
  );

  const { createLaneView } = await import('../../families/laneRuntime.ts');
  const { barsLane } = await import('../../families/bars/laneView.ts');
  const { installRecordingCanvas, LANE_THEME, LaneTransport } = await import('../../families/laneRuntime.fixture.ts');
  const { MEASURE_VIEW } = await import('./measureProjection.ts');
  const recording = installRecordingCanvas();
  try {
    const view = createLaneView(barsLane, () => {});
    const state = new Map<string, unknown>();
    let seq = 0;
    const content = () => `c${++seq}`;
    const binding = new LaneTransport().carry({
      family: 'bars' as const,
      kind: 'war-contribution',
      view: MEASURE_VIEW,
      options: { wars: splitWars, saved: '1', measure: 'casualties' },
      sources: {},
      width: 480,
      height: 240,
      scrollTop: 0,
      theme: LANE_THEME,
      audit: true,
      auditId: 1,
    });
    const paintAt = (hour: number, seek: number) => view.paint({
      widget: 1,
      subs: [],
      pictures: [],
      hour,
      seek,
      speed: 0,
      now: 1000 + seek * 100,
      inspect: null,
      binding: { generation: 1, surfaces: [{ width: 480, height: 240, dpr: 1 }], offset: 0, pin: null },
      model: binding,
    } as never, state, content);
    const at50 = paintAt(50, 0);
    assert.equal(at50.mode, 'picture');
    const text50 = JSON.stringify(at50.mode === 'picture' ? at50.fields : null);
    assert.ok(text50.includes('The selected war has not started at this date.'), text50);

    const at150 = paintAt(150, 1);
    assert.equal(at150.mode, 'picture');
    const text150 = JSON.stringify(at150.mode === 'picture' ? at150.fields : null);
    assert.ok(text150.includes('This war’s contributions were not recorded at this time.'), `direct seek 50 -> 150 must replace prestart with gap message: ${text150}`);

    const back50 = paintAt(50, 2);
    assert.equal(back50.mode, 'picture');
    const textBack50 = JSON.stringify(back50.mode === 'picture' ? back50.fields : null);
    assert.ok(textBack50.includes('The selected war has not started at this date.'), textBack50);
  } finally {
    recording.restore();
  }
});

test('war chip and menu watchers stay quiet across 100 played hours and wake only on observation or lead-identity changes', async () => {
  const { PlaybackController } = await import('../../dashboard/playback.ts');
  const { watchBoundary } = await import('../../dashboard/widgetTime.ts');
  const { IdentityBroker } = await import('../../families/identity/broker.ts');
  const { countryTagKey, readIdentityAtHeadHour, UNHELD_HEAD_HOUR } = await import('../../families/identity/react.ts');

  const wars = {
    available: true,
    wars: [
      {
        ...war('1', 100, 300, [['AAA'], ['BBB']]),
        observations: [
          observation(100, 229, ['AAA'], ['BBB'], 'Early war'),
          { ...observation(230, 300, ['AAA'], ['BBB'], 'Flipped war'), attackersAreViewpoint: false },
        ],
      } as unknown as WarTimeline,
      war('2', 400, 500, [['AAA'], ['CCC']]),
    ],
  } as unknown as WarsReply;

  const offset = 10;
  const clock = new PlaybackController();
  clock.setBounds({ firstHour: 0, lastHour: 600 }, true);
  clock.seek(110);

  let scans = 0;
  let wakes = 0;
  const shownHour = (axis: number) => Math.floor(axis) + offset;
  const pending = createWarChipTracker(undefined, null, shownHour, offset);
  const p1 = pending.read(140);
  assert.strictEqual(pending.read(140), p1, 'pending chip snapshot keeps reference equality while wars are loading');
  assert.strictEqual(pending.read(150), p1, 'pending chip snapshot stays stable across hours while wars are loading');
  const tracker = createWarChipTracker(wars, null, shownHour, offset, (reply, saved, at) => {
    scans++;
    return warChipStateAt(reply, saved, at);
  });
  const initial = tracker.read(clock.getHour());
  assert.equal(initial.war?.id, '1');
  assert.equal(initial.observation?.attackersAreViewpoint, true);
  assert.equal(scans, 1);

  const stopChip = watchBoundary(clock, tracker.span, () => { wakes++; });
  assert.equal(scans, 1, 'registering watchBoundary reuses cached span without rescanning');

  for (let step = 0; step < 100; step++) clock.step(1);
  assert.equal(clock.getHour(), 210);
  assert.equal(scans, 1, 'zero war-roster scans over 100 played hours inside an observation');
  assert.equal(wakes, 0, 'zero chip notifications inside an observation');

  clock.seek(219);
  assert.equal(scans, 1);
  assert.equal(wakes, 0);
  clock.step(1);
  assert.equal(clock.getHour(), 220);
  assert.equal(wakes, 1, 'wakes at exact observation flip (track hour 230)');
  assert.equal(scans, 2);
  assert.equal(tracker.read(clock.getHour()).observation?.attackersAreViewpoint, false);
  assert.equal(scans, 2, 'reading after wake hits the updated cache');

  clock.seek(390);
  assert.equal(wakes, 2, 'wakes when newer war starts (track hour 400)');
  assert.equal(tracker.read(clock.getHour()).war?.id, '2');
  stopChip();

  clock.seek(110);
  let menuWakes = 0;
  const menu = createWarMenuTracker(wars, shownHour, offset);
  assert.equal(menu.read(clock.getHour()), 100);
  const stopMenu = watchBoundary(clock, menu.span, () => { menuWakes++; });
  for (let step = 0; step < 100; step++) clock.step(1);
  assert.equal(menuWakes, 0);
  clock.seek(220);
  assert.equal(menuWakes, 1);
  assert.equal(menu.read(clock.getHour()), 230);
  stopMenu();

  const broker = new IdentityBroker({} as never);
  const scope = { sourceId: 's1', revision: '1', branchId: 'b1', content: 1 };
  const index = broker.indexOf(scope, undefined, [0, 600]);
  index.setFacts({
    branchId: 'b1',
    known: [[0, 600]],
    names: ['Old Alpha', 'New Alpha', 'Post-war Alpha'],
    reasons: [],
    flags: [{ keys: ['AAA', null, null], reasons: [null, null, null] }],
    countries: [{ tag: 'AAA', runs: [[0, 0, 0, -1, 0], [150, 1, 0, -1, 0], [300, 2, 0, -1, 0]] }],
    warnings: [],
  }, true);
  const obs = observation(100, 229, ['AAA'], ['BBB']);
  const keys = [countryTagKey('AAA')];
  const held = { current: UNHELD_HEAD_HOUR };
  clock.seek(110);
  const h1 = readIdentityAtHeadHour(broker, clock, scope, true, keys, h => warChipLeaderHour(obs, h, shownHour), undefined, [0, 600], held);
  assert.equal(h1, 120);
  clock.seek(139);
  const h2 = readIdentityAtHeadHour(broker, clock, scope, true, keys, h => warChipLeaderHour(obs, h, shownHour), undefined, [0, 600], held);
  assert.equal(h2, 120, 'holds resolved hour while lead-country identity is unchanged');
  clock.seek(140);
  const h3 = readIdentityAtHeadHour(broker, clock, scope, true, keys, h => warChipLeaderHour(obs, h, shownHour), undefined, [0, 600], held);
  assert.equal(h3, 150, 'updates lead-country identity hour when renamed mid-observation');
  assert.equal(index.resolve(countryTagKey('AAA'), h3!).kind, 'known');
  clock.seek(350);
  const h4 = readIdentityAtHeadHour(broker, clock, scope, true, keys, h => warChipLeaderHour(obs, h, shownHour), undefined, [0, 600], held);
  assert.equal(warChipLeaderHour(obs, clock.getHour(), shownHour), 229, 'clamps lead-country identity lookup to the observation end');
  assert.equal(h4, 150, 'post-war rename at 300 does not leak into the ended war chip');
  broker.dispose();
});

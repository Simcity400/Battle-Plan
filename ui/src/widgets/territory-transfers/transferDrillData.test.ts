import assert from 'node:assert/strict';
import test from 'node:test';
import type { BoundTrack, WidgetSpec } from '../../dashboard/model';
import { CLOSED, drillHour, reduceDrill } from '../../charts/drill/drillState.ts';
import { setOption } from '../../dashboard/options.ts';
import { buildTransferDiagram, decodeTransfers, transfersThrough, visibleConferences } from './territoryData.ts';
import type { TransferLink, TransferOccurrence } from './territoryData';
import { transferDrillKey, transferDrillOpen, transferDrawnSelection, transferHitSelection, transferRestoreFocus, transferScopeLinks, transferHistory, transferPartRaw, transferSelected, transferSelection, transferSelectionToken, transferTime } from './transferDrillData.ts';
import { definition, OPTIONS } from './definition.ts';
import type { PeaceDeal, PeaceDealsReply } from '../../generated/dashboard';

const branch = { id: '7', sessionId: '9', firstHour: 100, lastHour: 200, parentId: null, parentHour: null, sampleBoundary: 'Sample origin unknown', gaps: [] };
const track: BoundTrack = { id: 'main', roleId: 'primary', label: 'Campaign', branchIndex: 0, anchorOffset: null, axisOffset: 0, selectedTag: null, branch,
  source: { id: 'source', path: 'synthetic', revision: '1', metadata: { path: 'synthetic', revision: '1', name: 'Synthetic', format: 1, campaignKey: 'campaign',
    branches: [branch], countries: [], playerCountryId: null, contentVerified: false, partial: null, warnings: [] } } };
const widget: WidgetSpec = { id: 'transfers', kind: 'territory-transfers', configVersion: 1, title: 'Transfers', trackIds: ['main'], countries: { kind: 'all' },
  time: { mode: 'pin', windows: { main: { firstOffset: 36, lastOffset: 36 } } } };
const fromCountry = (hour: number) => ({ slot: 1, countryId: 101, tag: 'mod/from', hour });
const toCountry = (hour: number) => ({ slot: 2, countryId: 102, tag: 'mod/to', hour });
const state = (id: number, hour: number, vp: string | null, knownAt = hour): TransferOccurrence => ({ state: id, from: 'mod/from', to: 'mod/to', fromCountry: fromCountry(hour), toCountry: toCountry(hour), derived: false,
  dates: { beforeHour: hour, afterHour: hour }, knownAt, victoryPoints: vp, victoryPointsReadHour: vp == null ? null : knownAt });
const link = (id: string, states: TransferOccurrence[], victoryPoints: bigint | null): TransferLink => ({ id, source: '[1,101]', target: '[2,102]', from: states[0]?.from ?? 'mod/from', to: states[0]?.to ?? 'mod/to',
  states: [...states].sort((a, b) => a.dates.afterHour - b.dates.afterHour), nativeStates: states, stateCount: states.length, victoryPoints });

test('actual Pin/range windows wait for the supplying read and clip inclusively', () => {
  assert.deepEqual(definition.timeModes, ['campaign', 'follow', 'pin', 'range']);
  const parts = [link('exact', [state(1, 112, '-3', 136)], -3n), link('before', [state(2, 111, '1')], 1n),
    link('last', [state(3, 147, '2')], 2n), link('future', [state(4, 148, '4')], 4n)];
  const pin = transferTime(widget, track, [track], 199);
  assert.deepEqual(pin, { moment: 136, from: -Infinity });
  assert.equal(transfersThrough(parts, 135, 135).some(part => part.id === 'exact'), false);
  assert.equal(transfersThrough(parts, pin.moment, pin.moment)[0].states[0].dates.afterHour, 112);
  const range = transferTime({ ...widget, time: { mode: 'range', firstOffset: 12, lastOffset: 47 } }, track, [track], 199);
  assert.deepEqual(range, { moment: 147, from: 112 });
  assert.deepEqual(transfersThrough(parts, range.moment, range.moment, range.from).map(part => part.id), ['exact', 'last']);
  assert.deepEqual(transfersThrough(parts, 112, 112, 112), []);
});

test('decoded exact graphs preserve publication witnesses and conference pickers wait too', () => {
  const deal: PeaceDeal = { id: 'exact', derived: false, observation: 'recordedConference', conferenceId: 'exact', warName: null,
    firstObservedHour: 112, lastObservedHour: 112, currentTurn: null, finalized: true, firstFinalizedHour: 112, endHour: 112, endIsBy: false,
    firstAbsentHour: null, participantsDerived: false, winners: [], losers: [], captured: { hour: 112, publicationHour: 136, winners: [], losers: [], actions: [], transfers: [], subjects: [] },
    captureUnavailable: null, limitations: [], items: [{ derived: false, dates: { beforeHour: 112, afterHour: 112 }, winner: 'mod/to',
      action: { kind: 'stateTransfer', key: null, name: null }, change: { kind: 'stateTransfer', state: 1, from: 'mod/from', to: 'mod/to', fromCountry: fromCountry(112), toCountry: toCountry(112), victoryPoints: '-3', victoryPointsReadHour: 136 } }],
    transfers: { nodes: [{ id: 's', country: fromCountry(112), side: 'source' }, { id: 't', country: toCountry(112), side: 'target' }],
      links: [{ id: 'raw', source: 's', target: 't', stateCount: 1, victoryPoints: '-3', states: [{ state: 1, fromCountry: fromCountry(112), toCountry: toCountry(112), dates: { beforeHour: 112, afterHour: 112 }, victoryPoints: '-3', victoryPointsReadHour: 136 }] }] } };
  const reply: PeaceDealsReply = { branchId: '7', derived: false, deals: [deal], transfers: deal.transfers, warnings: [],
    unobserved: { settlements: [], conferences: [], ownership: [], wars: [], subjects: [], governments: [], existence: [], victoryPoints: [] } };
  const data = decodeTransfers(reply);
  assert.deepEqual(data.hours, [112, 136]);
  assert.equal(transfersThrough(data.campaign, 135, 135).length, 0);
  assert.equal(transfersThrough(data.campaign, 136, 136)[0].victoryPoints, -3n);
  assert.equal(visibleConferences([deal], 135).length, 0);
  assert.equal(visibleConferences([deal], 136).length, 1);
  assert.equal(visibleConferences([deal], 147, 113).length, 0);
});

test('sole negative, zero and unknown VP links stay exact selectable parts without positive geometry', () => {
  for (const amount of [-3n, 0n, null]) {
    const only = link('opaque-native-link', [state(41, 112, amount?.toString() ?? null)], amount);
    assert.equal(buildTransferDiagram([only], 'victoryPoints', 4).links.length, 0);
    const selected = transferSelected([only], ['opaque-native-link']);
    assert.equal(selected.length, 1);
    assert.equal(transferPartRaw(selected[0], 'victoryPoints'), amount);
    assert.equal(selected[0].states[0].state, 41);
    assert.equal(selected[0].from, 'mod/from'); assert.equal(selected[0].to, 'mod/to');
  }
});

test('history holds exact supplying-read steps and preserves native ordered intermediate overflow', () => {
  const negative = link('negative', [state(1, 112, '-3', 136)], -3n);
  assert.deepEqual(transferHistory([negative], 'states', 100, 147)[0], [{ firstHour: 100, lastHour: 135, value: 0 }, { firstHour: 136, lastHour: 147, value: 1 }]);
  assert.deepEqual(transferHistory([negative], 'victoryPoints', 100, 147)[0], [{ firstHour: 100, lastHour: 135, value: 0 }, { firstHour: 136, lastHour: 147, value: -3 }]);
  const delayed = { ...state(2, 112, '4'), victoryPointsReadHour: 125 };
  assert.deepEqual(transferHistory([link('delayed', [delayed], 4n)], 'victoryPoints', 112, 130)[0], [{ firstHour: 125, lastHour: 130, value: 4 }]);
  const max = (1n << 63n) - 1n;
  const overflow = link('ordered', [state(3, 112, max.toString()), state(4, 112, '1'), state(5, 112, '-1')], null);
  assert.deepEqual(transferHistory([overflow], 'victoryPoints', 112, 147)[0], []);
  assert.deepEqual(transferHistory([link('valid', [state(3, 112, max.toString()), state(5, 112, '-1'), state(4, 112, '1')], max)], 'victoryPoints', 112, 147)[0],
    [{ firstHour: 112, lastHour: 147, value: Number(max) }]);
});

test('5,000 contributors survive scope serialization, full selection and sparse history', () => {
  const parts = Array.from({ length: 5_000 }, (_, i) => link(`opaque ${i}:[mod]`, [{ ...state(i, 112 + i, i % 3 === 0 ? '-3' : i % 3 === 1 ? '0' : null), from: `mod-country:${i}`, to: `mod-country:${(i + 1) % 5_000}` }], i % 3 === 0 ? -3n : i % 3 === 1 ? 0n : null));
  assert.equal(new Set(parts.flatMap(part => [part.from, part.to])).size, 5_000);
  const ids = parts.map(part => part.id);
  const token = transferSelectionToken('campaign', '7', ids);
  assert.equal(transferSelection(token, 'campaign', '7')?.ids.length, 5_000);
  assert.equal(transferSelection(token, 'other', '7'), null);
  assert.equal(transferSelection(token, 'campaign', 'other'), null);
  const stored = setOption(OPTIONS, undefined, 'selection', token);
  assert.equal(stored?.selection, token);
  const ranged = transferSelection(transferSelectionToken('campaign', '7', ids, 12), 'campaign', '7')!;
  assert.equal(ranged.fromOffset, 12);
  assert.equal(ranged.ids.length, 5_000);
  const pinFromRange = transferTime(widget, track, [track], 199, ranged.fromOffset);
  assert.deepEqual(pinFromRange, { moment: 136, from: 112 });
  const samePair = link('same-pair', [state(10, 111, '1'), state(11, 112, '-3')], -2n);
  assert.deepEqual(transfersThrough([samePair], pinFromRange.moment, pinFromRange.moment, pinFromRange.from)[0].states.map(state => state.state), [11]);
  assert.equal(transferSelected(parts, ids).length, 5_000);
  assert.equal(transferSelected(parts, []).length, 5_000);
  const history = transferHistory(parts, 'states', 100, 10_000);
  assert.equal(history.length, 5_000);
  assert.ok(history.every(runs => runs.length === 2));
});

test('history picking stops Follow and resuming reads the actual clock', () => {
  const open = reduceDrill(CLOSED, { type: 'open', breakdown: 'parts', hour: 136, playing: false, playhead: null });
  const picked = reduceDrill(open, { type: 'pick', hour: 124 });
  assert.equal(drillHour(picked, 170), 124);
  assert.equal(drillHour(reduceDrill(picked, { type: 'follow' }), 170), 170);
});

test('Campaign, Pin and Range activate the held drawn cohort before explicit actual-clock Follow', () => {
  const cohort = [link('native-link', [state(41, 150, '3')], 3n)];
  const cases: { time: WidgetSpec['time']; clock: number; drawn: number }[] = [
    { time: { mode: 'campaign' }, clock: 100, drawn: 200 },
    { time: { mode: 'pin', windows: { main: { firstOffset: 50, lastOffset: 50 } } }, clock: 170, drawn: 150 },
    { time: { mode: 'range', firstOffset: 40, lastOffset: 60 }, clock: 170, drawn: 160 },
  ];
  for (const { time, clock, drawn } of cases) for (const playing of [false, true]) {
    const held = transferTime({ ...widget, time }, track, [track], clock);
    assert.equal(held.moment, drawn);
    const opened = reduceDrill(CLOSED, transferDrillOpen(held.moment, time.mode === 'follow', clock, playing));
    assert.equal(opened.follow, false);
    assert.equal(drillHour(opened, clock), drawn);
    assert.equal(transfersThrough(cohort, drillHour(opened, clock)!, drawn, held.from)[0].states[0].state, 41);
    assert.equal(drillHour(reduceDrill(opened, { type: 'follow' }), clock), clock);
  }
  const heldFrame = reduceDrill(CLOSED, transferDrillOpen(155, false, 100, false));
  assert.equal(drillHour(heldFrame, 100), 155, 'use the actual drawn frame, independently of a newer model target');
  const following = reduceDrill(CLOSED, transferDrillOpen(160, true, 170, false));
  assert.equal(drillHour(following, 170), 170);
});

test('a popout changing raw cohorts remounts its local detail but playback does not', () => {
  const a = transferDrillKey('source', '7', 'conference-A', ['raw-A']);
  const b = transferDrillKey('source', '7', 'conference-A', ['raw-B']);
  assert.notEqual(a, b, 'A detail must not filter the new B cohort');
  assert.equal(a, transferDrillKey('source', '7', 'conference-A', ['raw-A']), 'day/value updates keep the same cohort key');
  assert.equal(transferDrillKey('source', '7', null, ['raw-A', 'raw-B']), transferDrillKey('source', '7', null, ['raw-B', 'raw-A']));
  assert.notEqual(a, transferDrillKey('source', '7', 'conference-B', ['raw-A']), 'reused raw IDs in another conference are a new cohort');
});

test('a drawn conference A hit keeps A when the latest React model is B with reused pair IDs', () => {
  const pairA = link('same-native-pair', [state(11, 150, '3')], 3n);
  const pairB = link('same-native-pair', [state(22, 170, '4')], 4n);
  const data = { campaign: [pairA, pairB], conferences: new Map([['A', [pairA]], ['B', [pairB]]]) };
  const drawnLink = { id: 'ribbon', source: 'giver', target: 'taker', value: 3, color: 'neutral', order: 'pair', ...transferDrawnSelection([pairA.id], 'A') };
  const drawnNode = { id: 'giver', side: 'source' as const, label: 'Giver', color: 'neutral', order: 'giver', ...transferDrawnSelection([pairA.id], 'A') };
  const latest = transferDrawnSelection([pairB.id], 'B');
  for (const hit of [{ kind: 'link' as const, link: drawnLink }, { kind: 'node' as const, node: drawnNode }]) {
    const frozen = transferHitSelection(hit)!;
    assert.equal(frozen.transferConference, 'A');
    assert.equal(latest.transferConference, 'B');
    const selected = transferSelected(transferScopeLinks(data, frozen.transferConference), frozen.transferIds);
    assert.equal(selected[0].states[0].state, 11);
    assert.equal(transferScopeLinks(data, latest.transferConference)[0].states[0].state, 22);
  }
  assert.equal(transferScopeLinks(data, null), data.campaign, 'a campaign hit keeps campaign scope even if the latest picker is a conference');
});

test('queued close focus restores only the owned anchor/panel and leaves another drill focused', () => {
  const body = {} as Node, anchorControl = {} as Node, ownPanelControl = {} as Node, otherPanelControl = {} as Node, outsideButton = {} as Node;
  const anchor: Pick<Node, 'contains'> = { contains: target => target === anchorControl };
  const ownedPanel: Pick<Node, 'contains'> = { contains: target => target === ownPanelControl };
  assert.equal(transferRestoreFocus(null, body, anchor, ownedPanel), true);
  assert.equal(transferRestoreFocus(body, body, anchor, ownedPanel), true);
  assert.equal(transferRestoreFocus(anchorControl, body, anchor, ownedPanel), true);
  assert.equal(transferRestoreFocus(ownPanelControl, body, anchor, ownedPanel), true);
  assert.equal(transferRestoreFocus(otherPanelControl, body, anchor, ownedPanel), false, 'another .drill-popout is outside the owned closing panel');
  assert.equal(transferRestoreFocus(outsideButton, body, anchor, ownedPanel), false);
});

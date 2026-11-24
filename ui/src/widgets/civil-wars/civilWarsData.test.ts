import assert from 'node:assert/strict';
import test from 'node:test';
import { laneRequests } from '../../charts/timeline/laneRequests.ts';
import { changesBy } from '../../charts/timeline/runState.ts';
import { countryIdentityIndex } from '../../dashboard/countryIdentity.ts';
import { worldFilter } from '../../dashboard/groups.ts';
import { unknownHours } from '../../dashboard/politicsHistory.ts';
import type { CivilWar, CivilWarSide, CivilWarsReply, HolderTag, HourSpan, SideExit } from '../../generated/dashboard';
import { changeHours, civilWarLanes, civilWarsAt, exitText, jumpHours, laneCountries, NAME_NOT_KNOWN, sideTree, sideTooltip, summaryText,
  tooltipCountries, validateCivilWarsReply, warTooltip } from './civilWarsData.ts';
import type { CivilWarsOptions, LaneText, TooltipText } from './civilWarsData';

const tags = (...entries: [number, string][]): HolderTag[] => entries.map(([hour, tag]) => ({ hour, tag }));
const side = (holderTags: HolderTag[], firstHour: number, lastHour: number, parent: number | null = null, existedBefore = false,
  exit: SideExit | null = null): CivilWarSide => ({ tags: holderTags, firstHour, lastHour, parent, existedBefore, exit });
const war = (id: string, firstHour: number, lastHour: number, sides: CivilWarSide[], outcome: CivilWar['outcome'] = null,
  atWar: HourSpan[] = [{ firstHour, lastHour }]): CivilWar => ({ id, firstHour, lastHour, atWar, sides, outcome });
const reply = (wars: CivilWar[], lastHour: number | null = 1000, known: HourSpan[] = lastHour === null ? [] : [{ firstHour: 0, lastHour }]): CivilWarsReply =>
  ({ branchId: '1', lastHour, known, wars, warnings: [] });

const NAMES: Record<string, string> = { MOD: 'Modland', 'D01': 'Free Modland', 'D02': 'People’s Modland', 'D03': 'Modland Commune', REU: 'Reused' };
const text: LaneText = {
  side: (tag, hour, live) => ({ name: `${NAMES[tag] ?? tag}@${live ? 'live' : hour}`, known: true, color: '#123456' }),
  dated: (tag, hour) => ({ name: `${NAMES[tag] ?? tag}@${hour}`, known: true }),
  day: hour => `day ${hour}`, neutral: '#888888', positive: '#00aa00',
};
const tipText: TooltipText = { dated: text.dated, day: text.day, branchFirst: 0, unread: [] };
const all = (unread: readonly HourSpan[] = [], lastHour: number | null = 1000): CivilWarsOptions =>
  ({ ended: true, countryIds: null, identities: null, unread, lastHour });

const threeSided = war('w1', 100, 400, [
  side(tags([0, 'MOD']), 100, 400, null, true),
  side(tags([100, 'D01']), 100, 300, 0, false, { kind: 'annexedBySide', hour: 301, readHour: 305, by: 'MOD' }),
  side(tags([200, 'D02']), 200, 350, 1, false, { kind: 'annexedBySide', hour: 351, readHour: 360, by: 'MOD' }),
], { winner: 0, hour: 351, readHour: 360 });

test('a breakaway of a breakaway is indented under the side it broke from', () => {
  const data = validateCivilWarsReply(reply([threeSided]));
  assert.deepEqual(sideTree(data.wars[0].sides), [{ index: 0, depth: 1 }, { index: 1, depth: 2 }, { index: 2, depth: 3 }]);
  const view = civilWarsAt(data, 1000, all());
  const { lanes } = civilWarLanes(view.wars, new Set(), text, 0, []);
  assert.deepEqual(lanes.map(lane => [lane.identity?.label, lane.depth ?? 0, lane.heading === true]), [
    ['Civil war in Modland@100', 0, true], ['Modland@400', 1, false], ['Free Modland@300', 2, false], ['People’s Modland@350', 3, false]]);
  assert.equal(lanes[0].expanded, true);
  const collapsed = civilWarLanes(view.wars, new Set(['w1']), text, 0, []).lanes;
  assert.equal(collapsed.length, 1);
  assert.equal(collapsed[0].expanded, false);
  const tree = sideTree([side([], 0, 1), side([], 0, 1), side([], 0, 1, 0), side([], 0, 1, 2), side([], 0, 1, 0)]);
  assert.deepEqual(tree.map(item => [item.index, item.depth]), [[0, 1], [2, 2], [3, 3], [4, 2], [1, 1]]);
});

test('a winner only with a recorded civil-war outcome; otherwise Outcome unknown and nothing muted', () => {
  const data = validateCivilWarsReply(reply([threeSided]));
  const won = civilWarsAt(data, 1000, all()).wars[0];
  const { lanes } = civilWarLanes([won], new Set(), text, 0, []);
  assert.equal(lanes[0].label, 'Winner: Modland@351');
  assert.deepEqual(lanes[1].pill, { text: 'Winner', color: '#00aa00' });
  assert.deepEqual(lanes.slice(1).map(lane => lane.muted), [false, true, true]);
  const tip = warTooltip(won, 500, tipText);
  assert.equal(tip.rows.find(row => row.label === 'Outcome')?.value,
    'Winner: Modland@351 — Free Modland@300 and People’s Modland@350 annexed by a side by day 351');

  const unproven = validateCivilWarsReply(reply([war('w2', 100, 400, [
    side(tags([0, 'MOD']), 100, 400, null, true),
    side(tags([100, 'D01']), 100, 300, 0, false, { kind: 'annexedByOther', hour: 301, readHour: 301, by: 'REU' }),
    side(tags([200, 'D02']), 200, 350, 1, false, { kind: 'annexedBySideOtherwise', hour: 351, readHour: 351, by: 'MOD' }),
  ])]));
  const unknown = civilWarsAt(unproven, 1000, all()).wars[0];
  const lanesUnknown = civilWarLanes([unknown], new Set(), text, 0, []).lanes;
  assert.equal(lanesUnknown[0].label, 'Outcome unknown');
  assert.ok(lanesUnknown.every(lane => !lane.muted && !lane.pill));
  const unknownTip = warTooltip(unknown, 500, tipText);
  assert.equal(unknownTip.rows.find(row => row.label === 'Outcome')?.value, 'Unknown');
  assert.equal(unknownTip.note, 'No side was left after the others were annexed by a side.');
  const left = sideTooltip(unknown, unknown.sides[1], 320, { name: 'Free Modland' }, 'D01', tipText);
  assert.equal(left.rows.find(row => row.label === 'Annexed by')?.value, 'Reused@301, which was not a side');
  assert.equal(left.rows.find(row => row.label === 'Broke from')?.value, 'Modland@100 on day 100');
  assert.equal(left.rows.find(row => row.label === 'Tag')?.value, 'D01');
  assert.equal(exitText({ kind: 'annexedBySideOtherwise', hour: 1, readHour: 1, by: 'MOD' }, 'Modland'), 'Modland, not by the civil war’s own annexation');
  assert.equal(exitText({ kind: 'annexedBySide', hour: 1, readHour: 1, by: 'MOD' }, 'Modland'), 'Modland');
  const ongoing = civilWarsAt(reply([war('w3', 100, 1000, [side(tags([0, 'MOD']), 100, 1000, null, true), side(tags([100, 'D01']), 100, 1000, 0)])]), 1000, all()).wars[0];
  assert.equal(civilWarLanes([ongoing], new Set(), text, 0, []).lanes[0].label, 'Ongoing · 2 sides');
  assert.equal(warTooltip(ongoing, 500, tipText).rows.find(row => row.label === 'Outcome'), undefined);
});

test('dynamic and reused tags resolve per hour, never by a side’s final tag', () => {
  const data = validateCivilWarsReply(reply([war('w1', 100, 600, [
    side(tags([0, 'MOD'], [300, 'D03']), 100, 600, null, true),
    side(tags([100, 'D01'], [500, 'REU']), 100, 400, 0),
  ])]));
  const at = (hour: number) => civilWarLanes(civilWarsAt(data, hour, all()).wars, new Set(), text, 0, []).lanes.map(lane => lane.identity?.label);
  assert.deepEqual(at(200), ['Civil war in Modland@100', 'Modland@live', 'Free Modland@live']);
  assert.deepEqual(at(450), ['Civil war in Modland@100', 'Modland Commune@live', 'Free Modland@400']);
  assert.deepEqual(at(700), ['Civil war in Modland@100', 'Modland Commune@600', 'Free Modland@400']);
  const view = civilWarsAt(data, 450, all());
  const { lanes, info } = civilWarLanes(view.wars, new Set(), text, 0, []);
  assert.deepEqual(laneCountries(lanes, info), { live: ['D03'], dated: [{ hour: 100, tag: 'MOD' }, { hour: 400, tag: 'D01' }] });
  const ended = civilWarsAt(data, 700, all());
  const endedLanes = civilWarLanes(ended.wars, new Set(), text, 0, []);
  assert.deepEqual(laneCountries(endedLanes.lanes, endedLanes.info).dated, [{ hour: 100, tag: 'MOD' }, { hour: 600, tag: 'D03' }, { hour: 400, tag: 'D01' }]);
  const nameless = civilWarsAt(reply([war('w9', 100, 200, [side([], 100, 200, null, true), side(tags([150, 'D01']), 120, 200, 0)])]), 130, all());
  assert.equal(nameless.unnamed, 3);
  assert.deepEqual(civilWarLanes(nameless.wars, new Set(), text, 0, []).lanes.map(lane => lane.identity?.label),
    [`Civil war in ${NAME_NOT_KNOWN}`, NAME_NOT_KNOWN, NAME_NOT_KNOWN]);
});

test('Live reveals civil wars, sides and outcomes as playback reaches them, and seeking back removes them', () => {
  const data = validateCivilWarsReply(reply([threeSided]));
  const shown = (hour: number) => {
    const view = civilWarsAt(data, hour, all());
    return civilWarLanes(view.wars, new Set(), text, 0, []).lanes.map(lane => [lane.identity?.label, lane.pill?.text ?? lane.label, lane.muted ?? false]);
  };
  assert.deepEqual(shown(99), []);
  assert.deepEqual(shown(150), [['Civil war in Modland@100', 'Ongoing · 2 sides', false], ['Modland@live', 'Since day 100', false], ['Free Modland@live', 'Since day 100', false]]);
  assert.equal(shown(303)[2][1], 'Left day 301');
  assert.equal(shown(305)[2][1], 'Annexed day 301');
  assert.equal(shown(359)[0][1], 'Ongoing · 3 sides');
  assert.deepEqual(shown(360).map(row => row[2]), [false, false, true, true]);
  assert.equal(shown(360)[1][1], 'Winner');
  assert.deepEqual(shown(150).map(row => row[0]), ['Civil war in Modland@100', 'Modland@live', 'Free Modland@live']);
  assert.ok(shown(250).every(row => row[2] === false));
  const pending = validateCivilWarsReply(reply([war('w1', 100, 500, [side(tags([0, 'MOD']), 100, 500, null, true), side(tags([100, 'D01']), 100, 500, 0)])], 500));
  const later = civilWarsAt(pending, 800, all([{ firstHour: 501, lastHour: 900 }], 500)).wars[0];
  assert.equal(later.ongoing, true);
  assert.deepEqual(civilWarLanes([later], new Set(), text, 0, []).lanes.map(lane => lane.label), ['Ongoing · 2 sides', 'Since day 100', 'Since day 100']);
  assert.deepEqual(civilWarLanes([later], new Set(), text, 0, []).lanes.map(lane => lane.spans[0].openEnd), [true, true, true]);
});

test('ongoing civil wars come first, then the newest outbreak; ended ones can be hidden', () => {
  const data = validateCivilWarsReply(reply([
    war('old', 10, 50, [side(tags([0, 'MOD']), 10, 50, null, true), side(tags([10, 'D01']), 10, 50, 0)]),
    war('new', 200, 300, [side(tags([0, 'MOD']), 200, 300, null, true), side(tags([200, 'D02']), 200, 300, 0)]),
    war('going', 100, 1000, [side(tags([0, 'REU']), 100, 1000, null, true), side(tags([100, 'D03']), 100, 1000, 0)]),
  ]));
  const view = civilWarsAt(data, 1000, all());
  assert.deepEqual(view.wars.map(item => item.war.id), ['going', 'new', 'old']);
  assert.equal(summaryText(view), '3 civil wars · 1 ongoing');
  const hidden = civilWarsAt(data, 1000, { ...all(), ended: false });
  assert.deepEqual(hidden.wars.map(item => item.war.id), ['going']);
  assert.equal(hidden.total, 3);
  assert.deepEqual(jumpHours({ kind: 'war', war: view.wars[1] }), [{ label: 'the outbreak', hour: 200 }, { label: 'the end', hour: 300 }]);
  assert.deepEqual(jumpHours({ kind: 'side', war: view.wars[0], side: view.wars[0].sides[1] }), [{ label: 'the outbreak', hour: 100 }, { label: 'the split', hour: 100 }]);
  assert.deepEqual(civilWarsAt(data, 250, { ...all(), from: 60 }).wars.map(item => item.war.id), ['new', 'going']);
});

test('a listing the game no longer counts as a civil war is not ongoing, though still listed', () => {
  const sides = [side(tags([0, 'MOD']), 100, 1000, null, true), side(tags([100, 'D01']), 100, 1000, 0)];
  const data = validateCivilWarsReply(reply([war('stale', 100, 1000, sides, null, [{ firstHour: 100, lastHour: 500 }])]));
  const at = (hour: number, ended = true) => civilWarsAt(data, hour, { ...all(), ended });
  assert.equal(at(300).wars[0].ongoing, true);
  const stale = at(1000);
  assert.equal(stale.wars[0].ongoing, false);
  assert.equal(summaryText(stale), '1 civil war · 0 ongoing');
  assert.equal(civilWarLanes(stale.wars, new Set(), text, 0, []).lanes[0].label, 'Outcome unknown');
  assert.equal(warTooltip(stale.wars[0], 1000, tipText).rows.find(row => row.label === 'Ended')?.value, 'day 501');
  assert.deepEqual(jumpHours({ kind: 'war', war: stale.wars[0] }).at(-1), { label: 'the end', hour: 500 });
  assert.deepEqual(at(1000, false).wars, [], 'hidden with the ended ones');
  const never = civilWarsAt(validateCivilWarsReply(reply([war('never', 100, 1000, sides, null, [])])), 300, all()).wars[0];
  assert.equal(never.ongoing, false);
  assert.equal(warTooltip(never, 300, tipText).rows.some(row => row.label === 'Ended'), false);
});

test('hours the civil-war lists were not read are unknown: a listing ending there did not end then', () => {
  const sides = [side(tags([0, 'MOD']), 100, 100, null, true), side(tags([100, 'D01']), 100, 100, 0)];
  const data = validateCivilWarsReply(reply([war('w', 100, 100, sides)], 200, [{ firstHour: 0, lastHour: 100 }]));
  const unread = unknownHours([], data.known, 0, data.lastHour);
  assert.deepEqual(unread, [{ firstHour: 101, lastHour: 200 }]);
  const view = civilWarsAt(data, 150, all(unread, data.lastHour));
  assert.equal(view.wars[0].ongoing, true, 'not known to have ended');
  const lanes = civilWarLanes(view.wars, new Set(), text, 0, unread).lanes;
  assert.deepEqual(lanes.map(lane => lane.label), ['Ongoing · 2 sides', 'Last recorded day 100', 'Last recorded day 100']);
  assert.equal(warTooltip(view.wars[0], 150, { ...tipText, unread }).rows.find(row => row.label === 'Ended')?.value, 'Not recorded after day 100');
  const reread = unknownHours([], [{ firstHour: 0, lastHour: 100 }, { firstHour: 151, lastHour: 200 }], 0, 200);
  const after = civilWarsAt(data, 170, all(reread, 200)).wars[0];
  assert.equal(after.ongoing, false);
  assert.equal(warTooltip(after, 170, { ...tipText, unread: reread }).rows.find(row => row.label === 'Ended')?.value, 'by day 151');
});

test('chosen countries narrow to civil wars they were a side in, through aliases and not reused tags', () => {
  const holder = (tag: string, runs: [number, number, number][]) => ({ tag, identities: runs.map(([slot, firstHour, lastHour]) => ({ slot, countryId: slot, firstHour, lastHour })) });
  const identities = countryIdentityIndex([holder('MOD', [[1, 0, 299]]), holder('D03', [[1, 300, 1000]]), holder('D01', [[3, 0, 1000]]),
    holder('REU', [[2, 0, 99], [5, 100, 1000]]), holder('D02', [[4, 0, 1000]])]);
  const data = validateCivilWarsReply(reply([
    war('a', 400, 500, [side(tags([300, 'D03']), 400, 500, null, true), side(tags([400, 'D01']), 400, 500, 0)]),
    war('b', 200, 250, [side(tags([0, 'REU']), 200, 250, null, true), side(tags([200, 'D02']), 200, 250, 0)]),
  ]));
  const narrowed = (chosen: string[]) => {
    const filter = worldFilter({ kind: 'tags', tags: chosen }, [], undefined, 1000, identities);
    return civilWarsAt(data, 1000, { ...all(), countryIds: filter.countryIds ?? null, identities });
  };
  assert.deepEqual(narrowed(['MOD']).wars.map(item => item.war.id), ['a']);
  assert.deepEqual(narrowed(['REU']).wars.map(item => item.war.id), ['b']);
  const onlyFirst = civilWarsAt(data, 1000, { ...all(), countryIds: new Set([JSON.stringify([2, 2])]), identities });
  assert.deepEqual(onlyFirst.wars, []);
  assert.equal(onlyFirst.reached, 2);
  assert.equal(onlyFirst.total, 0);
});

test('malformed replies are refused', () => {
  assert.throws(() => validateCivilWarsReply({ branchId: '1', lastHour: 1, warnings: [] }));
  const loop = war('w', 0, 10, [side([], 0, 10, 1), side([], 0, 10, 0)]);
  assert.throws(() => validateCivilWarsReply(reply([loop])));
  assert.throws(() => validateCivilWarsReply(reply([war('w', 0, 10, [side([], 0, 10)], { winner: 3, hour: 1, readHour: 1 })])));
  assert.throws(() => validateCivilWarsReply(reply([war('w', 5, 10, [side([], 0, 10)])])));
  assert.throws(() => validateCivilWarsReply(reply([war('w', 0, 10, [side([], 0, 10, null, false, { kind: 'peace' as never, hour: 1, readHour: 1, by: null })])])));
  assert.throws(() => validateCivilWarsReply(reply([war('w', 0, 10, [side([], 0, 10)], null, [{ firstHour: 5, lastHour: 11 }])])), 'predicate hours outside the listings');
  assert.throws(() => validateCivilWarsReply({ ...reply([]), known: [{ firstHour: 5, lastHour: 1 }] }));
});

test('thousands of civil wars among 5,000 countries are all listed; only lanes in view read names', () => {
  const wars: CivilWar[] = [];
  for (let index = 0; index < 2_500; index++) {
    const base = index * 2;
    wars.push(war(`w${index}`, index, index + 50, [
      side(tags([0, `C${base}`]), index, index + 50, null, true),
      side(tags([index, `C${base + 1}`]), index, index + 40, 0),
      side(tags([index + 10, `C${(base + 2) % 5_000}`]), index + 10, index + 50, 1),
    ], index % 2 ? { winner: 0, hour: index + 50, readHour: index + 50 } : null));
  }
  const data = validateCivilWarsReply(reply(wars, 10_000));
  const view = civilWarsAt(data, 10_000, all([], 10_000));
  assert.equal(view.wars.length, 2_500);
  assert.equal(summaryText(view), '2,500 civil wars · 0 ongoing');
  const { lanes, info } = civilWarLanes(view.wars, new Set(), text, 0, []);
  assert.equal(lanes.length, 10_000);
  const requests = laneRequests(lanes, [0, 30]);
  const wanted = laneCountries(requests.looks, info);
  assert.ok(wanted.dated.length + wanted.live.length < 100);
  assert.ok(tooltipCountries(info.get(lanes[0].id)!).length <= 5);
});

test('the timeline renders only where a war, a side, a tag, an exit, an outcome or a stretch not read changes', () => {
  const hours = changeHours(reply([threeSided]), [{ firstHour: 500, lastHour: 509 }], [{ hour: 250 }]);
  for (const hour of [100, 200, 250, 301, 305, 351, 360, 401, 500, 510]) assert.ok(hours.includes(hour), `${hour} is a change`);
  assert.ok(hours.every((hour, index) => index === 0 || hours[index - 1] < hour), 'ascending and distinct');
  assert.equal(changesBy(hours, 201), changesBy(hours, 249));
  assert.notEqual(changesBy(hours, 300), changesBy(hours, 301));
});

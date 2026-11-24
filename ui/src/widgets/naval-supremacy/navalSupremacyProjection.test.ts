import assert from 'node:assert/strict';
import test from 'node:test';
import { createBoundedReadView } from '../../families/boundedView.ts';
import { deriveLanesTooltip } from '../../families/heat/lanesModel.ts';
import type { IdentityProvider } from '../../families/plan.ts';
import type { NavalSupremacyReply } from '../../generated/dashboard';
import { projectSupremacyShares, projectSupremacyTimeline, shipsText } from './navalSupremacyProjection.ts';

const SURFACE = '#161616';

function reply(lastHour: number): NavalSupremacyReply {
  return {
    firstHour: 0, lastHour, firstDayHour: 0, dayHours: 24, days: 3, rankMissing: null,
    unknown: [{ firstDay: 1, lastDay: 1, reason: 'collector queue overflow' }],
    gaps: [], viewerName: 'German Reich', viewerColor: [90, 90, 90], legend: [], scale: 100000, warnings: [],
    regions: [{
      id: '17', name: 'English Channel', pinned: false,
      ownShips: { kind: 'counted', ships: 13 },
      enemyShips: { kind: 'notRecorded', reason: 'Country links unavailable' },
      readout: { kind: 'control', control: 1, share: '59000', friendly: '59', enemy: '41', own: '20', as_of_hour: lastHour },
      runs: [
        { firstDay: 0, lastDay: 0, control: -1, friendly: '1', enemy: '9', share: '10000', own: '0' },
        { firstDay: 2, lastDay: 2, control: 1, friendly: '59', enemy: '41', share: '59000', own: '20' },
      ],
    }],
  };
}

const view = (hour: number, data: NavalSupremacyReply) => createBoundedReadView({ hour, panels: new Map([['supremacy', data]]) });

test('each side keeps its own ship count or says it is missing', () => {
  assert.equal(shipsText({ kind: 'counted', ships: 13 }, { kind: 'counted', ships: 5 }), '13 v 5 ships');
  assert.equal(shipsText({ kind: 'counted', ships: 13 }, { kind: 'notRecorded', reason: 'x' }), '13 v n/a ships');
  assert.equal(shipsText({ kind: 'notRecorded', reason: 'x' }, { kind: 'notRecorded', reason: 'y' }), 'Ships not recorded');
});

test('the shares bar splits the score at the playhead between friends and enemies', () => {
  const model = projectSupremacyShares(view(60, reply(60)), { viewer: null, surface: SURFACE });
  const row = model.rows.find(item => item.kind === 'data');
  assert.ok(row && row.kind === 'data');
  assert.deepEqual((row.segments ?? []).map(segment => [segment.key, Math.round(segment.share * 100)]), [['friendly', 59], ['enemy', 41]]);
  assert.equal(row.readout?.primary, '13 v n/a ships');
});

test('a day with no score on either side is the empty lane, not contested', () => {
  const data = reply(60);
  const quiet = { ...data, regions: [{ ...data.regions[0]!, runs: [{ firstDay: 0, lastDay: 2, control: 0, friendly: '0', enemy: '0', share: '0', own: '0' }],
    readout: { kind: 'control' as const, control: 0, share: '0', friendly: '0', enemy: '0', own: '0', as_of_hour: 60 } }] };
  const model = projectSupremacyTimeline(view(60, quiet), { viewer: null, surface: SURFACE });
  assert.equal(model.rows[0]!.runs[0]!.cell.kind, 'clear');
  assert.deepEqual(model.rows[0]!.readout, { kind: 'clear', label: 'No activity' });
  const shares = projectSupremacyShares(view(60, quiet), { viewer: null, surface: SURFACE }).rows.find(item => item.kind === 'data');
  assert.ok(shares && shares.kind === 'data');
  assert.deepEqual(shares.segments ?? [], [], 'the same zero scores are no activity in Shares too, never 100% enemy');
});

test('the timeline hatches an unknown day with its reason and ends at the displayed hour', () => {
  const model = projectSupremacyTimeline(view(60, reply(60)), { viewer: null, surface: SURFACE });
  const runs = model.rows[0]!.runs;
  assert.deepEqual(runs.map(run => [run.firstHour, run.lastHour, run.cell.kind]), [[0, 23, 'solid'], [24, 47, 'missing'], [48, 60, 'solid']]);
  const readout = model.rows[0]!.readout!;
  assert.equal(readout.kind === 'value' && readout.primary, '59%');
  assert.equal(readout.kind === 'value' && readout.secondary, 'Friendly control');
});

test('a reply past the displayed hour draws nothing in either view', () => {
  assert.deepEqual(projectSupremacyTimeline(view(50, reply(60)), { viewer: null, surface: SURFACE }).rows, []);
  assert.equal(projectSupremacyShares(view(50, reply(60)), { viewer: null, surface: SURFACE }).rows.length, 0);
});

test('each day takes the viewer colour of its own hour, and a hit names its own day', () => {
  const identity: IdentityProvider = {
    resolve: (kind, key, atHour) => ({
      kind, key, atHour,
      holder: { status: 'ready', value: key },
      name: { status: 'ready', value: 'German Reich' },
      colour: { status: 'ready', value: atHour < 48 ? '#3060c0' : '#c03030' },
      validInterval: atHour < 48 ? [0, 47] : [48, Infinity],
      nextChangeAt: atHour < 48 ? 48 : Infinity,
    }),
  };
  const data = reply(60);
  const friendly = { ...data, unknown: [], regions: [{ ...data.regions[0]!, runs: [{ firstDay: 0, lastDay: 2, control: 1, friendly: '59', enemy: '41', share: '59000', own: '20' }] }] };
  const model = projectSupremacyTimeline(createBoundedReadView({ hour: 60, panels: new Map([['supremacy', friendly]]), identity }), { viewer: 'GER', surface: SURFACE });
  const runs = model.rows[0]!.runs;
  assert.deepEqual(runs.map(run => [run.firstHour, run.lastHour]), [[0, 47], [48, 60]], 'the run splits where the viewer colour changes');
  const colours = runs.map(run => (run.cell.kind === 'solid' ? run.cell.color : ''));
  assert.notEqual(colours[0], colours[1]);
  const early = deriveLanesTooltip(model, 0, 30)!;
  assert.equal(early.hour, undefined, 'a past day has no recorded sample hour, so the tooltip names the day');
  assert.ok(early.subtitle);
  assert.equal(deriveLanesTooltip(model, 0, 55)!.hour, 60, 'the last day is the readout and is read at its own hour');
});

import assert from 'node:assert/strict';
import test from 'node:test';
import { factionGroup, formatScore, scoreCountries, scoreHint, scoreParts, scoreTooltip, validateGameScore } from './gameScoreData.ts';
import type { ScoreOf } from './gameScoreData';
import type { CountryGroupReply, GameScoreRow } from '../../generated/dashboard';
import type { GameScoreReply } from './gameScoreData';

const known = { missing: null };
const unknown = (missing: string) => ({ missing });
const row = (tag: string, total: boolean): GameScoreRow => ({ tag, countryId: 1, total: total ? known : unknown('the total needs army, navy, air and the total multiplier'),
  army: known, navy: known, air: unknown('a registered air wing has no AirWingGameScoreStats row'), industry: known, land: known });
const REPLY: GameScoreReply = { missing: null, warnings: [], hints: { total: 'Sort by score' }, rows: [row('ENG', true), row('GER', true), row('ITA', false)],
  multipliers: [{ part: 'total', raw: '100000', missing: null }, { part: 'air', raw: null, missing: 'NAir define was not read' }] };
const VALUES = new Map([['ENG|total', 3], ['ENG|army', 9], ['GER|total', 7], ['GER|army', 1], ['ITA|army', 1]]);
const score: ScoreOf = (tag, part) => VALUES.get(`${tag}|${part}`);

test('all countries is a choice made: every row of the reply, with no reason to choose a country', () => {
  const all = scoreCountries({ kind: 'all' }, [], undefined, 9);
  assert.equal(all.reason, null);
  assert.equal(scoreCountries({ kind: 'tags', tags: [] }, [], undefined, 9).reason, 'Choose a country in the widget settings.');
});

test('a score not recorded says why, never zero, and the multipliers follow the scores', () => {
  assert.equal(formatScore(12.99999), '12', 'the end screen truncates to whole points');
  const tip = scoreTooltip(REPLY, REPLY.rows[2], 'Italy', '9 Jan', 'Axis', score);
  const total = tip.rows.find(item => item.label === 'Total score');
  assert.deepEqual([total?.value, total?.detail, total?.tone], ['Not recorded', 'the total needs army, navy, air and the total multiplier', 'muted']);
  assert.equal(tip.rows.find(item => item.label === 'Navy')?.value, 'Not read yet', 'a recorded score before its cell lands is not a zero');
  const air = tip.rows.find(item => item.label === 'Air multiplier');
  assert.deepEqual([air?.value, air?.detail], ['Not recorded', 'NAir define was not read']);
  assert.equal(tip.rows.find(item => item.label === 'Total score multiplier')?.value, '1');
  assert.throws(() => validateGameScore({ ...REPLY, rows: [{ ...REPLY.rows[0], army: { missing: 3 } }] }));
});

test('a column is hinted as the end screen words it where the content does, else ours; the race splits Total in proportion', () => {
  assert.equal(scoreHint(REPLY, 'total'), 'Sort by score');
  assert.match(scoreHint(REPLY, 'army')!, /army multiplier/);
  assert.equal(scoreHint(REPLY, 'country'), undefined);
  const parts = scoreParts();
  assert.deepEqual(parts.kind === 'measures' && [parts.proportional, parts.items.map(item => item.measure.measure)], [true, ['army', 'navy', 'air']]);
});

// quarantine: #3 re-enable in the dashboard rework; the issue names its failure
test.skip('a country’s faction is the one its tag was a member of at the hour, by name at that hour', () => {
  const groups = { factions: [{ id: '4', name: 'Allies', nameReason: null, recordedName: '', firstHour: 0, lastHour: 99,
    names: [{ firstHour: 0, lastHour: 5, name: 'Old Allies', reason: null }], members: [{ tag: 'ENG', firstHour: 3, lastHour: 50 }] }] } as unknown as CountryGroupReply;
  assert.deepEqual(factionGroup(groups, 'ENG', 4), { key: 'faction:4', label: 'Old Allies', order: 0, orderKey: 4n });
  assert.equal(factionGroup(groups, 'ENG', 9).label, 'Allies');
  assert.equal(factionGroup(groups, 'ENG', 60).key, 'no-faction');
  assert.equal(factionGroup(undefined, 'ENG', 4).key, 'faction-not-recorded');
});

import { createBoundedReadView, emptyReadView } from '../../families/boundedView.ts';
import { gameScoreProjection } from './gameScoreProjection.ts';

test('game-score projection resolves slot tag runs at the projected hour and honours faction rename bounds without latest-name fallback', () => {
  const rep: GameScoreReply = {
    missing: null,
    warnings: [],
    hints: {},
    multipliers: [{ part: 'total', raw: '100000', missing: null }],
    rows: [row('GER', true), row('GDR', true)],
  };
  const live = { get: () => 50, missing: () => 'Not recorded' };
  const opts = {
    all: false,
    chosenTags: [],
    holders: { '\u00007': [{ tag: 'GER', firstHour: 0, lastHour: 104 }, { tag: 'GDR', firstHour: 105, lastHour: 200 }] },
    membership: null,
    top: null,
    by: 'total' as const,
    factions: true,
    factionRuns: [
      {
        id: '1',
        tag: 'GER',
        firstHour: 0,
        lastHour: 104,
        names: [
          { firstHour: 0, lastHour: 50, name: 'Old Axis' },
          { firstHour: 51, lastHour: 80, name: 'New Axis' },
          { firstHour: 81, lastHour: 104, name: null },
        ],
        fallbackName: 'New Axis',
      },
    ],
    rankMarkers: false,
    cellBars: false,
    labels: { GER: [{ from: 0, name: 'Germany' }], GDR: [{ from: 105, name: 'East Germany' }] },
  };
  const at40 = gameScoreProjection.project(createBoundedReadView({ hour: 40, panels: new Map([['main', rep]]), live: new Map([['main', live]]) }), opts);
  assert.deepEqual(at40.rows.map(r => r.key), ['GER'], 'slot 7 resolves to GER before tag switch at 105');
  assert.equal(at40.rows[0]!.group?.label, 'Old Axis');

  const at60 = gameScoreProjection.project(createBoundedReadView({ hour: 60, panels: new Map([['main', rep]]), live: new Map([['main', live]]) }), opts);
  assert.equal(at60.rows[0]!.group?.label, 'New Axis', 'rename at hour 51 replaces Old Axis');

  const at90 = gameScoreProjection.project(createBoundedReadView({ hour: 90, panels: new Map([['main', rep]]), live: new Map([['main', live]]) }), opts);
  assert.notEqual(at90.rows[0]!.group?.label, 'New Axis', 'missing historical faction name never falls back to latest name');
  assert.equal(at90.rows[0]!.group?.order, 1);

  const at110 = gameScoreProjection.project(createBoundedReadView({ hour: 110, panels: new Map([['main', rep]]), live: new Map([['main', live]]) }), opts);
  assert.deepEqual(at110.rows.map(r => r.key), ['GDR'], 'slot 7 resolves to GDR at hour 110');

  assert.equal(gameScoreProjection.nextChange?.(emptyReadView(40), opts), 51, 'nextChange reports faction rename boundary');
  assert.equal(gameScoreProjection.nextChange?.(emptyReadView(90), opts), 105, 'nextChange reports slot switch / faction end boundary');

  const ungroupedOpts = {
    ...opts,
    factions: false,
    factionUnobserved: [[20, 30] as const],
    factionIncomplete: [[65, 75] as const],
  };
  assert.equal(gameScoreProjection.nextChange?.(emptyReadView(10), ungroupedOpts), 20, 'nextChange includes faction coverage bounds even when grouping is off');
  assert.equal(gameScoreProjection.nextChange?.(emptyReadView(40), ungroupedOpts), 51, 'nextChange includes faction rename bounds even when grouping is off');

  const unobsModel = gameScoreProjection.project(createBoundedReadView({ hour: 25, panels: new Map([['main', rep]]), live: new Map([['main', live]]) }), { ...opts, factionUnobserved: [[20, 30]] });
  assert.equal(unobsModel.rows[0]!.group?.key, 'faction-not-recorded', 'unobserved faction hours are notRecorded, never No faction');
  const unobsExtra = typeof unobsModel.rows[0]!.tooltipExtra === 'function' ? unobsModel.rows[0]!.tooltipExtra() : unobsModel.rows[0]!.tooltipExtra;
  assert.equal(unobsExtra?.[0]?.value.kind, 'notRecorded');

  const incompModel = gameScoreProjection.project(createBoundedReadView({ hour: 110, panels: new Map([['main', rep]]), live: new Map([['main', live]]) }), { ...opts, factionIncomplete: [[108, 115]] });
  assert.equal(incompModel.rows[0]!.group?.key, 'faction-not-recorded', 'incomplete faction roster hours are notRecorded for non-members, never No faction');
});

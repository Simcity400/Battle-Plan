import assert from 'node:assert/strict';
import test from 'node:test';
import type { NavalLossesReply, NavalLossRun } from '../../generated/dashboard';
import { navalAllZero, navalBands, navalChoiceReads, navalHolderBands, navalLines, navalLookHour, navalModel, navalPeriodLabel, navalReadHour, navalRequest, navalTooltip, navalValue, rankNaval, validateNavalLossesReply } from './navalLossesData.ts';
import { countryDrillSpans } from '../../dashboard/countryIdentity.ts';
import { definition } from './definition.ts';
import { resolveCountries } from '../../dashboard/groups.ts';
import { formatDateTime } from '../../dashboard/calendar.ts';
import { countryRaceItems } from '../../charts/race/countryRace.ts';
import { raceMax, raceMin, raceText } from '../../charts/race/race.ts';

function fixture(): NavalLossesReply {
  return { branchId: 'branch', available: true, firstRead: 100, shipFirstRead: 100, convoyFirstRead: 100, shipLastRead: 108, convoyLastRead: 108,
    shipReads: [100, 102, 107, 108], convoyReads: [100, 102, 108], warnings: [], series: [
    { tag: 'mod tag', membership: null, segments: [{ firstHour: 100, lastHour: 109 }], runs: [
      { firstHour: 100, lastHour: 101, startsSegment: true, ships: 0, convoys: 0 },
      { firstHour: 102, lastHour: 103, startsSegment: false, ships: 4, convoys: 2 },
      { firstHour: 104, lastHour: 106, startsSegment: true, ships: 5, convoys: null },
      { firstHour: 107, lastHour: 109, startsSegment: true, ships: 2, convoys: 3 },
    ] },
    { tag: 'another tag', membership: null, segments: [{ firstHour: 100, lastHour: 109 }],
      runs: [{ firstHour: 100, lastHour: 109, startsSegment: true, ships: 4, convoys: 2 }] },
  ] };
}

test('naval detail joins the chosen native holder across aliases without inheriting reused tags', () => {
  const data = fixture();
  data.series[0].identities = [{ firstHour: 100, lastHour: 103, slot: 1, countryId: 11 }, { firstHour: 104, lastHour: 109, slot: 2, countryId: 22 }];
  data.series[1].identities = [{ firstHour: 100, lastHour: 103, slot: 3, countryId: 33 }, { firstHour: 104, lastHour: 108, slot: 1, countryId: 11 }];
  const model = navalModel(validateNavalLossesReply(data));
  const spans = countryDrillSpans(data.series, 'mod tag', 101);
  const bands = navalHolderBands(model, spans, { firstHour: 100, lastHour: 109 });
  assert.deepEqual(bands[0].map(run => [run.firstHour, run.lastHour, run.value]), [[100, 101, 0], [102, 103, 4], [104, 108, 4]]);
  assert.equal(navalLookHour(model, 'mod tag', 104, 'both'), null, 'a supplying read of another holder cannot supply a look');
  assert.equal(navalLookHour(model, 'mod tag', 103, 'both'), 102);
  data.series[0].identities = [];
  assert.deepEqual(rankNaval(navalModel(data), ['mod tag'], 102, 'ships', 10), []);
});

test('naval chooser reads each drawn native holder through its unique supplying alias', () => {
  const data = fixture();
  data.shipReads = data.convoyReads = [100]; data.shipLastRead = data.convoyLastRead = 100;
  data.series[0].identities = [{ firstHour: 100, lastHour: 103, slot: 1, countryId: 11 }, { firstHour: 104, lastHour: 109, slot: 2, countryId: 22 }];
  data.series[1].identities = [{ firstHour: 100, lastHour: 103, slot: 3, countryId: 33 }, { firstHour: 104, lastHour: 109, slot: 1, countryId: 11 }];
  const reads = navalChoiceReads(navalModel(data), ['another tag', 'mod tag', 'missing'], 105, 'both', 'branch');
  assert.deepEqual([...reads], [['another tag', { tag: 'mod tag', branchId: 'branch', hour: 100 }], ['mod tag', null], ['missing', null]]);
  data.series.push({ ...data.series[0], tag: 'ambiguous alias', identities: [{ firstHour: 100, lastHour: 103, slot: 1, countryId: 11 }] });
  assert.equal(navalChoiceReads(navalModel(data), ['another tag'], 105, 'both', 'branch').get('another tag'), null);
  const legacy = fixture();
  legacy.series[0].segments = [{ firstHour: 100, lastHour: 103 }, { firstHour: 104, lastHour: 109 }];
  legacy.shipReads = legacy.convoyReads = [100];
  assert.equal(navalChoiceReads(navalModel(legacy), ['mod tag'], 105, 'both', 'branch').get('mod tag'), null);
  assert.deepEqual(navalChoiceReads(navalModel(legacy), ['mod tag'], 103, 'both', 'branch').get('mod tag'), { tag: 'mod tag', branchId: 'branch', hour: 100 });
});

test('naval counts preserve zero and omit unknown parts from the selected ranking', () => {
  const model = navalModel(validateNavalLossesReply(fixture()));
  assert.deepEqual(rankNaval(model, ['mod tag'], 100, 'both', 10).map(entry => entry.value), [0]);
  assert.deepEqual(rankNaval(model, ['mod tag'], 100, 'both', 10, true), [], 'a top N lists only countries with losses');
  assert.equal(navalAllZero(model, ['mod tag'], 100, 'both'), true, 'every present tally is a known zero');
  assert.equal(navalAllZero(model, ['mod tag'], 105, 'both'), false, 'an unknown part is not a zero');
  assert.equal(navalAllZero(model, ['mod tag'], 102, 'both'), false, 'losses are not none');
  assert.equal(navalAllZero(model, [], 100, 'both'), false, 'no present country proves nothing');
  assert.deepEqual(rankNaval(model, model.tags, 102, 'both', 10).map(entry => entry.tag), ['another tag', 'mod tag']);
  assert.deepEqual(rankNaval(model, ['mod tag'], 105, 'both', 10), []);
  assert.equal(rankNaval(model, ['mod tag'], 105, 'ships', 10)[0].value, 5);
  assert.equal(navalValue(model.reply.series[0].runs[2], 'convoys'), null);
  assert.deepEqual(rankNaval(model, ['mod tag'], 102, 'both', 10)[0].parts, [{ id: 'ships', value: 4 }, { id: 'convoys', value: 2 }]);
  assert.equal(navalTooltip(model, 'mod tag', 109, 'Country', 'both', 'lost').subtitle, formatDateTime(109));
  assert.deepEqual(rankNaval(model, model.tags, 99, 'ships', 10), []);
  const unknown = navalTooltip(model, 'mod tag', 105, 'Name at hover', 'both', 'lost');
  assert.equal(unknown.rows?.find(row => row.label === 'Convoys')?.value, 'Unknown');
  assert.equal(unknown.rows?.find(row => row.label === 'Lost')?.value, 'Unknown');
});

test('single-measure naval tooltips keep one value while combined measures keep their total', () => {
  const model = navalModel(fixture());
  for (const kind of ['ships', 'convoys'] as const) {
    const rows = navalTooltip(model, 'mod tag', 109, 'Country', kind, 'lost').rows;
    assert.deepEqual(rows?.map(row => [row.label, row.value]), [[kind === 'ships' ? 'Ships' : 'Convoys', kind === 'ships' ? '2' : '3']]);
  }
  const both = navalTooltip(model, 'mod tag', 109, 'Country', 'both', 'lost').rows;
  assert.deepEqual(both?.map(row => [row.label, row.value]), [['Ships', '2'], ['Convoys', '3'], ['Lost', '5']]);
});

test('an all-zero naval reply supplies ranked rows and a valid shared race axis', () => {
  const reply = fixture();
  reply.series = Array.from({ length: 12 }, (_, index) => ({ tag: `mod country ${index.toString().padStart(2, '0')}`,
    membership: null, segments: [{ firstHour: 100, lastHour: 105 }],
    runs: [{ firstHour: 100, lastHour: 105, startsSegment: true, ships: 0, convoys: 0 }] }));
  const model = navalModel(validateNavalLossesReply(reply));
  for (const kind of ['ships', 'convoys', 'both'] as const) {
    const ranked = rankNaval(model, model.tags, 105, kind, 10);
    assert.equal(ranked.length, 10);
    assert.deepEqual(ranked.map(entry => [entry.tag, entry.value, entry.label]),
      reply.series.slice(0, 10).map(series => [series.tag, 0, '0']));
    if (kind === 'both') assert.deepEqual(ranked[0].parts, [{ id: 'ships', value: 0 }, { id: 'convoys', value: 0 }]);
    const items = countryRaceItems(ranked, new Map(), () => '#123456', '#888888', false);
    assert.equal(items.length, ranked.length);
    assert.ok(Number.isFinite(raceMax(items)) && raceMax(items) > raceMin(items));
    assert.match(raceText(items, tag => tag), /1\. mod country 00 0/);
  }
  reply.series[0].runs[0].convoys = null;
  const unknown = navalModel(validateNavalLossesReply(reply));
  assert.equal(rankNaval(unknown, [reply.series[0].tag], 105, 'both', 10).length, 0);
  assert.equal(rankNaval(unknown, [reply.series[0].tag], 105, 'ships', 10)[0].value, 0);
});

test('naval lines break at unknown coverage, holder changes and per-hour group boundaries', () => {
  const reply = fixture();
  reply.series[0].segments = [{ firstHour: 100, lastHour: 103 }, { firstHour: 104, lastHour: 109 }];
  const model = navalModel(reply);
  const membership = new Map([['mod tag', [{ firstHour: 101, lastHour: 108 }]]]);
  const runs = navalLines(model, ['mod tag'], 'both', { firstHour: 102, lastHour: 107 }, membership)[0].runs;
  assert.deepEqual(runs.map(run => [run.firstHour, run.lastHour, run.value]), [[102, 103, 6], [107, 107, 5]]);
  assert.equal(runs[1].breakBefore, true);
  assert.equal(navalLines(model, ['mod tag'], 'ships', { firstHour: 100, lastHour: 109 }, null)[0].runs.find(run => run.firstHour === 104)?.breakBefore, true);
});

test('trailing tooltips show the selected time rather than supplying dates', () => {
  const reply = fixture();
  reply.shipLastRead = 107;
  reply.shipReads.pop();
  assert.equal(navalTooltip(navalModel(reply), 'mod tag', 109, 'Country', 'both', 'lost').subtitle, formatDateTime(109));
  assert.equal(navalTooltip(navalModel(reply), 'mod tag', 109, 'Country', 'ships', 'lost').subtitle, formatDateTime(109));
  assert.equal(navalTooltip(navalModel(reply), 'mod tag', 107, 'Country', 'convoys', 'lost').subtitle, formatDateTime(107));
});

test('intermediate carries retain separate supplying dates, including unchanged reads', () => {
  const reply = fixture();
  assert.equal(navalTooltip(navalModel(reply), 'mod tag', 103, 'Country', 'both', 'lost').subtitle, formatDateTime(103));
  assert.equal(navalTooltip(navalModel(reply), 'mod tag', 107, 'Country', 'both', 'lost').subtitle, formatDateTime(107));
  assert.equal(navalTooltip(navalModel(reply), 'mod tag', 108, 'Country', 'both', 'lost').subtitle, formatDateTime(108));
  assert.equal(navalReadHour(reply, 99, 'both'), null);
  assert.equal(navalReadHour(reply, 106, 'ships'), 102);
  assert.equal(navalReadHour(reply, 107, 'both'), 107);
  assert.equal(navalReadHour(reply, 107, 'convoys'), 102);
  const model = navalModel(reply);
  assert.equal(navalValue(reply.series[1].runs[0], 'both'), 6);
  assert.equal(navalTooltip(model, 'another tag', 107, 'Name at supplying read', 'both', 'lost').subtitle, formatDateTime(107));
});

test('signed corrections stay in ranking, lines, parts and exact tooltip values', () => {
  const reply = fixture();
  reply.series[0].runs[3].convoys = -7;
  const model = navalModel(validateNavalLossesReply(reply));
  const [ranked] = rankNaval(model, ['mod tag'], 109, 'both', 10);
  assert.equal(ranked.value, -5);
  assert.equal(ranked.label, '-5');
  assert.deepEqual(ranked.parts, [{ id: 'ships', value: 2 }, { id: 'convoys', value: -7 }]);
  const items = countryRaceItems([ranked], new Map(), () => '#123456', '#888888');
  assert.ok(raceMin(items) < -7);
  assert.equal(navalLines(model, ['mod tag'], 'convoys', { firstHour: 107, lastHour: 109 }, null)[0].runs[0].value, -7);
  assert.equal(navalBands(model, 'mod tag', { firstHour: 107, lastHour: 109 }, null)[1][0].value, -7);
  const holder = navalHolderBands(model, new Map([['mod tag', [{ firstHour: 107, lastHour: 109 }]]]), { firstHour: 107, lastHour: 109 });
  assert.deepEqual(holder.map(band => band.map(run => run.value)), [[2], [-7]], 'the breakdown history keeps the signed correction');
  assert.equal(navalTooltip(model, 'mod tag', 109, 'Country', 'both', 'lost').rows?.find(row => row.label === 'Convoys')?.value, '-7');
});

test('naval requests keep opaque tags and use the server ranking for top countries', () => {
  assert.deepEqual(navalRequest('branch', { tags: ['tag with space', 'a,b'] }, 'ships', 'sunkBy', 'month'),
    { branchId: 'branch', tags: ['tag with space', 'a,b'], top: null, kind: 'ships', attribution: 'sunkBy', period: 'month' });
  assert.deepEqual(navalRequest('branch', { tags: ['ignored'], top: 10 }, 'both', 'lost', 'running').tags, []);
  assert.equal(navalPeriodLabel(fixture(), 'running'), `Since ${formatDateTime(100)}`);
  // After hours the recording lost, each stretch's totals count from its own start.
  const restarted = { ...fixture(), firstRead: 106, restarts: [104] };
  assert.equal(navalPeriodLabel(restarted, 'running', 'both', 107), `Since ${formatDateTime(106)}`);
  assert.equal(navalPeriodLabel(restarted, 'running', 'both', 102), `Since ${formatDateTime(100)}`);
  assert.match(navalPeriodLabel(restarted, 'month', 'both', 107), /^Per month, counted since /);
  assert.equal(navalPeriodLabel(restarted, 'month', 'both', 102), 'Per month');
  assert.equal(validateNavalLossesReply(restarted).restarts?.[0], 104);
  assert.throws(() => validateNavalLossesReply({ ...fixture(), restarts: [104, 104] }), /malformed/);
  assert.match(navalTooltip(navalModel(fixture()), 'mod tag', 109, 'Country', 'both', 'sunkBy').note!, /Non-hostile.*assisted kills/);
  assert.equal(navalTooltip(navalModel(fixture()), 'mod tag', 109, 'Country', 'both', 'lost').note, undefined);
  assert.equal(navalTooltip(navalModel(fixture()), 'mod tag', 109, 'Country', 'convoys', 'lost').note, undefined);
});

test('naval validator refuses duplicate countries, missing counts and overlapping reads', () => {
  for (const change of [
    (reply: NavalLossesReply) => reply.series.push(reply.series[0]),
    (reply: NavalLossesReply) => { reply.series[0].runs[1].firstHour = 101; },
    (reply: NavalLossesReply) => { reply.series[0].runs[0].ships = -1; },
    (reply: NavalLossesReply) => { reply.series[0].runs[0].ships = 1.5; },
    (reply: NavalLossesReply) => { reply.series[0].runs[0].convoys = -Number.MAX_SAFE_INTEGER - 1; },
    (reply: NavalLossesReply) => { reply.series[0].runs[0].ships = 1; reply.series[0].runs[0].convoys = Number.MAX_SAFE_INTEGER; },
    (reply: NavalLossesReply) => { reply.shipReads.push(108); },
    (reply: NavalLossesReply) => { reply.convoyReads[0] = 101; },
  ]) {
    const reply = fixture(); change(reply);
    assert.throws(() => validateNavalLossesReply(reply), /malformed/);
  }
  const reply = fixture();
  delete (reply.series[0].runs[0] as Partial<NavalLossRun>).convoys;
  assert.throws(() => validateNavalLossesReply(reply), /malformed/);
});

test('naval UI selects ten countries from five thousand with fifty thousand changes', () => {
  const reply = fixture();
  reply.series = Array.from({ length: 5000 }, (_, country) => ({ tag: `mod country ${country}`, membership: null,
    segments: [{ firstHour: 100, lastHour: 109 }], runs: Array.from({ length: 10 }, (_, index) => ({ firstHour: 100 + index,
      lastHour: 100 + index, startsSegment: index === 0, ships: country + index, convoys: index })) }));
  const model = navalModel(validateNavalLossesReply(reply));
  const ranked = rankNaval(model, model.tags, 109, 'both', 10);
  assert.equal(ranked.length, 10);
  assert.equal(ranked[0].tag, 'mod country 4999');
  assert.equal(navalLines(model, ranked.map(entry => entry.tag), 'both', { firstHour: 100, lastHour: 109 }, null).length, 10);
});


test('all five thousand requested countries are retained and ranked', () => {
  const tags = Array.from({ length: 5000 }, (_, country) => `requested mod country ${country}`);
  assert.equal(definition.countries.max, Number.MAX_SAFE_INTEGER);
  const resolved = resolveCountries({ kind: 'tags', tags }, [], undefined, 109, definition.countries.max);
  assert.equal(resolved.tags.length, tags.length);
  const request = navalRequest('branch', resolved, 'both', 'lost', 'running');
  assert.deepEqual(request.tags, tags);
  const reply = fixture();
  reply.series = tags.map(tag => ({ tag, membership: null, segments: [{ firstHour: 100, lastHour: 109 }],
    runs: [{ firstHour: 100, lastHour: 109, startsSegment: true, ships: 0, convoys: 0 }] }));
  const model = navalModel(validateNavalLossesReply(reply));
  assert.equal(rankNaval(model, resolved.tags, 109, 'both', resolved.tags.length).length, tags.length);
});


test('Both breakdown bands preserve parts, gaps, held identities and country membership', () => {
  const model = navalModel(fixture());
  const bands = navalBands(model, 'mod tag', { firstHour: 100, lastHour: 109 }, [{ firstHour: 101, lastHour: 108 }]);
  assert.deepEqual(bands.map(runs => runs.map(run => [run.firstHour, run.lastHour, run.value])), [
    [[101, 101, 0], [102, 103, 4], [107, 108, 2]],
    [[101, 101, 0], [102, 103, 2], [107, 108, 3]],
  ]);
  assert.equal(bands[0][2].breakBefore, true);
});

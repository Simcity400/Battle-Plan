import assert from 'node:assert/strict';
import test from 'node:test';
import { countryRaceEnds } from '../../charts/race/countryRace.ts';
import { SECONDARY_DASH } from '../../charts/palette.ts';
import { createWidget } from '../../dashboard/model.ts';
import { countryMatchFilter, projectResolvedCountries, rankedCandidates, resolveCountries, worldRequestTags } from '../../dashboard/groups.ts';
import { resolveOptions } from '../../dashboard/options.ts';
import type { OccupationTrendReply, OccupationTrendRun } from '../../generated/dashboard';
import { definition, OPTIONS } from './definition.ts';
import '../definitions.ts';
import { heldCoverage, occupationAt, occupationEntries, occupationHolderMix, occupationLines, occupationLineTooltip, occupationModel, occupationRequest,
  occupationSegmentAt, occupationTooltip, occupationTooltipTags, validateOccupationTrendReply } from './occupationTrendData.ts';

const run = (firstHour: number, lastHour: number, values: Partial<OccupationTrendRun> = {}): OccupationTrendRun => ({
  firstHour, lastHour, readHour: firstHour, startsSegment: false, resistanceRaw: '4133540', complianceRaw: '940099',
  states: 2, heldStates: 2, heldComplete: true, collaboration: [{ tag: 'Occupied country', raw: '12500', readHour: firstHour,
    actor: { slot: 3, countryId: 33, nameHour: firstHour } }], ...values,
});
const reply = (): OccupationTrendReply => ({ branchId: '1', decimals: 5, warnings: [], series: [{ tag: 'MOD A',
  runs: [run(100, 101), run(102, 102, { states: 1, heldStates: 2, resistanceRaw: '5000000', complianceRaw: '1000000' }),
    run(104, 105, { states: 1, heldStates: 1, startsSegment: true })],
  segments: [{ firstHour: 100, lastHour: 102 }, { firstHour: 104, lastHour: 105 }], membership: null }] });
const labels = (hour: number) => (tag: string) => ({ name: `${tag} renamed at ${hour}`, flag: `flag-${hour}` });

test('occupation manual roots follow every holder but a one-country mix retains only its chosen native holder', () => {
  const native = (firstHour: number, lastHour: number, slot: number, countryId: number) => ({ firstHour, lastHour, slot, countryId });
  const data: OccupationTrendReply = { branchId: '1', decimals: 5, warnings: [], series: [
    { tag: 'root', membership: null, segments: [{ firstHour: 100, lastHour: 102 }, { firstHour: 103, lastHour: 105 }],
      identities: [native(100, 102, 1, 11), native(103, 105, 2, 22)], runs: [run(100, 102), run(103, 105)] },
    { tag: 'alias', membership: null, segments: [{ firstHour: 103, lastHour: 104 }, { firstHour: 105, lastHour: 105 }],
      identities: [native(103, 104, 1, 11), native(105, 105, 3, 33)], runs: [run(103, 104), run(105, 105)] },
  ] };
  const model = occupationModel(validateOccupationTrendReply(data)), selection = { kind: 'tags' as const, tags: ['root'] };
  const resolved = resolveCountries(selection, [], undefined, 105, Number.MAX_SAFE_INTEGER);
  const counted = projectResolvedCountries(resolved, countryMatchFilter(selection, [], undefined, 105, model.identities, 100));
  assert.deepEqual(occupationRequest('1', resolved.tags, undefined, 'resistance').tags, ['root'], 'request roots never become recursive alias selections');
  assert.deepEqual(rankedCandidates(counted, 104, model.tags), ['root', 'alias']);
  assert.deepEqual(rankedCandidates(counted, 105, model.tags), ['root'], 'an unrelated holder of alias is excluded');
  const mix = occupationHolderMix(model, 'root', 101, 104, 'both', { firstHour: 100, lastHour: 105 }, null);
  assert.equal(mix.tag, 'alias'); assert.equal(mix.available, true);
  assert.ok(mix.lines.every(line => line.runs.at(-1)!.lastHour <= 104), 'neither reused tag contributes another native holder');
  assert.equal(occupationHolderMix(model, 'root', 101, 105, 'both', { firstHour: 100, lastHour: 105 }, null).available, false);
  data.series[0].identities = [];
  assert.deepEqual(occupationEntries(occupationModel(data), ['root'], 101, 'resistance', 10), []);
});

test('occupation defaults to all occupiers and lines with resistance, with top requests separate', () => {
  assert.deepEqual(createWidget('occupation-trend', 'main').countries, { kind: 'all' });
  const options = resolveOptions(OPTIONS, {});
  assert.equal(options.view, 'lines');
  assert.equal(options.measure, 'resistance');
  assert.equal(options.collaboration, false);
  assert.deepEqual(occupationRequest('1', [], undefined, 'resistance'), { branchId: '1', tags: [], top: null, collaboration: false });
  assert.deepEqual(occupationRequest('1', ['MOD A'], 10, 'compliance', true), { branchId: '1', tags: [], top: { count: 10, measure: 'compliance' }, collaboration: true });
});

test('gaining and losing held states changes coverage without inventing values for unread states', () => {
  const model = occupationModel(validateOccupationTrendReply(reply()));
  assert.equal(heldCoverage(occupationAt(model, 'MOD A', 101)!.run), '2/2 states');
  assert.equal(heldCoverage(occupationAt(model, 'MOD A', 102)!.run), '1/2 states');
  assert.equal(occupationEntries(model, ['MOD A'], 102, 'resistance', 10)[0].label, '50%');
  assert.equal(heldCoverage(occupationAt(model, 'MOD A', 104)!.run), '1/1 states');
  assert.equal(occupationAt(model, 'MOD A', 103), null);
  assert.equal(occupationAt(model, 'MOD A', 106), null);
  assert.equal(occupationSegmentAt(model, 'MOD A', 106), null);
  assert.deepEqual(occupationEntries(model, ['MOD A', 'not recorded'], 106, 'resistance', 10), []);
  const lines = occupationLines(model, model.tags, 'resistance', { firstHour: 100, lastHour: 106 }, null);
  assert.deepEqual(lines[0].runs.map(run => [run.firstHour, run.lastHour, run.value]), [[100, 101, 41.3354], [102, 102, 50], [104, 105, 41.3354]]);
  assert.equal(lines[0].runs.at(-1)?.breakBefore, true);
});

test('tooltips use hovered names and flags, exact averages and supplying read dates', () => {
  const model = occupationModel(reply());
  const tip = occupationTooltip(model, 'MOD A', 101, labels, 'both', true)!;
  assert.equal(tip.title, 'MOD A renamed at 101');
  assert.equal(tip.rows[0].value, '41.3354%');
  assert.equal(tip.rows[1].value, '9.40099%');
  assert.equal(tip.rows.find(row => row.label === 'Read')?.value, undefined);
  assert.deepEqual(tip.rows.find(row => row.label.startsWith('Country collaboration:')), {
    label: 'Country collaboration: Occupied country renamed at 101', countryFlag: { name: 'Occupied country renamed at 101', flag: 'flag-101' }, value: '12.5%',
  });
  assert.equal(tip.rows.find(row => row.label === 'Collaboration read')?.value, undefined);
  assert.match(tip.note!, /Average across 2 states/);
  assert.deepEqual(occupationTooltipTags(model, 101, ['MOD A'], true), ['Occupied country']);
  assert.deepEqual(occupationTooltipTags(model, 101, ['MOD A'], false), []);
  const line = occupationLineTooltip(model, 101, ['MOD A'], labels, () => 'recorded-colour', 'both', 'resistance', false);
  assert.equal(line.rows[0].label, 'MOD A renamed at 101');
  assert.equal(line.rows[0].countryFlag?.flag, 'flag-101');
});

test('both means remain independent solid and dashed lines, with exact country end labels', () => {
  const model = occupationModel(reply());
  const lines = occupationLines(model, model.tags, 'both', { firstHour: 100, lastHour: 105 }, null, 5);
  assert.equal(lines.length, 2);
  assert.equal(lines[0].dash, undefined);
  assert.equal(lines[1].dash, SECONDARY_DASH);
  const ends = countryRaceEnds(lines, 100, 105, 101, (tag, hour) => labels(hour!)(tag).name, () => 'country', String);
  assert.deepEqual(ends.map(end => end.text), ['MOD A renamed at 101 Resistance 41.3354%', 'MOD A renamed at 101 Compliance 9.40099%']);
  assert.notEqual(ends[0].id, ends[1].id);
  const single = occupationLines(model, model.tags, 'resistance', { firstHour: 100, lastHour: 105 }, null, 5);
  assert.deepEqual(countryRaceEnds(single, 100, 105, 101, (tag, hour) => labels(hour!)(tag).name, () => 'country', String)
    .map(end => end.text), ['MOD A renamed at 101 41.3354%']);
});

test('held collaboration refuses another holder\'s looks while keeping its exact amount and read', () => {
  for (const item of [
    { tag: 'reused tag', raw: '12500', readHour: 100, actor: { slot: 13, countryId: 3, nameHour: null } },
    { tag: null, raw: '12500', readHour: 100, actor: { slot: 13, countryId: 3, nameHour: null } },
    { tag: 'reused tag', raw: '12500', readHour: 100 },
  ]) {
    const data = reply(); data.series[0].runs[0].collaboration = [item];
    const model = occupationModel(validateOccupationTrendReply(data)), requested: string[] = [];
    const tip = occupationTooltip(model, 'MOD A', 101, hour => tag => {
      requested.push(tag); return labels(hour)(tag);
    }, 'resistance', true)!;
    assert.deepEqual(requested, ['MOD A'], 'an unproved occupied actor never reaches the country look hook');
    assert.deepEqual(occupationTooltipTags(model, 101, ['MOD A'], true), []);
    const row = tip.rows.find(row => row.label.startsWith('Country collaboration:'))!;
    assert.equal(row.value, '12.5%'); assert.equal(row.countryFlag?.flag, undefined);
    assert.match(row.countryFlag!.flagReason!, /identity/);
    assert.equal(row.label, 'Country collaboration: Country name unavailable');
    assert.equal(tip.rows.find(row => row.label === 'Collaboration read')?.value, undefined);
    assert.equal(tip.rows.find(row => row.label === 'Country identity'), undefined);
  }
});

test('partial held and missing collaboration reads stay explicitly unknown', () => {
  const data = reply();
  data.series[0].runs[0] = run(100, 101, { heldComplete: false, heldStates: 2, collaboration: [{ tag: 'Occupied country', raw: null, readHour: null }] });
  const tip = occupationTooltip(occupationModel(validateOccupationTrendReply(data)), 'MOD A', 101, labels, 'resistance', true)!;
  assert.equal(tip.rows.find(row => row.label === 'States')?.value, '2 states; total unknown');
  assert.equal(tip.rows.find(row => row.label.startsWith('Country collaboration:'))?.value, 'Not recorded');
  assert.equal(tip.rows.find(row => row.label === 'Collaboration read')?.value, undefined);
});

test('zero is a readable average and ranking uses the exact native raw before geometry', () => {
  const data = reply();
  const first = data.series[0];
  data.series = [
    { ...first, tag: 'zero', runs: [run(100, 102, { resistanceRaw: '0' })] },
    { ...first, tag: 'smaller', runs: [run(100, 102, { resistanceRaw: '9007199254740992' })] },
    { ...first, tag: 'larger', runs: [run(100, 102, { resistanceRaw: '9007199254740993' })] },
  ];
  const model = occupationModel(validateOccupationTrendReply(data));
  assert.deepEqual(occupationEntries(model, model.tags, 101, 'resistance', 10).map(entry => entry.tag), ['larger', 'smaller', 'zero']);
  assert.equal(occupationEntries(model, ['zero'], 101, 'resistance', 10)[0].label, '0%');
});

test('all 5000 country series validate and rank before the top selection is applied', () => {
  const data = reply();
  const first = data.series[0];
  data.series = Array.from({ length: 5000 }, (_, index) => ({ ...first, tag: `Mod country ${String(index).padStart(4, '0')}`,
    runs: [run(100, 102, { resistanceRaw: String(index * 100000) })] }));
  const model = occupationModel(validateOccupationTrendReply(data));
  const entries = occupationEntries(model, model.tags, 101, 'resistance', 10);
  assert.equal(entries.length, 10);
  assert.equal(entries[0].tag, 'Mod country 4999');
});

test('chosen 5000 occupiers stay chosen and all uses the recorded series without changing shared selection behavior', () => {
  assert.deepEqual(definition.countries, { max: Number.MAX_SAFE_INTEGER, ranked: true, all: true });
  const data = reply();
  const first = data.series[0];
  data.series = Array.from({ length: 5000 }, (_, index) => ({ ...first, tag: `Mod country ${index}`,
    identities: first.segments.map(span => ({ ...span, slot: index + 1, countryId: index + 1 })) }));
  const model = occupationModel(data);
  const chosen = { kind: 'tags' as const, tags: model.tags };
  const selected = resolveCountries(chosen, ['unselected'], undefined, 105, definition.countries.max);
  assert.equal(selected.dropped, 0);
  assert.deepEqual(selected.tags, model.tags);
  const request = worldRequestTags(chosen, ['unselected'], undefined, 105, model.identities);
  assert.equal(request.ready, true);
  assert.deepEqual(occupationRequest('1', request.tags, selected.top, 'resistance').tags, model.tags);
  assert.equal(occupationEntries(model, rankedCandidates(selected, 101, model.tags), 101, 'resistance', selected.tags.length).length, 5000);

  const all = worldRequestTags({ kind: 'all' }, [], undefined, 105);
  assert.deepEqual(all, { tags: [], ready: true, reason: null });
  assert.equal(occupationEntries(model, model.tags, 101, 'resistance', model.tags.length).length, 5000);
  assert.deepEqual(resolveCountries({ kind: 'all' }, ['selected elsewhere'], undefined, 105, 10).tags, ['selected elsewhere']);
});

test('malformed occupation data refuses unread zero denominators and invalid reads', () => {
  const mutations: ((data: OccupationTrendReply) => void)[] = [
    data => { data.series[0].runs[0].states = 0; },
    data => { data.series[0].runs[0].heldStates = 1; },
    data => { data.series[0].runs[2].readHour = 106; },
    data => { data.series[0].runs[0].lastHour = 103; },
    data => { data.series[0].runs[0].collaboration[0].readHour = null; },
    data => { data.series[0].runs[0].resistanceRaw = '1.2'; },
    data => { data.series[0].runs[0].collaboration[0].actor!.countryId = -1; },
    data => { data.series[0].runs[0].collaboration[0].actor!.slot = 2 ** 32; },
    data => { data.series[0].runs[0].collaboration[0].actor!.nameHour = 101; },
    data => { data.series[0].runs[0].collaboration[0].tag = null; },
    data => { data.series[0].runs[0].collaboration.push({ ...data.series[0].runs[0].collaboration[0], tag: 'another alias' }); },
    data => { data.series.push(data.series[0]); },
  ];
  for (const mutate of mutations) { const data = reply(); mutate(data); assert.throws(() => validateOccupationTrendReply(data), /malformed/); }
});

test('top membership clipping preserves earlier supplying reads', () => {
  const data = reply();
  data.series[0].runs = [run(101, 101, { readHour: 100, collaboration: [] }), run(105, 105, { readHour: 104, collaboration: [] })];
  data.series[0].membership = [{ firstHour: 101, lastHour: 101 }, { firstHour: 105, lastHour: 105 }];
  const model = occupationModel(validateOccupationTrendReply(data));
  assert.equal(occupationAt(model, 'MOD A', 101)?.run.readHour, 100);
  assert.equal(occupationAt(model, 'MOD A', 105)?.run.readHour, 104);
});

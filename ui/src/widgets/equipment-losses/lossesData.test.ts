import assert from 'node:assert/strict';
import test from 'node:test';
import type { Coverage, LossCause, LossLeaf, LossesReply } from '../../generated/dashboard';
import { amountText, arcTargets, foldTree, groupTree, listed, sliceColours, sliceTooltip, sunburstSummary, walk } from '../../charts/sunburst/layout.ts';
import { resolveOptions } from '../../dashboard/options.ts';
import { sunburstSettings, levelsShown } from '../../charts/sunburst/sunburstOptions.ts';
import { definition, OPTIONS } from './definition.ts';
import { nearest, shipLeaves, stepHours, withEdges, withHead } from './lossesData.ts';
import { LEVELS, atHead, causeName, destructionCoverageAt, destructionCoverageHours, coverageAt, coverageHours, coverageLegend, destructionLegend, destructionCountingNote, formatLosses, landNote, lossItems, lossesTotal, lossesFootnote, lossKey, pointAt, unknownCauses, validateLossesReply } from './lossesData.ts';
import { repliedCells } from '../../dashboard/follow/liveCells.ts';

const ROOT = { id: 'all', label: 'All losses' };
const TOKENS = { positive: '#00aa00', warning: '#aaaa00', error: '#aa0000', surface: '#000000', text: '#ffffff' };
const live = (hour: number) => ({ first: 100, hour, end: 109, campaign: false });
const whole = { ...live(109), campaign: true };
const leaf = (extra: Partial<LossLeaf> = {}): LossLeaf => ({ design: 0, key: '0', points: [{ hour: 100, totalRaw: '200000' }, { hour: 105, totalRaw: '350001' }], exact: [], lowerBoundFrom: null, ...extra });
const cause = (extra: Partial<LossCause> = {}): LossCause => ({ source: { kind: 'combatLog', reason: 0 }, domain: 'land', name: { key: 'mod_loss_combat', template: 'Mod combat', text: 'Mod combat' },
  group: null, loss: true, unknown: null, unknownSpans: [], leaves: [leaf()], ...extra });
const reply = (): LossesReply => ({
  branchId: 'mod-branch', ships: [], from: 100, to: 110, width: 64, exactHours: [], restarts: [], tag: 'opaque: tag / 1', amountScale: 100_000, landSource: 'combatLog', landLowerBound: null, destructionRuns: [{ firstHour: 100, lastHour: 109, countedFrom: 100, withheldFrom: null }], destructionCoverage: [{ firstHour: 100, lastHour: 109, reason: null }], retainedMonths: 7, warnings: [],
  designs: [{ variant: 91001, equipment: 'mod_walkers_v2', archetype: 'mod_walkers', archetypeName: 'Walking machines',
    names: [{ firstHour: 100, name: 'First walker', known: true }, { firstHour: 105, name: 'Renamed walker', known: true }] }],
  coverage: [{ firstHour: 100, lastHour: 109, coverage: 'logged' }],
  causes: [cause(), cause({ source: { kind: 'sunkConvoys' }, domain: 'naval', name: { key: 'mod_sunk', template: 'Mod sinkings', text: 'Mod sinkings' },
    leaves: [leaf({ design: null, points: [{ hour: 104, totalRaw: '100000' }] })] }),
  cause({ source: { kind: 'destruction', reason: 'air_accident' }, domain: 'air', name: { key: null, template: null, text: null }, leaves: [] })],
});
const treeAt = (data: LossesReply, hour: number) => groupTree(lossItems(data, live(hour)), ROOT, [], lossesTotal(lossItems(data, live(hour))));
const TEXT = { format: (value: number, exact: bigint | null) => formatLosses(value, exact, 100_000), unit: 'units', valueLabel: 'Losses' };

test('validates the contract and refuses malformed amounts, names, indices, spans and unknown causes with leaves', () => {
  const data = reply();
  assert.equal(validateLossesReply(data), data);
  for (const mutate of [
    (bad: LossesReply) => { bad.causes[0].leaves[0].design = 10; },
    (bad: LossesReply) => { bad.designs.push({ ...bad.designs[0] }); },
    (bad: LossesReply) => { bad.causes[0].leaves.push({ ...bad.causes[0].leaves[0] }); },
    (bad: LossesReply) => { delete (bad.causes[0] as Partial<LossCause>).domain; },
    (bad: LossesReply) => { bad.causes[0].leaves[0].points[0].totalRaw = '1.25'; },
    (bad: LossesReply) => { bad.causes[0].leaves[0].points.reverse(); },
    (bad: LossesReply) => { bad.causes[0].leaves[0].exact = [{ hour: 100, totalRaw: '1.25' }]; },
    (bad: LossesReply) => { bad.causes[0].leaves[0].exact = [{ hour: 105, totalRaw: null }, { hour: 100, totalRaw: null }]; },
    (bad: LossesReply) => { bad.designs[0].names.reverse(); },
    (bad: LossesReply) => { bad.ships = [{ signature: 's', listedHour: 100, sinkHour: 99, design: 9, count: 1 }]; },
    (bad: LossesReply) => { delete (bad.causes[0].leaves[0] as Partial<LossLeaf>).key; },
    (bad: LossesReply) => { bad.coverage[0].lastHour = 99; },
    (bad: LossesReply) => { bad.coverage.push({ firstHour: 105, lastHour: 110, coverage: 'unknown' }); },
    (bad: LossesReply) => { bad.causes[2].unknown = 'Not available.'; bad.causes[2].leaves = [leaf()]; },
    (bad: LossesReply) => { bad.amountScale = 3; },
    (bad: LossesReply) => { bad.destructionRuns = [{ firstHour: 100, lastHour: 109, countedFrom: NaN, withheldFrom: null }]; },
    (bad: LossesReply) => { bad.destructionRuns = [{ firstHour: 105, lastHour: 104, countedFrom: 105, withheldFrom: null }]; },
    (bad: LossesReply) => { bad.destructionRuns = [{ firstHour: 100, lastHour: 105, countedFrom: 100, withheldFrom: null }, { firstHour: 105, lastHour: 109, countedFrom: 105, withheldFrom: null }]; },
    (bad: LossesReply) => { bad.destructionCoverage = [{ firstHour: 105, lastHour: 104, reason: null }]; },
    (bad: LossesReply) => { bad.causes[0].unknownSpans = [{ firstHour: 105, lastHour: 104, reason: 'unread', detail: null }]; },
  ]) {
    const bad = reply(); mutate(bad);
    assert.throws(() => validateLossesReply(bad), /malformed/);
  }
  const overlap = reply();
  overlap.causes[0].unknownSpans = [{ firstHour: 102, lastHour: 109, reason: 'endedAfterLastRead', detail: null }, { firstHour: 104, lastHour: 105, reason: 'unread', detail: null }];
  assert.equal(validateLossesReply(overlap), overlap);
  const decrease = reply();
  decrease.causes[0].leaves[0].points[1].totalRaw = '1';
  assert.equal(validateLossesReply(decrease), decrease);
  assert.equal(treeAt(decrease, 105).exact, 100001n);
  decrease.causes[0].leaves[0].points[1].totalRaw = '-1';
  assert.equal(validateLossesReply(decrease), decrease);
  assert.equal(treeAt(decrease, 105).exact, 99999n);
});

test('totals grow on the recorded hour, without interpolation; campaign includes each latest total once', () => {
  const data = reply();
  assert.equal(pointAt(data.causes[0].leaves[0], 99), null);
  assert.equal(treeAt(data, 100).exact, 200000n);
  assert.equal(treeAt(data, 103).exact, 200000n);
  assert.equal(treeAt(data, 104).exact, 300000n);
  assert.equal(treeAt(data, 105).exact, 450001n);
  const campaign = groupTree(lossItems(data, whole), ROOT);
  assert.equal(campaign.exact, 450001n);
  assert.equal(amountText(campaign, TEXT), '4.5');
  assert.equal(treeAt(data, 100).final, campaign.value);
  assert.equal(formatLosses(Number(9007199254741499n), 9007199254741499n, 100_000), '90,071,992,547');
});

test('losses read as whole units from ten, and a fraction of a unit never reads as none', () => {
  assert.equal(formatLosses(0, 39_831_823_130n, 100_000), '398,318');
  assert.equal(formatLosses(0, 22_156_862_990n, 100_000), '221,569');
  assert.equal(formatLosses(0, 250_000n, 100_000), '2.5');
  assert.equal(formatLosses(0, 9_560n, 100_000), '0.096');
  assert.equal(formatLosses(0, -1_278_460n, 100_000), 'Returned: −13');
});

test('a recorded correction to zero preserves earlier slice colours and folding without changing final totals', () => {
  const data = reply(); data.causes = [data.causes[0]];
  data.causes[0].leaves[0].points[1].totalRaw = '0';
  const early = treeAt(data, 100);
  const end = treeAt(data, 109);
  assert.equal(early.value, 200000);
  assert.equal(early.final, 0);
  assert.equal(end.exact, 0n);
  assert.equal(sliceColours(early, ['#ff0000'], { light: '#ffffff', dark: '#000000' }).get(early.children[0].id), '#ff0000');
  assert.equal(early.children[0].weight, end.children[0].weight);
});

test('names are content types and the design name recorded at the hour shown, never a later name', () => {
  const data = reply();
  assert.deepEqual(lossItems(data, live(104))[1].path.map(key => key.label), ['Mod combat', 'Walking machines', 'First walker']);
  assert.equal(lossItems(data, whole)[1].path[2].label, 'Renamed walker');
  data.designs[0].names = [{ firstHour: 105, name: 'Future name', known: true }];
  assert.equal(lossItems(data, live(104))[1].path[2].label, 'Name unavailable');
  data.designs[0].names.unshift({ firstHour: 100, name: null, known: false });
  assert.equal(lossItems(data, live(104))[1].path[2].label, 'Name unavailable');
  data.designs[0].archetypeName = null;
  assert.equal(lossItems(data, live(104))[1].path[1].label, 'Equipment type name unavailable');
});

test('each hourly land state has its legend entry, shared status colour and Partial exact N', () => {
  const states: Coverage[] = ['logged', { partial: { ungrouped: 17 } }, 'notLogged', 'unknown'];
  const labels = ['Complete', 'Incomplete', 'Incomplete', 'Incomplete'];
  const colours = [TOKENS.positive, TOKENS.warning, TOKENS.warning, TOKENS.warning];
  for (const [index, coverage] of states.entries()) {
    const data = reply(); data.coverage[0].coverage = coverage;
    const entries = coverageLegend(data, live(105), TOKENS);
    assert.equal(entries.length, 1);
    assert.equal(entries[0].label, labels[index]);
    assert.equal(entries[0].color, colours[index]);
    assert.deepEqual(coverageAt(data, 105), coverage);
    assert.equal(treeAt(data, 105).exact, 450001n);
    if (index === 0) assert.equal(landNote(data, 100, 105), undefined);
    else assert.match(lossesFootnote(data, live(105)), /Incomplete/i);
  }
});

test('Partial N follows the hour, and Live off lists hours in every state present', () => {
  const data = reply(); data.coverage = [
    { firstHour: 100, lastHour: 101, coverage: 'logged' },
    { firstHour: 102, lastHour: 103, coverage: { partial: { ungrouped: 3 } } },
    { firstHour: 104, lastHour: 105, coverage: { partial: { ungrouped: 19 } } },
    { firstHour: 106, lastHour: 107, coverage: 'notLogged' },
    { firstHour: 108, lastHour: 109, coverage: 'unknown' },
  ];
  assert.equal(coverageLegend(data, live(103), TOKENS)[0].label, 'Incomplete');
  assert.equal(coverageLegend(data, live(104), TOKENS)[0].label, 'Incomplete');
  assert.deepEqual(coverageLegend(data, whole, TOKENS).map(entry => entry.label), ['Incomplete']);
  assert.deepEqual(coverageHours(data, 101, 108), { logged: 1, partial: 4, notLogged: 2, unknown: 1 });
});

test('missing coverage hours and unobserved retention are Unknown, with the explicit reason', () => {
  const data = reply(); data.coverage = [
    { firstHour: 100, lastHour: 102, coverage: 'logged' },
    { firstHour: 105, lastHour: 109, coverage: 'pruneUnobserved' },
  ];
  assert.equal(coverageAt(data, 104), 'unknown');
  assert.deepEqual(coverageHours(data, 100, 109), { logged: 3, partial: 0, notLogged: 0, unknown: 7 });
  assert.equal(coverageLegend(data, live(105), TOKENS)[0].label, 'Incomplete');
  assert.match(landNote(data, 100, 105)!, /Incomplete total/);
  data.coverage = [];
  assert.equal(coverageLegend(data, whole, TOKENS)[0].label, 'Incomplete');
});

test('a later Logged hour keeps cumulative land losses explicitly incomplete', () => {
  const data = reply(); data.coverage = [{ firstHour: 100, lastHour: 103, coverage: { partial: { ungrouped: 2 } } },
    { firstHour: 104, lastHour: 109, coverage: 'logged' }];
  assert.equal(coverageLegend(data, live(105), TOKENS)[0].label, 'Complete');
  const root = treeAt(data, 105);
  const land = root.children.find(node => node.label === 'Mod combat')!;
  for (const node of walk(land)) assert.match(node.notes.join(' '), /Incomplete total/);
  assert.match(lossesFootnote(data, live(105)), /Incomplete/i);
});

test('unknown causes say Not known with the reason, and have no equipment leaves or slices', () => {
  const data = reply();
  data.causes[0].unknown = 'Content retention deletes changing entries.';
  data.causes[2].unknown = 'Destruction table is unavailable.';
  data.causes[0].leaves = [];
  const root = treeAt(data, 105);
  const unknown = root.children.filter(node => node.unmeasured);
  assert.equal(unknown.length, 2);
  for (const node of unknown) {
    assert.deepEqual(node.children, []);
    assert.equal(arcTargets(root, root, 'share').get(node.id)!.shown, false);
    assert.equal(sliceTooltip(node, root, TEXT)[0].rows[0].value, 'Not known');
    assert.ok(sliceTooltip(node, root, TEXT)[0].note);
  }
  assert.match(sunburstSummary(root, TEXT), /not known/);
  assert.deepEqual(unknownCauses(data, 105).map(entry => entry.label), ['Mod combat: Unavailable',
    'Cause name unavailable: Unavailable']);
  assert.equal(root.exact, 100000n);
});

test('unread spans never become zero; they mark the cause once and stop when it is read again', () => {
  const data = reply(); data.causes[0].unknownSpans = [{ firstHour: 103, lastHour: 104, reason: 'unread', detail: null }];
  assert.equal(treeAt(data, 104).exact, 300000n);
  const unknown = treeAt(data, 104).children.find(node => node.label === 'Mod combat')!;
  assert.deepEqual(unknown.unknown, {});
  assert.ok(!sliceTooltip(unknown, treeAt(data, 104), TEXT)[0].rows.some(row => row.label === 'Not counted'));
  assert.match(unknown.notes.join(' '), /Incomplete total/);
  assert.match(unknownCauses(data, 104)[0].label, /Incomplete/);
  assert.deepEqual(treeAt(data, 105).unknown, {});
  data.causes[0].leaves = [];
  assert.equal(treeAt(data, 104).children.find(node => node.label === 'Mod combat')!.unmeasured, true);
});

test('lend-lease incomplete subtotals retain exact amounts and named notes through grouping and folding', () => {
  const data = reply(); data.causes[0].source = { kind: 'lendLeaseSunk' }; data.causes[0].domain = 'other';
  data.causes[0].leaves[0].lowerBoundFrom = 105;
  data.causes[0].unknownSpans = [{ firstHour: 105, lastHour: 109, reason: 'endedAfterLastRead', detail: null }];
  assert.equal(lossesTotal(lossItems(data, live(104))).incomplete, false);
  const root = treeAt(data, 105);
  const lease = root.children.find(node => node.label === 'Mod combat')!;
  for (const node of walk(lease)) assert.match(node.notes.join(' '), /Incomplete total/i);
  assert.equal(root.exact, 450001n);
  assert.equal(root.finalUnknown, true);
  assert.match(sliceTooltip(lease, root, TEXT)[0].note!, /Incomplete total/i);
  const folded = foldTree(groupTree(lossItems(data, live(105)), ROOT), 1);
  assert.match(folded.children.find(node => node.other)!.notes.join(' '), /Incomplete total/i);
  data.causes[0].leaves = [];
  assert.equal(lossesTotal(lossItems(data, live(105))).incomplete, true);
});

test('recovered and lend-lease sent are left out of totals, hierarchy, counts and tooltips', () => {
  const data = reply();
  for (const kind of ['recovered', 'lendLeaseSent'] as const) data.causes.push(cause({ source: { kind }, domain: 'other', loss: false,
    name: { key: kind, template: kind, text: kind }, leaves: [leaf({ points: [{ hour: 100, totalRaw: '999999999999' }] })] }));
  const root = treeAt(data, 109);
  assert.equal(root.exact, 450001n);
  assert.equal(root.count, 2);
  assert.ok(!walk(root).some(node => ['recovered', 'lendLeaseSent'].includes(node.label)));
  assert.ok(!root.counted.has(lossKey(data.causes[3], data.designs[0])));
  assert.ok(!root.counted.has(lossKey(data.causes[4], data.designs[0])));
});

test('all three levels are optional and default behaviour uses the shared sunburst settings', () => {
  const options = resolveOptions(OPTIONS, undefined);
  assert.deepEqual(levelsShown(options, LEVELS), [true, true, true]);
  assert.deepEqual(sunburstSettings(options), { growth: 'share', labels: true, legend: true, breadcrumb: true, dimming: true });
  assert.equal(definition.defaultTime, 'follow');
  const data = reply();
  for (const levels of [[false, true, true], [true, false, true], [true, true, false], [false, false, false]]) {
    assert.equal(groupTree(lossItems(data, whole), ROOT, levels).exact, 450001n);
  }
});

const destruction = (): LossesReply => {
  const data = reply();
  data.landSource = 'destruction';
  data.landLowerBound = null;
  data.coverage = [];
  data.destructionRuns = [{ firstHour: 100, lastHour: 109, countedFrom: 101, withheldFrom: null }];
  data.destructionCoverage = [{ firstHour: 101, lastHour: 109, reason: null }];
  data.causes[0].source = { kind: 'destruction', reason: 'land_combat' };
  data.causes[0].leaves[0].points[0].hour = 101;
  data.causes.pop();
  return data;
};


test('destruction coverage uses Recorded and Unknown with the drain reason and hours per state', () => {
  const data = destruction();
  assert.equal(validateLossesReply(data), data);
  assert.equal(destructionCoverageAt(data, 100), 'No destruction drain was recorded at this hour.');
  assert.equal(destructionCoverageAt(data, 101), null);
  assert.deepEqual(coverageLegend(data, live(100), TOKENS), []);
  assert.deepEqual(coverageLegend(data, live(105), TOKENS), [{ id: 'recorded', color: TOKENS.positive, label: 'Complete' }]);
  assert.deepEqual(destructionCoverageHours(data, 100, 109), { recorded: 9, unknown: 0, reasons: [] });
  assert.deepEqual(coverageLegend(data, whole, TOKENS).map(entry => entry.label), ['Complete']);
  data.destructionCoverage = [{ firstHour: 103, lastHour: 104, reason: null },
    { firstHour: 105, lastHour: 109, reason: 'Destruction table withheld after overflow.' }];
  assert.match(coverageLegend(data, live(101), TOKENS)[0].label, /Incomplete/);
  assert.deepEqual(destructionCoverageHours(data, 100, 109), { recorded: 2, unknown: 7,
    reasons: ['Destruction table withheld after overflow.', 'No destruction drain was recorded at this hour.'] });
  assert.match(coverageLegend(data, whole, TOKENS)[0].label, /Incomplete/);
});

test('complete supplemental destruction has no permanent deletion or paradrop exclusion', () => {
  const data = destruction();
  const root = treeAt(data, 105);
  assert.equal(lossesTotal(lossItems(data, live(105))).incomplete, false);
  assert.equal(root.exact, 450001n);
  assert.equal(landNote(data, 100, 105), undefined);
  assert.equal(lossesFootnote(data, whole), '');
  assert.doesNotMatch(root.notes.join(' '), /Not included/);
});

test('destruction unknown intervals never become zero; known subtotals remain incomplete with named reasons', () => {
  for (const reason of ['destructionWithheld', 'destructionGap', 'destructionUnresolved', 'destructionDomain'] as const) {
    const data = destruction();
    data.causes[0].unknownSpans = [{ firstHour: 100, lastHour: 109, reason, detail: `Named ${reason} reason.` }];
    const root = treeAt(data, 105);
    const land = root.children.find(node => node.label === 'Mod combat')!;
    assert.equal(land.exact, 350001n);
    assert.match(land.notes.join(' '), /Incomplete total/i);
    assert.doesNotMatch(land.notes.join(' '), new RegExp(`Named ${reason}`));
    assert.deepEqual(land.unknown, {});
    assert.ok(!sliceTooltip(land, root, TEXT)[0].rows.some(row => row.label === 'Not counted'));
    assert.match(unknownCauses(data, 105)[0].label, /Incomplete/);
    data.causes[0].leaves = [];
    const empty = treeAt(data, 105).children.find(node => node.label === 'Mod combat')!;
    assert.equal(empty.unmeasured, true);
    assert.equal(sliceTooltip(empty, treeAt(data, 105), TEXT)[0].rows[0].value, 'Not known');
    assert.match(unknownCauses(data, 105)[0].label, /Unavailable/);
  }
});

test('every stable air cause has design leaves in both land modes, with product game or app labels', () => {
  const labels = [
    ['air_nuclear', 'Nuclear air losses'], ['air_accident', 'Air accidents'], ['air_strategic', 'Strategic air losses'],
    ['air_mission_aa', 'Air mission losses'], ['air_land_combat_cas', 'Land combat anti-air'], ['air_raid', 'Raid air losses'], ['air_naval_aa', 'Naval anti-air'],
  ] as const;
  for (const landSource of ['destruction', 'combatLog'] as const) {
    const data = landSource === 'destruction' ? destruction() : reply();
    if (landSource === 'combatLog') data.causes.pop();
    for (const [reason, label] of labels) data.causes.push(cause({ source: { kind: 'destruction', reason }, domain: 'air',
      name: { key: null, template: null, text: label }, leaves: [leaf({ points: [{ hour: 103, totalRaw: '100000' }] })] }));
    assert.equal(validateLossesReply(data), data);
    assert.equal(treeAt(data, 102).exact, 200000n);
    assert.equal(treeAt(data, 105).exact, 1150001n);
    for (const [, label] of labels) {
      const air = treeAt(data, 105).children.find(node => node.label === label)!;
      assert.equal(air.children[0].children[0].label, 'Renamed walker');
      assert.equal(air.exact, 100000n);
      assert.doesNotMatch(air.notes.join(' '), /Incomplete total/i);
      assert.doesNotMatch(air.notes.join(' '), /Not included:/);
    }
    assert.ok(!lossItems(data, whole).some(item => item.path[0].label === 'Unnamed cause (air)'));
  }
  const game = cause({ source: { kind: 'destruction', reason: 'land_combat' },
    name: { key: 'mod_combat_key', template: 'Mod content combat', text: 'Resolved mod combat' } });
  assert.equal(causeName(game), 'Resolved mod combat');
  game.name.text = null;
  assert.equal(causeName(game), 'Mod content combat');
  game.name.template = null;
  assert.equal(causeName(game), 'Cause name unavailable');
  assert.equal(causeName(cause({ source: { kind: 'destruction', reason: 'air_accident' }, domain: 'air',
    name: { key: null, template: null, text: 'Air accidents' } })), 'Air accidents');
});

test('unrecognised destruction callers retain their own cause, never a guessed known cause', () => {
  for (const landSource of ['destruction', 'combatLog'] as const) {
    const data = landSource === 'destruction' ? destruction() : reply();
    if (landSource === 'combatLog') data.causes.pop();
    data.causes.push(cause({ source: { kind: 'destruction', reason: 'unknown' }, domain: 'other',
      name: { key: null, template: null, text: 'Unrecognised destruction caller' },
      leaves: [leaf({ points: [{ hour: 102, totalRaw: '700000' }] })] }));
    const root = treeAt(data, 105);
    assert.equal(root.exact, 1150001n);
    assert.equal(root.children.find(node => node.label === 'Unrecognised destruction caller')!.exact, 700000n);
    assert.equal(root.children.find(node => node.label === 'Mod combat')!.exact, 350001n);
  }
});

test('unavailable air writers stay Not known in both modes and a complete empty cause stays zero', () => {
  for (const landSource of ['destruction', 'combatLog'] as const) {
    const data = landSource === 'destruction' ? destruction() : reply();
    if (landSource === 'combatLog') data.causes.pop();
    const air = cause({ source: { kind: 'destruction', reason: 'air_accident' }, domain: 'air',
      name: { key: null, template: null, text: 'Air accidents' }, leaves: [],
      unknownSpans: [{ firstHour: 100, lastHour: 109, reason: 'destructionWithheld', detail: 'Destruction table withheld after a lost drain.' }] });
    data.causes.push(air);
    const root = treeAt(data, 105);
    assert.equal(sliceTooltip(root.children.find(node => node.label === 'Air accidents')!, root, TEXT)[0].rows[0].value, 'Not known');
    assert.match(unknownCauses(data, 105).find(item => item.label.startsWith('Air accidents'))!.label, /Unavailable/);
    air.unknownSpans = [];
    assert.equal(treeAt(data, 105).children.find(node => node.label === 'Air accidents')!.value, 0);
    assert.ok(!unknownCauses(data, 105).some(item => item.label.startsWith('Air accidents')));
  }
});

test('an ended exchange keeps unrelated lend-lease designs exact while its cause and centre are incomplete', () => {
  const data = reply();
  data.causes[0].source = { kind: 'lendLeaseSunk' };
  data.causes[0].domain = 'other';
  data.causes[0].unknownSpans = [{ firstHour: 105, lastHour: 109, reason: 'endedAfterLastRead', detail: null }];
  data.causes[0].leaves[0].lowerBoundFrom = 105;
  data.causes[0].leaves.push(leaf({ design: null, lowerBoundFrom: null, points: [{ hour: 100, totalRaw: '100000' }] }));
  const items = lossItems(data, live(105));
  const key = lossKey(data.causes[0], undefined);
  assert.equal(items.find(item => item.counted?.includes(key))!.incomplete, false);
  const root = treeAt(data, 105);
  const unrelated = walk(root).find(node => node.label === 'Design not recorded' && node.counted.has(key))!;
  assert.doesNotMatch(unrelated.notes.join(' '), /Incomplete total/i);
  assert.equal(amountText(unrelated, TEXT), '1');
  assert.equal(lossesTotal(items).incomplete, true);
});

test('counting starts at a complete drain; empty causes have no slice, row or unknown count', () => {
  const data = destruction();
  data.causes = [cause({ source: { kind: 'destruction', reason: 'air_accident' }, domain: 'air', leaves: [] })];
  assert.equal(treeAt(data, 100).valueUnknown, true);
  assert.ok(!treeAt(data, 100).children.some(listed));
  assert.ok(!treeAt(data, 101).children.some(listed));
  assert.deepEqual(treeAt(data, 101).unknown, {});
  assert.deepEqual(unknownCauses(data, 101), []);
  assert.match(destructionCountingNote(data, 100), /Unavailable/);
  assert.equal(destructionCountingNote(data, 101), '');
  assert.equal(lossesTotal(lossItems(data, live(101))).incomplete, false);
});

test('each gap reason appears once without claiming a known number of uncounted parts', () => {
  const data = destruction();
  data.causes[0].unknownSpans = [
    { firstHour: 102, lastHour: 109, reason: 'destructionUnresolved', detail: 'Owner unresolved.' },
    { firstHour: 103, lastHour: 109, reason: 'destructionUnresolved', detail: 'Owner unresolved.' },
  ];
  assert.equal(unknownCauses(data, 105).length, 1);
  const root = treeAt(data, 105);
  const land = root.children.find(node => node.label === 'Mod combat')!;
  assert.deepEqual(land.unknown, {});
  assert.equal(land.notes.filter(note => note.includes('Owner unresolved.')).length, 0);
  assert.ok(!sliceTooltip(land, root, TEXT)[0].rows.some(row => row.label === 'Not counted'));
  assert.doesNotMatch(land.notes.join(' '), /lower bound from/);
});

test('distinct details of the same reason survive in cause notes and Unknown causes once each', () => {
  const data = destruction();
  data.causes[0].unknownSpans = [
    { firstHour: 102, lastHour: 109, reason: 'destructionUnresolved', detail: 'Country identity was unavailable.' },
    { firstHour: 103, lastHour: 109, reason: 'destructionUnresolved', detail: 'Raw owner had no CountryLink.' },
    { firstHour: 104, lastHour: 109, reason: 'destructionUnresolved', detail: 'Raw owner had no CountryLink.' },
    { firstHour: 102, lastHour: 109, reason: 'destructionGap', detail: 'A drain was lost.' },
    { firstHour: 104, lastHour: 109, reason: 'destructionGap', detail: 'A later interval was partial.' },
  ];
  const unknown = unknownCauses(data, 105);
  assert.equal(unknown.length, 1);
  assert.equal(new Set(unknown.map(item => item.id)).size, 1);
  const notes = treeAt(data, 105).children.find(node => node.label === 'Mod combat')!.notes.join(' ');
  for (const detail of ['Country identity was unavailable.', 'Raw owner had no CountryLink.', 'A drain was lost.', 'A later interval was partial.']) {
    assert.equal(notes.split(detail).length - 1, 0);
    assert.equal(unknown.filter(item => item.label.includes(detail)).length, 0);
  }
});

test('destruction without a counting start stays unavailable even with later refusal spans', () => {
  const data = destruction();
  data.destructionRuns = [{ firstHour: 100, lastHour: 109, countedFrom: null, withheldFrom: 102 }];
  data.destructionCoverage = [{ firstHour: 102, lastHour: 109, reason: 'Destruction table withheld after overflow.' }];
  for (const hour of [100, 105, 109]) assert.equal(destructionCountingNote(data, hour),
    'Unavailable');
  assert.deepEqual(destructionLegend(data, live(100), TOKENS), []);
  assert.equal(destructionLegend(data, live(105), TOKENS)[0].label, 'Incomplete');
  assert.equal(destructionLegend(data, whole, TOKENS)[0].label, 'Incomplete');
  assert.match(lossesFootnote(data, whole), /Unavailable/);
  data.destructionRuns = [{ firstHour: 100, lastHour: 109, countedFrom: null, withheldFrom: null }];
  data.destructionCoverage = [];
  assert.equal(destructionCountingNote(data, 100), 'Unavailable');
  assert.deepEqual(destructionLegend(data, live(105), TOKENS), []);
});

test('per-run boundaries separate contiguous branch loads, supplemental gaps and uncounted startup totals', () => {
  const data = destruction();
  data.from = 100;
  data.to = 250;
  data.restarts = [];
  data.destructionRuns = [
    { firstHour: 100, lastHour: 199, countedFrom: 101, withheldFrom: null },
    { firstHour: 200, lastHour: 250, countedFrom: 201, withheldFrom: null },
  ];
  data.destructionCoverage = [
    { firstHour: 101, lastHour: 139, reason: 'Supplemental hooks unavailable' },
    { firstHour: 140, lastHour: 199, reason: null },
    { firstHour: 201, lastHour: 250, reason: null },
  ];
  data.causes = [
    cause({
      source: { kind: 'destruction', reason: 'land_combat' },
      leaves: [
        leaf({ points: [{ hour: 101, totalRaw: '500000' }, { hour: 201, totalRaw: '200000' }] }),
        leaf({ design: null, key: 'parent-only', points: [{ hour: 101, totalRaw: '900000' }] }),
      ],
    }),
  ];
  assert.equal(validateLossesReply(data), data);

  assert.equal(destructionCountingNote(data, 120), '');
  assert.equal(destructionLegend(data, { first: 100, hour: 120, end: 250, campaign: false }, TOKENS)[0].label, 'Incomplete');

  assert.equal(destructionCountingNote(data, 150), '');
  assert.equal(destructionLegend(data, { first: 100, hour: 150, end: 250, campaign: false }, TOKENS)[0].label, 'Complete');
  assert.equal(lossesTotal(lossItems(data, { first: 100, hour: 150, end: 250, campaign: false })).exact, 1400000n);

  assert.equal(destructionCountingNote(data, 200), 'Unavailable');
  assert.deepEqual(destructionLegend(data, { first: 100, hour: 200, end: 250, campaign: false }, TOKENS), []);
  const startupItems = lossItems(data, { first: 100, hour: 200, end: 250, campaign: false });
  assert.equal(lossesTotal(startupItems).exact, null);
  assert.equal(lossesTotal(startupItems).unknownValue, true);

  assert.equal(destructionCountingNote(data, 201), '');
  assert.equal(destructionLegend(data, { first: 100, hour: 201, end: 250, campaign: false }, TOKENS)[0].label, 'Complete');
  assert.equal(lossesTotal(lossItems(data, { first: 100, hour: 201, end: 250, campaign: false })).exact, 200000n);
  assert.equal(destructionLegend(data, { first: 100, hour: 250, end: 250, campaign: true }, TOKENS)[0].label, 'Complete');
  assert.deepEqual(destructionCoverageHours(data, 100, 250), { recorded: 50, unknown: 0, reasons: [] });
});

test('exact read absence overrides a multi-hour chart bin without erasing the later loss', () => {
  const make = (exactHour: number) => {
    const data = destruction();
    data.from = 120;
    data.to = 130;
    data.width = 1;
    data.exactHours = [exactHour];
    data.destructionRuns = [{ firstHour: 120, lastHour: 130, countedFrom: 121, withheldFrom: null }];
    data.destructionCoverage = [
      { firstHour: 121, lastHour: 123, reason: 'supplemental hooks unavailable' },
      { firstHour: 124, lastHour: 130, reason: null },
    ];
    data.causes = [
      cause({
        source: { kind: 'destruction', reason: 'land_combat' },
        unknownSpans: [{ firstHour: 121, lastHour: 130, reason: 'destructionWithheld', detail: 'supplemental hooks unavailable' }],
        leaves: [leaf({ points: [{ hour: 121, totalRaw: '100000' }], exact: [{ hour: exactHour, totalRaw: null }] })],
      }),
    ];
    return data;
  };

  const at124 = make(124);
  assert.equal(validateLossesReply(at124), at124);
  assert.equal(pointAt(at124.causes[0].leaves[0], 124), null);
  assert.equal(lossesTotal(lossItems(at124, { first: 120, hour: 124, end: 130, campaign: false })).exact, null);
  assert.equal(lossesTotal(lossItems(at124, { first: 120, hour: 125, end: 130, campaign: false })).exact, 100000n);
  assert.equal(lossesTotal(lossItems(at124, { first: 120, hour: 130, end: 130, campaign: true })).exact, 100000n);

  const at121 = make(121);
  assert.equal(pointAt(at121.causes[0].leaves[0], 121), null);
  assert.equal(lossesTotal(lossItems(at121, { first: 120, hour: 121, end: 130, campaign: false })).exact, null);
  assert.equal(lossesTotal(lossItems(at121, { first: 120, hour: 125, end: 130, campaign: false })).exact, 100000n);
});

test('withHead records explicit exact absence for missing live cells and preserves 64-bit integer precision', () => {
  const missingBase = destruction();
  missingBase.causes = [
    cause({
      source: { kind: 'destruction', reason: 'land_combat' },
      leaves: [leaf({ points: [{ hour: 101, totalRaw: '200000' }] })],
    }),
  ];
  const missingHead = withHead(missingBase, 105, () => 'Not recorded at this hour');
  assert.equal(pointAt(missingHead.causes[0].leaves[0], 105), null);
  assert.deepEqual(missingHead.causes[0].leaves[0].exact, [{ hour: 105, totalRaw: null }]);
  assert.equal(lossesTotal(lossItems(missingHead, live(105))).exact, null);
  assert.deepEqual(missingHead.causes[0].leaves[0].points, [{ hour: 101, totalRaw: '200000' }]);

  const preciseBase = destruction();
  preciseBase.causes = [
    cause({
      source: { kind: 'destruction', reason: 'land_combat' },
      leaves: [leaf({ points: [{ hour: 105, totalRaw: '9007199254740993' }], exact: [{ hour: 105, totalRaw: '9007199254740993' }] })],
    }),
  ];
  const preciseHead = withHead(preciseBase, 105, () => Number(9007199254740993n));
  assert.equal(pointAt(preciseHead.causes[0].leaves[0], 105)?.totalRaw, '9007199254740993');
  assert.equal(lossesTotal(lossItems(preciseHead, live(105))).exact, 9007199254740993n);

  const betweenBase = destruction();
  betweenBase.causes = [
    cause({
      source: { kind: 'destruction', reason: 'land_combat' },
      leaves: [leaf({ points: [{ hour: 101, totalRaw: '9007199254740993' }], exact: [] })],
    }),
  ];
  const exactBigIntHead = withHead(betweenBase, 106, () => 9007199254740993n);
  assert.equal(pointAt(exactBigIntHead.causes[0].leaves[0], 106)?.totalRaw, '9007199254740993');
  assert.equal(lossesTotal(lossItems(exactBigIntHead, live(106))).exact, 9007199254740993n);

  const roundedFloatHead = withHead(betweenBase, 106, () => Number(9007199254740993n));
  assert.equal(pointAt(roundedFloatHead.causes[0].leaves[0], 106), null);
  assert.equal(lossesTotal(lossItems(roundedFloatHead, live(106))).exact, null);

  const fallbackHeadJson = JSON.parse(JSON.stringify({
    leaves: ['0'],
    warnings: [],
    live: {
      groups: [{
        table: 70,
        columns: [0],
        keys: ['0'],
        values: [Number(9007199254740993n)],
        rawValues: ['9007199254740993'],
      }],
      rows: ['0'],
      kinds: [{ id: 'total', op: 'value', scale: 1 }],
      cells: [0, 0, 1, 0, 0, 0],
    },
  }));
  const fallbackOverlay = atHead(betweenBase, 106, repliedCells(fallbackHeadJson).read(), new Set(fallbackHeadJson.leaves));
  assert.equal(pointAt(fallbackOverlay.causes[0].leaves[0], 106)?.totalRaw, '9007199254740993');
  assert.equal(lossesTotal(lossItems(fallbackOverlay, live(106))).exact, 9007199254740993n);
  assert.deepEqual(fallbackOverlay.causes[0].unknownSpans, []);

  const coarseZero = destruction();
  coarseZero.from = 120;
  coarseZero.to = 130;
  coarseZero.width = 1;
  coarseZero.exactHours = [124];
  coarseZero.destructionRuns = [{ firstHour: 120, lastHour: 130, countedFrom: 121, withheldFrom: null }];
  coarseZero.destructionCoverage = [{ firstHour: 121, lastHour: 130, reason: null }];
  coarseZero.causes = [
    cause({
      source: { kind: 'destruction', reason: 'land_combat' },
      leaves: [leaf({ points: [{ hour: 121, totalRaw: '100000' }, { hour: 125, totalRaw: '100000' }], exact: [{ hour: 124, totalRaw: '0' }] })],
    }),
  ];
  assert.equal(lossesTotal(lossItems(coarseZero, { first: 120, hour: 124, end: 130, campaign: false })).exact, 0n);
  assert.equal(lossesTotal(lossItems(coarseZero, { first: 120, hour: 125, end: 130, campaign: false })).exact, 100000n);
  assert.equal(lossesTotal(lossItems(coarseZero, { first: 120, hour: 126, end: 130, campaign: false })).exact, 100000n);
});

test('before counting starts destruction coverage has no rows in either land mode', () => {
  const data = destruction();
  assert.deepEqual(coverageLegend(data, live(100), TOKENS), []);
  assert.deepEqual(destructionLegend(data, live(100), TOKENS), []);
  data.landSource = 'combatLog';
  data.coverage = [{ firstHour: 100, lastHour: 109, coverage: 'logged' }];
  assert.equal(coverageLegend(data, live(100), TOKENS)[0].label, 'Complete');
  assert.deepEqual(destructionLegend(data, live(100), TOKENS), []);
});

test('unresolved cause labels and dated lend-lease notes stay on their own items', () => {
  let data = reply();
  data.causes = [data.causes[0]];
  data.causes[0].name.text = null;
  let root = treeAt(data, 105);
  assert.equal(root.children[0].label, 'Mod combat');
  assert.doesNotMatch(root.notes.join(' '), /no fully resolved cause label/);
  assert.equal(root.exact, 350001n);
  data = reply();
  data.causes = [data.causes[0]];
  data.causes[0].source = { kind: 'lendLeaseSunk' };
  data.causes[0].domain = 'other';
  data.causes[0].unknownSpans = [{ firstHour: 105, lastHour: 109, reason: 'endedAfterLastRead', detail: null }];
  data.causes[0].leaves[0].lowerBoundFrom = 105;
  root = treeAt(data, 105);
  const causeNode = root.children[0];
  const design = causeNode.children[0].children[0];
  assert.doesNotMatch(sliceTooltip(design, root, TEXT)[0].note ?? '', /Incomplete from Hour/);
  for (const node of [root, causeNode, causeNode.children[0]]) assert.doesNotMatch(node.notes.join(' '), /Incomplete from Hour 105/);
  assert.match(root.notes.join(' '), /Incomplete total/);
  assert.equal(root.exact, 350001n);
  assert.equal(root.count, 1);
  const items = lossItems(data, live(105));
  for (const levels of [[false, true, true], [true, true, false], [false, false, false]]) {
    const hidden = groupTree(items, ROOT, levels, lossesTotal(items));
    assert.doesNotMatch(hidden.notes.join(' '), /no fully resolved cause label|A lower bound from/);
  }
});

test('a view containing only unknown amounts has no fabricated zero subtotal', () => {
  const data = destruction();
  data.causes = [cause({ source: { kind: 'destruction', reason: 'air_accident' }, domain: 'air', leaves: [],
    unknownSpans: [{ firstHour: 102, lastHour: 109, reason: 'destructionGap', detail: 'A drain was lost.' }] })];
  const root = treeAt(data, 105);
  assert.equal(amountText(root, TEXT), 'Not known');
  assert.equal(root.exact, null);
});

test('an unstarted destruction source has no fabricated zero subtotal', () => {
  const data = destruction();
  data.causes = [cause({ source: { kind: 'destruction', reason: 'land_combat' }, leaves: [] })];
  assert.equal(lossesTotal(lossItems(data, live(100))).exact, null);
  assert.equal(lossesTotal(lossItems(data, live(101))).exact, 0n);
});

test('Combat Log keeps land states and exposes shared air destruction coverage separately', () => {
  const data = reply();
  data.destructionCoverage = [{ firstHour: 100, lastHour: 103, reason: null },
    { firstHour: 104, lastHour: 109, reason: 'A drain was lost.' }];
  assert.equal(coverageLegend(data, live(105), TOKENS)[0].label, 'Complete');
  assert.equal(destructionLegend(data, live(105), TOKENS)[0].label, 'Incomplete');
  assert.deepEqual(destructionLegend(data, whole, TOKENS).map(item => item.label), ['Incomplete']);
});

test('resolved cause labels keep game templates out of their slice notes', () => {
  const data = reply();
  data.causes[0].name = { key: 'FIXTURE_RESOLVED_LOSS_HINT', template: 'A completed source label for this loss category', text: 'Sunk ships' };
  const notes = (tree: ReturnType<typeof treeAt>) => sliceTooltip(tree.children.find(node => node.label === 'Sunk ships')!, tree, TEXT)[0].note ?? '';
  assert.doesNotMatch(notes(treeAt(data, 105)), /Game text/);
  data.causes[0].name = { key: 'FIXTURE_PENDING_LOSS_HINT', template: 'Synthetic category amount: $AMOUNT|H$', text: 'Sunk ships' };
  assert.doesNotMatch(notes(treeAt(data, 105)), /Game text/);
});


test('negative corrections retain exact digits, legend access and zero arc geometry through folding', () => {
  const data = reply(); data.causes = [data.causes[0]];
  data.causes[0].leaves[0].points = [{ hour: 100, totalRaw: '-9007199254741499' }];
  assert.equal(validateLossesReply(data), data);
  const root = treeAt(data, 100);
  const part = root.children[0];
  assert.equal(root.exact, -9007199254741499n);
  assert.equal(root.value, Number(-9007199254741499n));
  assert.equal(listed(part), true);
  assert.equal(arcTargets(root, root, 'share').get(part.id)!.shown, false);
  const text = TEXT;
  assert.equal(amountText(part, text), 'Returned: \u221290,071,992,547');
  assert.match(sunburstSummary(root, text), /Returned:.*90,071,992,547/);
  assert.equal(sliceTooltip(part, root, text)[0].rows.some(row => row.label.startsWith('Of ')), false);
  const folded = foldTree(root, 1);
  assert.equal(folded.exact, root.exact);
});

test('a missing signed interval is an incomplete subtotal rather than a mathematical lower bound', () => {
  const data = reply(); data.landSource = 'destruction'; data.landLowerBound = null;
  data.causes = [cause({ source: { kind: 'destruction', reason: 'land_combat' },
    leaves: [leaf({ points: [{ hour: 100, totalRaw: '-1200000' }], lowerBoundFrom: 102 })],
    unknownSpans: [{ firstHour: 102, lastHour: 109, reason: 'destructionGap', detail: 'Missing drain' }] })];
  const root = treeAt(data, 105);
  const text = TEXT;
  assert.match(amountText(root, text), /^Returned: \u221212/);
  assert.equal(lossesTotal(lossItems(data, live(105))).incomplete, true);
  assert.equal(root.finalUnknown, true);
  assert.doesNotMatch(amountText(root, text), /at least/);
  assert.match(unknownCauses(data, 105)[0].label, /Incomplete/);
  assert.equal(root.exact, -1200000n);
});


test('a measured subtotal keeps exact signed data and Fill uncertainty without claiming full-total percentages', () => {
  const data = destruction();
  data.causes[0].unknownSpans = [{ firstHour: 104, lastHour: 109, reason: 'destructionGap', detail: 'Missing native contribution' }];
  const items = lossItems(data, live(105)), total = lossesTotal(items);
  const root = groupTree(items, ROOT, [], total);
  const text = { ...TEXT, shares: !total.incomplete };
  assert.equal(root.exact, 450001n);
  assert.equal(root.finalUnknown, true);
  assert.equal(arcTargets(root, root, 'share').get(root.children[0].id)!.shown, true);
  assert.equal(sliceTooltip(root.children[0], root, text)[0].rows.some(row => row.label.startsWith('Of ')), false);
  assert.doesNotMatch(sunburstSummary(root, text), /%/);
});

test('the keyboard steps through sunk ships beside the series, the nearer answer winning', () => {
  const hours = [105, 110, 120];
  assert.equal(stepHours(hours, 'Home', null, 100, 130), 105);
  assert.equal(stepHours(hours, 'End', null, 100, 115), 110);
  assert.equal(stepHours(hours, 'ArrowRight', 110, 100, 130), 120);
  assert.equal(stepHours(hours, 'ArrowLeft', 110, 100, 130), 105);
  assert.equal(stepHours(hours, 'ArrowLeft', 105, 100, 130), null);
  assert.equal(nearest('ArrowRight', 118, 120), 118);
  assert.equal(nearest('ArrowLeft', 118, null), 118);
  assert.equal(nearest('End', 118, 120), 120);
});

test('ships count the most one read listed, dated at their sinking whichever read listed them', () => {
  const leaves = shipLeaves([
    { signature: 'a', listedHour: 100, sinkHour: 90, design: 0, count: 1 },
    { signature: 'a', listedHour: 104, sinkHour: 90, design: 0, count: 2 },
    { signature: 'b', listedHour: 108, sinkHour: 95, design: 0, count: 1 },
  ], 100000);
  assert.deepEqual(leaves.map(leaf => leaf.points), [[{ hour: 90, totalRaw: '200000' }, { hour: 95, totalRaw: '300000' }]]);
  assert.equal(withEdges('Home', 120, null, 100), 100);
  assert.equal(withEdges('End', null, null, 100), 100);
  assert.equal(withEdges('ArrowLeft', null, 110, 100), 100);
  assert.equal(withEdges('ArrowRight', null, 110, 100), 110);
  assert.equal(withEdges('ArrowRight', 115, 110, 100), 115);
});

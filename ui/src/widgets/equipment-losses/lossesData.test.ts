import assert from 'node:assert/strict';
import test from 'node:test';
import type { Coverage, LossCause, LossLeaf, LossesReply } from '../../generated/dashboard';
import { amountText, arcTargets, foldTree, groupTree, listed, sliceColours, sliceTooltip, sunburstSummary, walk } from '../../charts/sunburst/layout.ts';
import { resolveOptions } from '../../dashboard/options.ts';
import { sunburstSettings, levelsShown } from '../../charts/sunburst/sunburstOptions.ts';
import { definition, OPTIONS } from './definition.ts';
import { LEVELS, causeName, destructionCoverageAt, destructionCoverageHours, coverageAt, coverageHours, coverageLegend, destructionLegend, destructionCountingNote, formatLosses, landNote, lossDetails, lossItems, lossesTotal, lossesFootnote, lossKey, pointAt, unknownCauses, validateLossesReply } from './lossesData.ts';

const ROOT = { id: 'all', label: 'All losses' };
const TOKENS = { positive: '#00aa00', warning: '#aaaa00', error: '#aa0000', surface: '#000000', text: '#ffffff' };
const live = (hour: number) => ({ first: 100, hour, end: 109, campaign: false });
const whole = { ...live(109), campaign: true };
const leaf = (extra: Partial<LossLeaf> = {}): LossLeaf => ({ design: 0, points: [{ hour: 100, totalRaw: '200000' }, { hour: 105, totalRaw: '350001' }], lowerBoundFrom: null, ...extra });
const cause = (extra: Partial<LossCause> = {}): LossCause => ({ source: { kind: 'combatLog', reason: 0 }, domain: 'land', name: { key: 'mod_loss_combat', template: 'Mod combat', text: 'Mod combat' },
  group: null, loss: true, unknown: null, unknownSpans: [], leaves: [leaf()], ...extra });
const reply = (): LossesReply => ({
  branchId: 'mod-branch', tag: 'opaque: tag / 1', amountScale: 100_000, landSource: 'combatLog', landLowerBound: null, destructionCountedFrom: 100, destructionCoverage: [{ firstHour: 100, lastHour: 109, reason: null }], retainedMonths: 7, warnings: [],
  designs: [{ variant: 91001, equipment: 'mod_walkers_v2', archetype: 'mod_walkers', archetypeName: 'Walking machines',
    names: [{ firstHour: 100, name: 'First walker', known: true, design: 'fingerprint-one' }, { firstHour: 105, name: 'Renamed walker', known: true, design: 'fingerprint-one' }] }],
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
    (bad: LossesReply) => { bad.designs[0].names.reverse(); },
    (bad: LossesReply) => { bad.designs[0].names[0].design = ''; },
    (bad: LossesReply) => { bad.coverage[0].lastHour = 99; },
    (bad: LossesReply) => { bad.coverage.push({ firstHour: 105, lastHour: 110, coverage: 'unknown' }); },
    (bad: LossesReply) => { bad.causes[2].unknown = 'Not available.'; bad.causes[2].leaves = [leaf()]; },
    (bad: LossesReply) => { bad.amountScale = 3; },
    (bad: LossesReply) => { bad.destructionCountedFrom = NaN; },
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
  assert.equal(amountText(campaign, TEXT), '4.50001');
  assert.equal(treeAt(data, 100).final, campaign.value);
  assert.equal(formatLosses(Number(9007199254741499n), 9007199254741499n, 100_000), '90,071,992,547.41499');
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
  data.designs[0].names = [{ firstHour: 105, name: 'Future name', known: true, design: null }];
  assert.equal(lossItems(data, live(104))[1].path[2].label, 'Design 91001 (name not known)');
  data.designs[0].names.unshift({ firstHour: 100, name: null, known: false, design: null });
  assert.equal(lossItems(data, live(104))[1].path[2].label, 'Design 91001 (name not known)');
  data.designs[0].archetypeName = null;
  assert.equal(lossItems(data, live(104))[1].path[1].label, 'Equipment type name unavailable');
});

test('each hourly land state has its legend entry, shared status colour and Partial exact N', () => {
  const states: Coverage[] = ['logged', { partial: { ungrouped: 17 } }, 'notLogged', 'unknown'];
  const labels = ['Logged', 'Partial: 17 divisions outside armies', 'Not logged', 'Unknown'];
  const colours = [TOKENS.positive, TOKENS.warning, TOKENS.error, '#1a1a1a'];
  for (const [index, coverage] of states.entries()) {
    const data = reply(); data.coverage[0].coverage = coverage;
    const entries = coverageLegend(data, live(105), TOKENS);
    assert.equal(entries.length, 1);
    assert.equal(entries[0].label, labels[index]);
    assert.equal(entries[0].color, colours[index]);
    assert.deepEqual(coverageAt(data, 105), coverage);
    assert.equal(treeAt(data, 105).exact, 450001n); // Coverage never discards captured losses.
    if (index === 0) assert.equal(landNote(data, 100, 105), undefined);
    else assert.match(lossesFootnote(data, live(105)), index === 3 ? /unknown/ : /incomplete/);
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
  assert.equal(coverageLegend(data, live(103), TOKENS)[0].label, 'Partial: 3 divisions outside armies');
  assert.equal(coverageLegend(data, live(104), TOKENS)[0].label, 'Partial: 19 divisions outside armies');
  assert.deepEqual(coverageLegend(data, whole, TOKENS).map(entry => entry.label), ['Logged: 2 hours', 'Partial: 4 hours', 'Not logged: 2 hours', 'Unknown: 2 hours']);
  assert.deepEqual(coverageHours(data, 101, 108), { logged: 1, partial: 4, notLogged: 2, unknown: 1 });
});

test('missing coverage hours and unobserved retention are Unknown, with the explicit reason', () => {
  const data = reply(); data.coverage = [
    { firstHour: 100, lastHour: 102, coverage: 'logged' },
    { firstHour: 105, lastHour: 109, coverage: 'pruneUnobserved' },
  ];
  assert.equal(coverageAt(data, 104), 'unknown');
  assert.deepEqual(coverageHours(data, 100, 109), { logged: 3, partial: 0, notLogged: 0, unknown: 7 });
  assert.equal(coverageLegend(data, live(105), TOKENS)[0].label, 'Unknown: retention was not observed');
  assert.match(landNote(data, 100, 105)!, /retention was not observed/);
  data.coverage = [];
  assert.equal(coverageLegend(data, whole, TOKENS)[0].label, 'Unknown: 10 hours');
});

test('a later Logged hour keeps cumulative land losses explicitly incomplete', () => {
  const data = reply(); data.coverage = [{ firstHour: 100, lastHour: 103, coverage: { partial: { ungrouped: 2 } } },
    { firstHour: 104, lastHour: 109, coverage: 'logged' }];
  assert.equal(coverageLegend(data, live(105), TOKENS)[0].label, 'Logged');
  const root = treeAt(data, 105);
  const land = root.children.find(node => node.label === 'Mod combat')!;
  for (const node of walk(land)) assert.match(node.notes.join(' '), /Land losses are incomplete/);
  assert.match(lossesFootnote(data, live(105)), /incomplete/);
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
  assert.deepEqual(unknownCauses(data, 105).map(entry => entry.label), ['Mod combat: Not known — Content retention deletes changing entries.',
    'Cause name unavailable: Not known — Destruction table is unavailable.']);
  assert.equal(root.exact, 100000n);
});

test('unread spans never become zero; they mark the cause once and stop when it is read again', () => {
  const data = reply(); data.causes[0].unknownSpans = [{ firstHour: 103, lastHour: 104, reason: 'unread', detail: null }];
  assert.equal(treeAt(data, 104).exact, 300000n);
  const unknown = treeAt(data, 104).children.find(node => node.label === 'Mod combat')!;
  assert.deepEqual(unknown.unknown, {});
  assert.ok(!sliceTooltip(unknown, treeAt(data, 104), TEXT)[0].rows.some(row => row.label === 'Not counted'));
  assert.match(unknown.notes.join(' '), /not read/);
  assert.match(unknownCauses(data, 104)[0].label, /cause not read/);
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
  for (const node of walk(lease)) assert.match(node.notes.join(' '), /recorded subtotal/i);
  assert.equal(amountText(root, TEXT), '4.50001');
  assert.equal(root.finalUnknown, true);
  assert.match(sliceTooltip(lease, root, TEXT)[0].note!, /recorded subtotal/i);
  const folded = foldTree(groupTree(lossItems(data, live(105)), ROOT), 1);
  assert.match(folded.children.find(node => node.other)!.notes.join(' '), /recorded subtotal/i);
  data.causes[0].leaves = [];
  assert.equal(lossesTotal(lossItems(data, live(105))).incomplete, true); // An ended exchange can hide losses with no known sunk leaf.
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

test('date details name recorded changes and do not invent equal later read dates', () => {
  const data = reply(); const root = treeAt(data, 109);
  const land = root.children.find(node => node.label === 'Mod combat')!;
  assert.deepEqual(lossDetails(land, data, 109), [{ label: 'Last recorded change', value: 'Hour 105', tone: 'muted' }]);
  assert.equal(lossDetails(root, data, 109)[0].value, 'Hour 104 - Hour 105');
});

test('all three levels are optional and default behaviour uses the shared sunburst settings', () => {
  const options = resolveOptions(OPTIONS, undefined);
  assert.deepEqual(levelsShown(options, LEVELS), [true, true, true]);
  assert.deepEqual(sunburstSettings(options), { growth: 'share', labels: true, legend: true, breadcrumb: true, dimming: true });
  assert.equal(definition.defaultTime, 'campaign');
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
  data.destructionCountedFrom = 101;
  data.destructionCoverage = [{ firstHour: 101, lastHour: 109, reason: null }];
  data.causes[0].source = { kind: 'destruction', reason: 'land_combat' };
  data.causes[0].leaves[0].points[0].hour = 101;
  data.causes.pop();
  return data;
};

test('Land losses from uses a shared options row and defaults to Every destruction', () => {
  const field = OPTIONS.find(option => option.key === 'landSource')!;
  assert.equal(field.section, 'options');
  assert.equal(field.label, 'Land losses from');
  assert.equal(resolveOptions(OPTIONS).landSource, 'destruction');
  assert.equal(resolveOptions(OPTIONS, { landSource: 'combatLog' }).landSource, 'combatLog');
  assert.equal(resolveOptions(OPTIONS, { landSource: 'bad' }).landSource, 'destruction');
  assert.equal(ROOT.label, 'All losses');
  assert.equal(TEXT.unit, 'units');
});

test('destruction coverage uses Recorded and Unknown with the drain reason and hours per state', () => {
  const data = destruction();
  assert.equal(validateLossesReply(data), data);
  assert.equal(destructionCoverageAt(data, 100), 'No destruction drain was recorded at this hour.');
  assert.equal(destructionCoverageAt(data, 101), null);
  assert.deepEqual(coverageLegend(data, live(100), TOKENS), []);
  assert.deepEqual(coverageLegend(data, live(105), TOKENS), [{ id: 'recorded', color: TOKENS.positive, label: 'Recorded' }]);
  assert.deepEqual(destructionCoverageHours(data, 100, 109), { recorded: 9, unknown: 0, reasons: [] });
  assert.deepEqual(coverageLegend(data, whole, TOKENS).map(entry => entry.label), ['Recorded: 9 hours']);
  data.destructionCoverage = [{ firstHour: 103, lastHour: 104, reason: null },
    { firstHour: 105, lastHour: 109, reason: 'Destruction table withheld after overflow.' }];
  assert.match(coverageLegend(data, live(101), TOKENS)[0].label, /Unknown: No destruction drain/);
  assert.deepEqual(destructionCoverageHours(data, 100, 109), { recorded: 2, unknown: 7,
    reasons: ['Destruction table withheld after overflow.', 'No destruction drain was recorded at this hour.'] });
  assert.match(coverageLegend(data, whole, TOKENS)[1].label, /overflow/);
});

test('complete supplemental destruction has no permanent deletion or paradrop exclusion', () => {
  const data = destruction();
  const root = treeAt(data, 105);
  assert.equal(lossesTotal(lossItems(data, live(105))).incomplete, false);
  assert.equal(amountText(root, TEXT), '4.50001');
  assert.equal(landNote(data, 100, 105), undefined);
  assert.match(lossesFootnote(data, whole), /Destruction counted from Hour 101/);
  assert.doesNotMatch(root.notes.join(' '), /Not included/);
});

test('destruction unknown intervals never become zero; known subtotals remain incomplete with named reasons', () => {
  for (const reason of ['destructionWithheld', 'destructionGap', 'destructionUnresolved', 'destructionDomain'] as const) {
    const data = destruction();
    data.causes[0].unknownSpans = [{ firstHour: 100, lastHour: 109, reason, detail: `Named ${reason} reason.` }];
    const root = treeAt(data, 105);
    const land = root.children.find(node => node.label === 'Mod combat')!;
    assert.equal(land.exact, 350001n);
    assert.match(land.notes.join(' '), /recorded subtotal/i);
    assert.match(land.notes.join(' '), new RegExp(`Named ${reason}`));
    assert.deepEqual(land.unknown, {});
    assert.ok(!sliceTooltip(land, root, TEXT)[0].rows.some(row => row.label === 'Not counted'));
    assert.match(unknownCauses(data, 105)[0].label, /recorded subtotal; total Not known/);
    data.causes[0].leaves = [];
    const empty = treeAt(data, 105).children.find(node => node.label === 'Mod combat')!;
    assert.equal(empty.unmeasured, true);
    assert.equal(sliceTooltip(empty, treeAt(data, 105), TEXT)[0].rows[0].value, 'Not known');
    assert.match(unknownCauses(data, 105)[0].label, /Not known/);
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
      assert.doesNotMatch(air.notes.join(' '), /recorded subtotal/i);
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
    assert.match(unknownCauses(data, 105).find(item => item.label.startsWith('Air accidents'))!.label, /lost drain/);
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
  assert.doesNotMatch(unrelated.notes.join(' '), /recorded subtotal/i);
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
  assert.match(destructionCountingNote(data, 100), /has not started/);
  assert.equal(destructionCountingNote(data, 101), 'Destruction counted from Hour 101.');
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
  assert.equal(land.notes.filter(note => note.includes('Owner unresolved.')).length, 1);
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
  assert.equal(unknown.length, 4);
  assert.equal(new Set(unknown.map(item => item.id)).size, 4);
  const notes = treeAt(data, 105).children.find(node => node.label === 'Mod combat')!.notes.join(' ');
  for (const detail of ['Country identity was unavailable.', 'Raw owner had no CountryLink.', 'A drain was lost.', 'A later interval was partial.']) {
    assert.equal(notes.split(detail).length - 1, 1);
    assert.equal(unknown.filter(item => item.label.includes(detail)).length, 1);
  }
});

test('withheld destruction before a complete drain says it was not recorded on the branch', () => {
  const data = destruction();
  data.destructionCountedFrom = null;
  data.destructionCoverage = [{ firstHour: 102, lastHour: 109, reason: 'Destruction table withheld after overflow.' }];
  for (const hour of [100, 105, 109]) assert.equal(destructionCountingNote(data, hour),
    'Destruction was not recorded on this branch: Destruction table withheld after overflow.');
  assert.doesNotMatch(lossesFootnote(data, whole), /has not started/);
  data.destructionCoverage = [];
  assert.equal(destructionCountingNote(data, 100), 'Destruction counting has not started at this moment.');
});

test('before counting starts destruction coverage has no rows in either land mode', () => {
  const data = destruction();
  assert.deepEqual(coverageLegend(data, live(100), TOKENS), []);
  assert.deepEqual(destructionLegend(data, live(100), TOKENS), []);
  data.landSource = 'combatLog';
  data.coverage = [{ firstHour: 100, lastHour: 109, coverage: 'logged' }];
  assert.equal(coverageLegend(data, live(100), TOKENS)[0].label, 'Logged');
  assert.deepEqual(destructionLegend(data, live(100), TOKENS), []);
});

test('unresolved cause labels and dated lend-lease notes stay on their own items', () => {
  let data = reply();
  data.causes = [data.causes[0]];
  data.causes[0].name.text = null;
  let root = treeAt(data, 105);
  assert.match(lossDetails(root.children[0], data, 105).map(row => row.value).join(' '), /no fully resolved cause label/);
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
  assert.match(lossDetails(design, data, 105).map(row => row.value).join(' '), /Hour 105/);
  for (const node of [root, causeNode, causeNode.children[0]]) assert.doesNotMatch(node.notes.join(' '), /Incomplete from Hour 105/);
  assert.match(root.notes.join(' '), /An exchange ended after its last read/);
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

test('Combat Log keeps land states and exposes shared air destruction coverage separately', () => {
  const data = reply();
  data.destructionCoverage = [{ firstHour: 100, lastHour: 103, reason: null },
    { firstHour: 104, lastHour: 109, reason: 'A drain was lost.' }];
  assert.equal(coverageLegend(data, live(105), TOKENS)[0].label, 'Logged');
  assert.equal(destructionLegend(data, live(105), TOKENS)[0].label, 'Unknown: A drain was lost.');
  assert.deepEqual(destructionLegend(data, whole, TOKENS).map(item => item.label), ['Recorded: 4 hours', 'Unknown: 6 hours — A drain was lost.']);
});

test('an app label keeps resolved source text in its tooltip, never a template with fill-ins', () => {
  const data = reply();
  data.causes[0].name = { key: 'FIXTURE_RESOLVED_LOSS_HINT', template: 'A completed source label for this loss category', text: 'Sunk ships' };
  const notes = (tree: ReturnType<typeof treeAt>) => lossDetails(tree.children.find(node => node.label === 'Sunk ships')!, data, 105).map(row => `${row.label}: ${row.value}`).join(' ');
  assert.match(notes(treeAt(data, 105)), /Game text: A completed source label for this loss category/);
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
  assert.equal(amountText(part, text), 'Correction: \u221290,071,992,547.41499 (the game returned equipment)');
  assert.match(sunburstSummary(root, text), /Correction:.*90,071,992,547/);
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
  assert.match(amountText(root, text), /^Correction: \u221212/);
  assert.equal(lossesTotal(lossItems(data, live(105))).incomplete, true);
  assert.equal(root.finalUnknown, true);
  assert.doesNotMatch(amountText(root, text), /at least/);
  assert.match(unknownCauses(data, 105)[0].label, /total Not known/);
  assert.equal(root.exact, -1200000n);
});


test('a measured subtotal keeps exact signed data and Fill uncertainty without claiming full-total percentages', () => {
  const data = destruction();
  data.causes[0].unknownSpans = [{ firstHour: 104, lastHour: 109, reason: 'destructionGap', detail: 'Missing native contribution' }];
  const items = lossItems(data, live(105)), total = lossesTotal(items);
  const root = groupTree(items, ROOT, [], total);
  const text = { ...TEXT, shares: !total.incomplete };
  assert.equal(amountText(root, text), '4.50001');
  assert.equal(root.finalUnknown, true);
  assert.equal(arcTargets(root, root, 'share').get(root.children[0].id)!.shown, true);
  assert.equal(sliceTooltip(root.children[0], root, text)[0].rows.some(row => row.label.startsWith('Of ')), false);
  assert.doesNotMatch(sunburstSummary(root, text), /%/);
});

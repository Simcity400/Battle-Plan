import test from 'node:test';
import assert from 'node:assert/strict';
import { countryKey } from '../../dashboard/countryKey.ts';
import { createBoundedReadView } from '../../families/boundedView.ts';
import {
  displayedHour,
  type BranchToHourSourceRead,
  type FamilyPlanContext,
  type IdentityProvider,
} from '../../families/contract.ts';
import type { ExactHoverSourceRead } from '../../families/plan.ts';
import { deriveTimeChartTooltip } from '../../families/time-chart/hitTest.ts';
import { layoutTimeChart } from '../../families/time-chart/layout.ts';
import { DEFAULT_LINE_SETTINGS } from '../../families/time-chart/settings.ts';
import type { RaidTallyAtReply, RaidTallyReply } from '../../generated/dashboard';
import { TIME_CHART_VIEWS } from '../manifest/time-chart.ts';
import {
  raidTallyLinesDeclaration,
  raidTallyLinesProjection,
  raidTallyProof,
  unresolvedSlotLabel,
  validateRaidTallyReply,
} from './raidTallyLinesProjection.ts';
import { validateRaidTallyAt } from './raidTallyRaceProjection.ts';

function fakeIdentity(slots: Record<number, { tag: string; name: string; colour: string }>): IdentityProvider {
  return {
    resolve(kind, key, atHour) {
      const slot = key.startsWith('\u0000') ? Number(key.slice(1)) : NaN;
      const found = slots[slot];
      if (!found) {
        return {
          kind,
          key,
          atHour,
          holder: { status: 'ready', value: key },
          name: { status: 'absent', reason: 'tag not recorded' },
          tag: { status: 'absent', reason: 'tag not recorded' },
          colour: { status: 'absent', reason: 'tag not recorded' },
          flagChoice: { status: 'absent', reason: 'tag not recorded' },
          validInterval: [0, Infinity],
          nextChangeAt: Infinity,
        };
      }
      return {
        kind,
        key,
        atHour,
        holder: { status: 'ready', value: key },
        name: { status: 'ready', value: found.name },
        tag: { status: 'ready', value: found.tag },
        colour: { status: 'ready', value: found.colour },
        flagChoice: { status: 'ready', value: found.tag },
        validInterval: [0, Infinity],
        nextChangeAt: Infinity,
      };
    },
  };
}

test('raid-tally lines manifest entry, canonical slot plan, causal hour params, and nextChange proof', () => {
  const entry = TIME_CHART_VIEWS.find(v => v.kind === 'raid-tally' && v.view === 'lines');
  assert.ok(entry, 'raid-tally:lines is registered in TIME_CHART_VIEWS');
  assert.equal(entry.family, 'time-chart');

  const slot7 = countryKey(7);
  const ctx = {
    track: { source: { id: 's1', revision: 'r1' }, axisOffset: 0 },
    branch: { id: '1', firstHour: 100, lastHour: 140, gaps: [] },
    options: { metric: 'both', endLabels: true },
    countrySubject: null,
    countryName: '',
    countries: {
      all: false,
      tags: [slot7],
      holders: null,
      every: [slot7],
      top: null,
      membership: null,
      membershipRuns: null,
      unobserved: [],
      filter: { kind: 'all' },
      labels: {},
      factions: null,
      factionUnobserved: [],
      known: () => true,
      unobservedIn: () => false,
    },
    following: false,
    aux: undefined,
    gateSpan: (a: number, b: number) => ({ firstHour: a, lastHour: b }),
    staticSpan: null,
  } as unknown as FamilyPlanContext;

  const plan = raidTallyLinesDeclaration.sources(ctx, { width: 600, height: 260, following: false, staticSpan: [100, 140] });
  const tally = plan.entries.find(e => e.name === 'tally')?.read as BranchToHourSourceRead;
  const hover = plan.entries.find(e => e.name === 'hover')?.read as ExactHoverSourceRead;
  assert.equal(tally.shape, 'branch-to-hour');
  assert.equal(tally.product, 'raid-tally');
  assert.deepEqual(tally.params!.countries, [7], 'requests canonical slot number rather than tag string');
  assert.deepEqual(tally.dynamicKeys, ['hour']);
  assert.equal(
    tally.buildSpanParams!(
      tally.params!,
      { firstHour: 100, lastHour: 120 } as Parameters<NonNullable<BranchToHourSourceRead['buildSpanParams']>>[1],
    )!.hour,
    120,
  );
  assert.equal(hover.shape, 'exact-hover');
  assert.equal(hover.product, 'raid-tally-at');
  assert.deepEqual(hover.params!.countries, [7]);
});

test('raid-tally lines breaks across unread Raids spans, keeps distinct unresolved slots, and answers earlier-hour exact hover', () => {
  const reply: RaidTallyReply = {
    hour: 130,
    nextChange: 135,
    available: true,
    unavailable: null,
    unattributed: 1,
    series: [
      {
        slot: 7,
        runs: [
          { firstHour: 100, lastHour: 109, launched: 1, succeeded: 0 },
          { firstHour: 120, lastHour: 130, launched: 3, succeeded: 2 },
        ],
        membership: null,
      },
      {
        slot: 11,
        runs: [
          { firstHour: 100, lastHour: 109, launched: 0, succeeded: 0 },
          { firstHour: 120, lastHour: 130, launched: 2, succeeded: 1 },
        ],
        membership: null,
      },
      {
        slot: 12,
        runs: [
          { firstHour: 100, lastHour: 109, launched: 0, succeeded: 0 },
          { firstHour: 120, lastHour: 130, launched: 1, succeeded: 0 },
        ],
        membership: null,
      },
    ],
    missing: [{ firstHour: 110, lastHour: 119, reason: 'Raids were not recorded at these hours' }],
    warnings: [],
  };

  assert.equal(validateRaidTallyReply(reply), reply);
  assert.deepEqual(raidTallyProof(reply, [100, 130], 132), [130, 134]);
  assert.equal(raidTallyProof(reply, [100, 130], 135), null);
  assert.throws(() => validateRaidTallyReply({ ...reply, hour: 120 }));

  const hoverAt105: RaidTallyAtReply = {
    branchId: '1',
    hour: 105,
    firstHour: 100,
    lastHour: 109,
    metric: 'both',
    available: true,
    unavailable: null,
    missing: null,
    rows: [
      { country: 7, tag: 'AAA', launched: 1, succeeded: 0, missing: null },
      { country: 11, tag: '', launched: 0, succeeded: 0, missing: null },
    ],
    unattributed: 0,
    unattributedLaunched: 0,
    unattributedSucceeded: 0,
    warnings: [],
  };
  assert.equal(validateRaidTallyAt(hoverAt105), hoverAt105);

  const identity = fakeIdentity({
    7: { tag: 'AAA', name: 'Alpha Republic', colour: '#4488cc' },
  });
  const view = createBoundedReadView({
    hour: displayedHour(130),
    panels: new Map<string, unknown>([['tally', reply]]),
    hovers: new Map([['hover', { hour: 105, data: hoverAt105 }]]),
    identity,
  });

  const opts = {
    tags: [countryKey(7), countryKey(11), countryKey(12)],
    top: null,
    metric: 'both' as const,
    holders: null,
    membership: null,
    endLabels: true,
    settings: DEFAULT_LINE_SETTINGS,
    extent: [100, 140] as const,
    gaps: [],
    surface: '#161616',
  };

  assert.equal(raidTallyLinesProjection.nextChange?.(view, opts), 135);
  const model = raidTallyLinesProjection.project(view, opts);
  assert.equal(model.mode, 'lines');
  assert.equal(model.series.length, 6, 'both metric emits solid launched and dashed succeeded series per slot');
  assert.ok(model.legend?.some(l => l.label === 'Solid: launched'));
  assert.ok(model.legend?.some(l => l.label === 'Dashed: succeeded'));
  assert.ok(model.warnings?.some(w => w.includes('1 raids have no named raider')));

  const slot7Launched = model.series.find(s => s.id === `${countryKey(7)}\u0000launched`);
  assert.ok(slot7Launched?.points?.some(pt => pt.hour === 110 && pt.value === null), 'unread Raids span breaks the step line with null');
  assert.ok(model.gaps?.some(g => g.firstHour === 110 && g.lastHour === 119));

  const slot11Launched = model.series.find(s => s.id === `${countryKey(11)}\u0000launched`);
  const slot12Launched = model.series.find(s => s.id === `${countryKey(12)}\u0000launched`);
  assert.equal(slot11Launched?.endLabel?.name, unresolvedSlotLabel(11));
  assert.equal(slot12Launched?.endLabel?.name, unresolvedSlotLabel(12));
  assert.notEqual(slot11Launched?.endLabel?.name, slot12Launched?.endLabel?.name, 'two unresolved slots stay distinct');

  const layout = layoutTimeChart(model, 600, 260, 'sans-serif', t => t.length * 6);
  const tip = deriveTimeChartTooltip(model, layout, 'plot:105');
  assert.ok(
    tip?.payload?.items?.some(i => i.label === 'Alpha Republic' && i.value === '1 launched · 0 succeeded'),
    'hover at hour 105 returns hour 105 counts rather than playhead counts (3 launched · 2 succeeded)',
  );
  assert.ok(
    tip?.payload?.items?.some(i => i.label === unresolvedSlotLabel(11) && i.value === '0 launched · 0 succeeded'),
    'hover formats unresolved slot with distinct slot id and reason',
  );

  const unreadHover: RaidTallyAtReply = {
    branchId: '1',
    hour: 115,
    firstHour: 110,
    lastHour: 119,
    metric: 'both',
    available: true,
    unavailable: null,
    missing: 'Raids were not recorded at these hours',
    rows: [],
    unattributed: 0,
    unattributedLaunched: 0,
    unattributedSucceeded: 0,
    warnings: [],
  };
  const unreadView = createBoundedReadView({
    hour: displayedHour(130),
    panels: new Map<string, unknown>([['tally', reply]]),
    hovers: new Map([['hover', { hour: 115, data: unreadHover }]]),
    identity,
  });
  const unreadModel = raidTallyLinesProjection.project(unreadView, opts);
  assert.ok(
    unreadModel.hoverDetail?.items.some(i => i.missingReason === 'Raids were not recorded at these hours'),
    'hover over unread Raids hour reports missing rather than zeros',
  );
});

test('raid-tally lines reports unavailable when raid types are absent or head is unrecorded', () => {
  const unavailableReply: RaidTallyReply = {
    hour: 110,
    nextChange: null,
    available: false,
    unavailable: 'The content declares no raid type.',
    unattributed: 0,
    series: [],
    missing: [],
    warnings: [],
  };
  const view = createBoundedReadView({
    hour: displayedHour(110),
    panels: new Map<string, unknown>([['tally', unavailableReply]]),
    identity: fakeIdentity({}),
  });
  const model = raidTallyLinesProjection.project(view, {
    tags: [],
    top: 5,
    metric: 'launched',
    holders: null,
    membership: null,
    endLabels: true,
    settings: DEFAULT_LINE_SETTINGS,
    extent: [100, 120],
    gaps: [],
    surface: '#161616',
  });
  assert.equal(model.empty?.kind, 'unavailable');
  assert.equal(model.empty?.message, 'No raid types available.');
});

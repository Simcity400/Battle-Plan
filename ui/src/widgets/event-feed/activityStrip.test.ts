import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import { countryIdentityIndex } from '../../dashboard/countryIdentity.ts';
import type { BoundTrack, CountrySelection } from '../../dashboard/model';
import { PlaybackController } from '../../dashboard/playback.ts';
import type { EngineTask, FrameEngine } from '../../engine/frame.ts';
import type { EventFeedActivityReply } from '../../generated/dashboard';
import {
  activityCountryFilter,
  activityDisplayedHour,
  activityRequestPlan,
  activityScopeKey,
  activityStripStart,
  buildFullActivityStrip,
  canDisplayStrip,
  createActivityClockTracker,
  mergeActivityStrip,
  shouldFetchActivityStrip,
  stripBinCoverage,
  stripBinHours,
  subscribeActivityClock,
} from './activityStrip.ts';

const track = (
  axisOffset: number,
  selectedSlot: number | null = 7,
  gaps: { readonly firstHour: number; readonly lastHour: number; readonly reason?: string }[] = [],
): BoundTrack => ({
  id: 't1',
  roleId: 'r1',
  source: { id: 'src-1', revision: 'rev-1', path: '', metadata: { campaignKey: 'c1', playerCountryId: null, branches: [] } },
  branch: { id: '1', firstHour: 100, lastHour: 5000, gaps },
  axisOffset,
  selectedSlot,
} as unknown as BoundTrack);

function createFakeFrameEngine() {
  type Phase = Parameters<FrameEngine['every']>[0];
  const tasks = new Map<Phase, Set<{ run: (frame: { now: number }) => boolean; active: boolean }>>();
  const phaseSet = (phase: Phase) => {
    let set = tasks.get(phase);
    if (!set) {
      set = new Set();
      tasks.set(phase, set);
    }
    return set;
  };
  const engine = {
    every(phase: Phase, run: (frame: { now: number }) => boolean): EngineTask {
      const entry = { run, active: true };
      phaseSet(phase).add(entry);
      return {
        wake() { entry.active = true; },
        stop() {
          entry.active = false;
          phaseSet(phase).delete(entry);
        },
      } as EngineTask;
    },
  } as unknown as FrameEngine;
  const tick = (now: number) => {
    for (const phase of ['clock', 'paint'] as const) {
      for (const entry of [...phaseSet(phase)]) {
        if (!entry.active) continue;
        entry.active = entry.run({ now });
      }
    }
  };
  return { engine, tick };
}

test('activityDisplayedHour adds track.axisOffset, clamps to branch bounds, caps frozen pages, and honors live installedHour (#823 R2-S12)', () => {
  assert.equal(activityDisplayedHour(track(50), null, 200), 250, 'positive axisOffset is added, never subtracted');
  assert.equal(activityDisplayedHour(track(-50), null, 200), 150, 'negative axisOffset is added accurately');
  assert.equal(activityDisplayedHour(track(50), null, 40), null, 'hours before branch.firstHour return null');
  assert.equal(activityDisplayedHour(track(50), null, 4980), 5000, 'hours past branch.lastHour clamp to lastHour');
  assert.equal(activityDisplayedHour(track(50), 312.7, 980), 312, 'pinnedHour overrides playback head and floors');
  assert.equal(activityDisplayedHour(track(0), null, 250, 180), 180, 'a frozen table page caps the followed strip at the frozen snapshot hour');
  assert.equal(activityDisplayedHour(track(0), null, 150, 180), 150, 'seeking backward before the frozen page hour uses the live displayed hour');
  assert.equal(activityDisplayedHour(track(0), null, 100, null, 105, 105), 105, 'a mid-day table install advances the displayed hour inside the same day bin');
  assert.equal(activityDisplayedHour(track(0), null, 102, null, 105, 102), 102, 'a stale installedHour past the live clock after seek-back is ignored');
  assert.equal(activityStripStart(0, null, 2000), 552, 'rolling 60-day start aligns to 24h bin boundaries');
  assert.equal(activityStripStart(0, null, 2010), 552, 'and stays constant across hours within the same day');
});

test('activityCountryFilter resolves selected dashboard country and rejects empty selections (#823 R2-S12)', () => {
  assert.deepEqual(activityCountryFilter({ kind: 'all' }, track(0)), { kind: 'all' });
  assert.deepEqual(activityCountryFilter({ kind: 'selected' }, track(0, 12)), { kind: 'countries', countries: [12] });
  assert.deepEqual(activityCountryFilter({ kind: 'selected' }, track(0, null)), { kind: 'empty' });
  assert.deepEqual(activityCountryFilter({ kind: 'tags', tags: [] }, track(0)), { kind: 'empty' });
  assert.deepEqual(activityCountryFilter({ kind: 'tags', tags: ['GER', 'ITA'] }, track(0)), { kind: 'countries', countries: ['GER', 'ITA'] });
  assert.deepEqual(
    activityCountryFilter({ kind: 'group', group: { type: 'humans' }, membership: 'hour', aggregate: false }, track(0)),
    { kind: 'group', group: { type: 'humans', membership: 'hour' } },
  );
  assert.deepEqual(
    activityCountryFilter({ kind: 'group', group: { type: 'top', count: 5 }, membership: 'hour' } as CountrySelection, track(0)),
    { kind: 'empty' },
  );
  const key12 = activityScopeKey({ sourceId: 's', revision: '1', branchId: '1', binHours: 24, categories: ['politics'], countryFilter: activityCountryFilter({ kind: 'selected' }, track(0, 12)) });
  const key19 = activityScopeKey({ sourceId: 's', revision: '1', branchId: '1', binHours: 24, categories: ['politics'], countryFilter: activityCountryFilter({ kind: 'selected' }, track(0, 19)) });
  assert.notEqual(key12, key19, 'changing the selected country slot on the track changes the strip scope key');

  const identities = countryIdentityIndex([
    { tag: 'AAA', identities: [{ slot: 7, countryId: 101, firstHour: 0, lastHour: 199 }] },
    { tag: 'BBB', identities: [{ slot: 7, countryId: 101, firstHour: 200, lastHour: 400 }] },
  ]);
  assert.deepEqual(
    activityCountryFilter({ kind: 'tags', tags: ['AAA'] }, track(0), identities),
    { kind: 'countries', countries: [7, 'AAA'] },
    'saved tag selection resolves slot 7 alongside AAA so renamed BBB events match the feed',
  );
});

test('activityRequestPlan, mergeActivityStrip, and canDisplayStrip reuse historical bins on overlap and clear on disjoint seeks or null targets (#823 R2-S12)', () => {
  const scope = activityScopeKey({
    sourceId: 's',
    revision: '1',
    branchId: '1',
    binHours: 24,
    categories: ['politics', 'warsPeace'],
    countryFilter: { kind: 'all' },
  });
  const initialPlan = activityRequestPlan(null, scope, 0, 70, 24);
  assert.equal(initialPlan.incremental, false);
  assert.equal(initialPlan.requestFrom, 0);

  const initialReply: EventFeedActivityReply = {
    branchId: '1',
    firstHour: 0,
    binHours: 24,
    lines: [
      { category: 'politics', before: '0', bins: [[0, 3], [1, 5], [2, 2]] },
      { category: 'warsPeace', before: '0', bins: [[1, 1], [2, 4]] },
    ],
    missing: [{ firstHour: 12, lastHour: 60, reason: 'Unread stretch' }],
    warnings: [],
  };
  const settled = buildFullActivityStrip(initialReply, initialPlan);
  assert.equal(settled.binCount, 3);
  assert.deepEqual(settled.series[0]!.counts, [3, 5, 2]);

  const forwardPlan = activityRequestPlan(settled, scope, 0, 110, 24);
  assert.equal(forwardPlan.incremental, true);
  assert.equal(forwardPlan.invalidatePrev, false);
  assert.equal(forwardPlan.requestFrom, 48, 'incremental fetch starts at the last bin start hour so bins 0..1 are reused');
  assert.equal(
    canDisplayStrip(settled, { queryKey: forwardPlan.queryKey, scopeKey: scope, binHours: 24, bound: { firstHour: 0, lastHour: 110 } }),
    true,
    'overlapping forward advance keeps bars painted while incremental tail loads',
  );

  const tailReply: EventFeedActivityReply = {
    branchId: '1',
    firstHour: 48,
    binHours: 24,
    lines: [
      { category: 'politics', before: '8', bins: [[0, 7], [1, 1], [2, 9]] },
      { category: 'warsPeace', before: '1', bins: [[0, 4], [2, 2]] },
    ],
    missing: [
      { firstHour: 48, lastHour: 65, reason: 'Unread stretch' },
      { firstHour: 100, lastHour: 105, reason: 'Second gap' },
    ],
    warnings: [],
  };
  const merged = mergeActivityStrip(settled, tailReply, forwardPlan)!;
  assert.ok(merged !== null);
  assert.equal(merged.firstHour, 0);
  assert.equal(merged.lastHour, 110);
  assert.equal(merged.binCount, 5);
  assert.deepEqual(merged.series[0]!.counts, [3, 5, 7, 1, 9], 'historical bins 0..1 are kept while active tail and new bins update');
  assert.deepEqual(merged.series[1]!.counts, [0, 1, 4, 0, 2]);
  assert.deepEqual(merged.missing, [
    { firstHour: 12, lastHour: 65, reason: 'Unread stretch' },
    { firstHour: 100, lastHour: 105, reason: 'Second gap' },
  ], 'adjacent missing spans across the tail boundary merge cleanly');

  const backdatedReply: EventFeedActivityReply = {
    branchId: '1',
    firstHour: 48,
    binHours: 24,
    lines: [
      { category: 'politics', before: '9', bins: [[0, 7], [1, 1], [2, 9]] },
      { category: 'warsPeace', before: '1', bins: [[0, 4], [2, 2]] },
    ],
    missing: [],
    warnings: [],
  };
  assert.equal(
    mergeActivityStrip(settled, backdatedReply, forwardPlan),
    null,
    'when a backdated event increases line.before past the cached prefix sum, merge rejects so the caller refetches the full span',
  );

  const disjointForward = activityRequestPlan(merged, scope, 2400, 2447, 24);
  assert.equal(disjointForward.incremental, false);
  assert.equal(disjointForward.invalidatePrev, true, 'forward seek to a disjoint window invalidates stale bars immediately');
  assert.equal(
    canDisplayStrip(merged, { queryKey: disjointForward.queryKey, scopeKey: scope, binHours: 24, bound: { firstHour: 2400, lastHour: 2447 } }),
    false,
    'disjoint forward seek never leaves old bars painted while waiting for the new reply',
  );
  assert.equal(canDisplayStrip(merged, null), false, 'null target hides stale bars immediately on render');

  const seekBack = activityRequestPlan(merged, scope, 0, 50, 24);
  assert.equal(seekBack.incremental, false);
  assert.equal(seekBack.invalidatePrev, true, 'backward seek invalidates stale future bars immediately');
  assert.equal(
    canDisplayStrip(merged, { queryKey: seekBack.queryKey, scopeKey: scope, binHours: 24, bound: { firstHour: 0, lastHour: 50 } }),
    false,
  );

  const otherScope = activityScopeKey({
    sourceId: 's',
    revision: '1',
    branchId: '1',
    binHours: 24,
    categories: ['politics'],
    countryFilter: { kind: 'all' },
  });
  const scopeChanged = activityRequestPlan(merged, otherScope, 0, 120, 24);
  assert.equal(scopeChanged.incremental, false);
  assert.equal(scopeChanged.invalidatePrev, true, 'changing filters invalidates stale bars immediately');
  assert.equal(
    shouldFetchActivityStrip(merged, { queryKey: merged.key, scopeKey: scope, binHours: 24, bound: { firstHour: 0, lastHour: 110 } }),
    false,
    'a retained strip matching the target queryKey does not re-fetch',
  );
  assert.equal(
    shouldFetchActivityStrip(null, { queryKey: merged.key, scopeKey: scope, binHours: 24, bound: { firstHour: 0, lastHour: 110 } }),
    true,
    'switching A -> B (clearing retained strip) -> A before B replies still fetches A',
  );
  assert.equal(
    canDisplayStrip(
      settled,
      { queryKey: forwardPlan.queryKey, scopeKey: scope, binHours: 24, bound: { firstHour: 0, lastHour: 110 } },
      forwardPlan.queryKey,
    ),
    false,
    'a failed overlapping forward request hides previous bars instead of leaving stale data painted',
  );

  const sliding = activityRequestPlan(merged, scope, 24, 134, 24);
  assert.equal(sliding.incremental, true, 'aligned sliding window reuses surviving middle bins');
  assert.equal(sliding.invalidatePrev, false);
  assert.equal(sliding.requestFrom, 96, 'tail fetch starts at the previous final bin');
  const slideReply: EventFeedActivityReply = {
    branchId: '1',
    firstHour: 96,
    binHours: 24,
    lines: [
      { category: 'politics', before: '16', bins: [[0, 10], [1, 4]] },
      { category: 'warsPeace', before: '5', bins: [[0, 2], [1, 3]] },
    ],
    missing: [{ firstHour: 100, lastHour: 105, reason: 'Second gap' }],
    warnings: [],
  };
  const slid = mergeActivityStrip(merged, slideReply, sliding)!;
  assert.ok(slid !== null);
  assert.equal(slid.firstHour, 24);
  assert.equal(slid.boundFirstHour, 24);
  assert.deepEqual(slid.series[0]!.counts, [5, 7, 1, 10, 4], 'dropped bin 0 shifts out, middle bins 1..3 survive, tail updates');
  assert.equal(slid.beforeByCategory.politics, 3n, 'beforeByCategory advances by dropped bin 0 count');
  assert.deepEqual(slid.missing, [
    { firstHour: 24, lastHour: 65, reason: 'Unread stretch' },
    { firstHour: 100, lastHour: 105, reason: 'Second gap' },
  ], 'sliding prefix clips earlier missing spans to the new window start');
});

test('stripBinCoverage clips missing and group gaps to boundFirstHour..lastHour and distinguishes full vs partial bins (#823 R2-S12)', () => {
  assert.equal(stripBinHours(72 * 24), 24);
  assert.equal(stripBinHours(120 * 24), 48);

  const preBound = stripBinCoverage(96, 110, 143, 24, 2, [], [[100, 102]]);
  assert.deepEqual([...preBound.gapBins], [], 'group gaps before boundFirstHour do not mark bin 0 as a gap');
  assert.deepEqual([...preBound.partBins], [], 'group gaps before boundFirstHour do not mark bin 0 as partial');

  const mixed = stripBinCoverage(
    96,
    110,
    191,
    24,
    4,
    [
      { firstHour: 10, lastHour: 109, reason: 'Before boundFirstHour' },
      { firstHour: 120, lastHour: 143, reason: 'Full bin 1' },
      { firstHour: 150, lastHour: 155, reason: 'Partial bin 2' },
      { firstHour: 192, lastHour: 250, reason: 'After lastHour' },
    ],
    [[110, 119], [153, 160], [172, 180]],
  );
  assert.deepEqual([...mixed.gapBins], [0, 1], 'bin 0 covered across its bounded 110..119 hours and full bin 1 are full gaps');
  assert.deepEqual([...mixed.partBins], [2, 3], 'overlapping missing + group gap in bin 2 and partial group gap in bin 3 are partial');
});

test('subscribeActivityClock advances across ordinary forward playback without per-hour renders or requests when nothing in the strip changes (#823 R2-S12)', () => {
  const fake = createFakeFrameEngine();
  const clock = new PlaybackController(fake.engine);
  clock.setBounds({ firstHour: 0, lastHour: 5000 }, true);
  clock.seek(100);
  clock.setSpeed(1);

  const t = track(0);
  const tracker = createActivityClockTracker(t, null, null, undefined);
  let renders = 0;
  let requests = 0;
  let lastQueryKey = '';
  const scope = activityScopeKey({
    sourceId: 'src-1',
    revision: 'rev-1',
    branchId: '1',
    binHours: 24,
    categories: ['politics'],
    countryFilter: { kind: 'all' },
  });
  const render = (installedHour: number | null = null) => {
    renders++;
    const shown = tracker.read(clock.getHour());
    const displayed = activityDisplayedHour(t, null, shown, null, installedHour, clock.getHour());
    if (displayed === null) return null;
    const start = activityStripStart(t.branch.firstHour, null, displayed);
    const key = JSON.stringify([scope, start, displayed]);
    if (key !== lastQueryKey) {
      lastQueryKey = key;
      requests++;
    }
    return displayed;
  };

  assert.equal(render(), 100);
  assert.equal(renders, 1);
  assert.equal(requests, 1);

  const stop = subscribeActivityClock(clock, tracker, true, () => { render(); });

  clock.setPlaying(true);
  let now = clock.composedAt() + 1;
  assert.equal(renders, 1, 'pressing play without advancing the hour causes zero extra renders');
  assert.equal(requests, 1, 'pressing play without advancing the hour causes zero extra requests');

  for (let h = 101; h <= 119; h++) {
    now += 1000;
    fake.tick(now);
    assert.equal(clock.getHour(), h);
  }
  assert.equal(renders, 1, '19 quiet played hours inside the same day bin add zero renders');
  assert.equal(requests, 1, '19 quiet played hours inside the same day bin add zero requests');

  for (let h = 120; h <= 124; h++) {
    now += 1000;
    fake.tick(now);
    assert.equal(clock.getHour(), h);
  }
  assert.equal(renders, 2, 'crossing from hour 100 to 124 wakes once at the day boundary (hour 120), never skipped and never per hour');
  assert.equal(requests, 2, 'crossing from hour 100 to 124 issues exactly one incremental request');
  assert.equal(tracker.read(clock.getHour()), 120);

  assert.equal(render(122), 122, 'a mid-day table install advances the strip to 122 immediately');
  assert.equal(requests, 3);

  stop();
  clock.stop();

  const frozenTracker = createActivityClockTracker(t, null, 130, undefined);
  clock.seek(130);
  let frozenWakes = 0;
  const stopFrozen = subscribeActivityClock(clock, frozenTracker, true, () => { frozenWakes++; });
  clock.setPlaying(true);
  now = clock.composedAt() + 1;
  for (let h = 131; h <= 180; h++) {
    now += 1000;
    fake.tick(now);
  }
  assert.equal(frozenWakes, 0, 'a frozen table page stays at zero wakes across 50 played hours past frozenHour');
  stopFrozen();
  clock.stop();

  let pinnedWakes = 0;
  const stopPinned = subscribeActivityClock(clock, tracker, false, () => { pinnedWakes++; });
  clock.seek(200);
  assert.equal(pinnedWakes, 0, 'inactive or pinned mode registers no clock subscription');
  stopPinned();

  const css = readFileSync(new URL('./eventFeedActivityStrip.css', import.meta.url), 'utf8');
  const gapRule = /\.efs-gap\s*\{([^}]*)\}/.exec(css)?.[1] ?? '';
  assert.match(gapRule, /flex:\s*1\s+1\s+0/, '.efs-gap takes an equal share of the flex axis like .efs-bar');
  assert.doesNotMatch(gapRule, /flex:\s*0\s+0\s+3px/, '.efs-gap must not compress gaps to a fixed 3px width');
});

import assert from 'node:assert/strict';
import test from 'node:test';
import { createServer } from 'vite';
import { raceRows } from './decryptionData.ts';
import type { DecryptionReply } from '../../generated/dashboard';
import type { DashboardClient, ProductReply, ProductRequest } from '../../dashboard/client';
import type { BoundTrack } from '../../dashboard/model';

type Effect = { deps: readonly unknown[]; run: () => void | (() => void); cleanup?: () => void };
const settle = () => new Promise<void>(resolve => setImmediate(resolve));

test('actual cipher label reads finish during fast playback and retain only their authenticated supplying segment', async () => {
  const slots = new Map<number, unknown>(), effects = new Map<number, Effect>(), pending = new Map<number, Effect>();
  const timers = new Map<number, { at: number; run: () => void }>();
  let cursor = 0, nextTimer = 0;
  const same = (a: readonly unknown[], b: readonly unknown[]) => a.length === b.length && a.every((value, i) => Object.is(value, b[i]));
  const fixture = {
    useSyncExternalStore: (_subscribe: unknown, snapshot: () => number) => snapshot(),
    now: 0,
    useState(initial?: unknown) {
      const index = cursor++;
      if (!slots.has(index)) slots.set(index, initial);
      return [slots.get(index), (next: unknown) => slots.set(index, typeof next === 'function' ? next(slots.get(index)) : next)];
    },
    useRef(initial: unknown) {
      const index = cursor++;
      if (!slots.has(index)) slots.set(index, { current: initial });
      return slots.get(index);
    },
    useMemo(make: () => unknown, deps: readonly unknown[]) {
      const index = cursor++, old = slots.get(index) as { deps: readonly unknown[]; value: unknown } | undefined;
      if (!old || !same(old.deps, deps)) slots.set(index, { deps: [...deps], value: make() });
      return (slots.get(index) as { value: unknown }).value;
    },
    useCallback(callback: unknown, deps: readonly unknown[]) { return fixture.useMemo(() => callback, deps); },
    useEffect(run: Effect['run'], deps: readonly unknown[]) {
      const index = cursor++, old = effects.get(index);
      if (!old || !same(old.deps, deps)) pending.set(index, { run, deps: [...deps] });
    },
    after(run: () => void, ms: number) { const id = ++nextTimer; timers.set(id, { at: fixture.now + ms, run }); return id; },
    clear(id: number) { timers.delete(id); },
    advance(now: number) {
      fixture.now = now;
      for (const [id, timer] of [...timers]) if (timer.at <= now) { timers.delete(id); timer.run(); }
    },
    commit() {
      for (const [index, effect] of pending) {
        effects.get(index)?.cleanup?.();
        const cleanup = effect.run();
        effect.cleanup = typeof cleanup === 'function' ? cleanup : undefined;
        effects.set(index, effect);
      }
      pending.clear();
    },
  };
  const host = globalThis as typeof globalThis & { __cipherCountryHooks?: typeof fixture };
  host.__cipherCountryHooks = fixture;
  // Only React's dispatcher, the throttle's clock and pixel composition are
  // synthetic. The consumer, rich label hooks, stores and transport are real.
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'cipher-country-request-fixture', enforce: 'pre',
      transform(source, id) {
        const path = id.replaceAll('\\', '/');
        if (['decryptionCountries', 'countryNamesAt', 'countryFlagsAt', 'useThrottled'].some(name => path.endsWith(`/${name}.ts`))) {
          let code = source.replace(/import \{ ([^}]+) \} from 'react';/, 'const { $1 } = globalThis.__cipherCountryHooks;');
          if (path.endsWith('/useThrottled.ts')) code = code.replaceAll('Date.now()', 'globalThis.__cipherCountryHooks.now')
            .replace('setTimeout(show, wait)', 'globalThis.__cipherCountryHooks.after(show, wait)')
            .replace('clearTimeout(timer)', 'globalThis.__cipherCountryHooks.clear(timer)');
          return code;
        }
        if (path.endsWith('/contentRevision.ts')) return source.replace(
          'return useSyncExternalStore(revision.subscribe, revision.snapshot, revision.snapshot);', 'return revision.snapshot();');
      },
    }] });
  try {
    const { useDecryptionCountries } = await vite.ssrLoadModule('/ui/src/widgets/decryption/decryptionCountries.ts') as typeof import('./decryptionCountries.ts');
    const { contentRevision } = await vite.ssrLoadModule('/ui/src/dashboard/contentRevision.ts') as typeof import('../../dashboard/contentRevision.ts');
    const data: DecryptionReply = { branchId: '1', warnings: [], coverage: [{ firstHour: 100, lastHour: 150, complete: true, reason: null }],
      pairs: [{ ownerId: '11', targetId: '22', tagRuns: [{ firstHour: 100, lastHour: 129, ownerTag: 'Owner', targetTag: 'Target' },
        { firstHour: 130, lastHour: 150, ownerTag: 'Renamed', targetTag: 'Target' }], breaks: [],
      runs: [{ firstHour: 100, lastHour: 150, startAmountRaw: '400000', stepRawPerDay: '0', thresholdRaw: '800000', thresholdUnknown: null,
        bonusDays: 0, bonusDaysStepPerDay: 0, stateFlags: 1, broken: false, started: { hour: 100, upperBound: false } }] }] };
    const track = { source: { id: 'source', revision: 'revision' }, branch: { id: '1', firstHour: 100, lastHour: 150, gaps: [] } } as unknown as Pick<BoundTrack, 'source' | 'branch'>;
    const transport = () => {
      const calls: { input: ProductRequest<{ hour: number; tags: string[] }>; signal: AbortSignal; answer: (data: unknown) => void }[] = [];
      const client = { product: (input: typeof calls[number]['input'], signal: AbortSignal) => new Promise<ProductReply<unknown>>(resolve => {
        calls.push({ input, signal, answer: data => resolve({ ...input, data }) });
      }) } as unknown as DashboardClient;
      return { client, calls };
    };
    const first = transport();
    let client = first.client, active = true, currentTrack = track;
    const render = (hour: number, visible = true, reply = data) => {
      const rows = raceRows(reply, hour, 'progress');
      cursor = 0;
      const labels = useDecryptionCountries(client, currentTrack, rows,
        visible ? rows.map(item => ({ key: item.key, item, slot: 0, phase: 'staying' as const, since: 0 })) : [], active);
      fixture.commit();
      return { labels, row: rows[0] };
    };
    let view = render(100); await settle();
    assert.equal(first.calls.length, 2);
    assert.equal(view.labels.pair(view.row).owner.flagPending, true);
    for (let tick = 1; tick <= 24; tick++) {
      fixture.advance(tick * 1000 / 24);
      view = render(100 + tick); await settle();
    }
    assert.equal(first.calls.length, 2, 'responses slower than both one played hour and 400 ms are not replaced');
    assert.ok(first.calls.every(call => !call.signal.aborted && call.input.params.hour === 100));
    first.calls.find(call => call.input.product === 'country-styles')!.answer({ styles: [
      { tag: 'Owner', name: 'Owner at 100', color: null }, { tag: 'Target', name: 'Target at 100', color: null },
    ] });
    await settle(); view = render(124);
    assert.equal(first.calls.length, 2, 'the unfinished flag read stays wanted after names arrive');
    first.calls.find(call => call.input.product === 'country-flags')!.answer({ flags: [
      { tag: 'Owner', key: 'synthetic#7', width: 1, height: 1, png: 'data:synthetic-7' },
    ], unavailable: [{ tag: 'Target', reason: 'Native target sprite unavailable at 100' }] });
    await settle(); view = render(124);
    const shown = view.labels.pair(view.row);
    assert.deepEqual([shown.hour, shown.owner.name, shown.owner.flag, shown.owner.flagPending, shown.target.flagReason],
      [100, 'Owner at 100', 'data:synthetic-7', false, 'Native target sprite unavailable at 100']);
    fixture.advance(1100); view = render(125); await settle();
    assert.equal(view.labels.pair(view.row).hour, 100, 'a pending newer cohort keeps the completed pair and its statuses together');
    assert.equal(view.labels.pair(view.row).target.flagReason, 'Native target sprite unavailable at 100');
    const unfinished = first.calls.filter(call => call.input.params.hour !== 100);
    assert.equal(unfinished.length, 2);
    view = render(110); await settle();
    assert.ok(unfinished.every(call => call.signal.aborted), 'a pending sample later than a backward seek is released');
    assert.equal(view.labels.pair(view.row).hour, 100, 'an earlier completed look stays valid in the same segment');
    for (const call of unfinished) call.answer(call.input.product === 'country-styles'
      ? { styles: [{ tag: 'Owner', name: 'Future owner', color: null }] }
      : { flags: [{ tag: 'Owner', key: 'synthetic#9', width: 1, height: 1, png: 'data:synthetic-9' }], unavailable: [] });
    await settle(); view = render(110);
    assert.equal(view.labels.pair(view.row).owner.name, 'Owner at 100');
    assert.equal(view.labels.pair(view.row).owner.flag, 'data:synthetic-7');
    view = render(99); await settle();
    assert.ok(unfinished.every(call => call.signal.aborted), 'a backward move before the read segment releases its requests');
    view = render(130); await settle();
    assert.equal(view.labels.pair(view.row).owner.name, 'Loading name…');
    assert.equal(view.labels.pair(view.row).owner.flag, undefined, 'a native tag change cannot borrow the old pair');
    assert.equal(view.labels.pair(view.row).owner.flagPending, true, 'a valid new actor awaits its deferred sample, rather than claiming unavailable');
    fixture.advance(2000); render(130); render(130); await settle();
    const changed = first.calls.at(-1)!;
    assert.deepEqual(changed.input.params.tags, ['Renamed', 'Target']);
    active = false; view = render(130); await settle();
    assert.equal(changed.signal.aborted, true);
    assert.equal(view.labels.pair(view.row).target.flagReason, undefined);
    // Off screen nothing is awaited: an absent pending flag shows no pending indicator, as false does.
    assert.ok(!view.labels.pair(view.row).target.flagPending);
    active = true; fixture.advance(3000); render(130); render(130); await settle();
    const priorClient = first.calls.at(-1)!;
    const other = transport(); client = other.client; view = render(130); await settle();
    assert.equal(priorClient.signal.aborted, true);
    assert.equal(view.labels.pair(view.row).owner.flag, undefined);
    const priorContent = other.calls.at(-1)!;
    contentRevision(client).observe({ game: 'Synthetic game', user: 'Synthetic user', contentReady: true });
    view = render(130); await settle();
    assert.equal(priorContent.signal.aborted, true);
    const priorRevision = other.calls.at(-1)!;
    currentTrack = { ...track, source: { ...track.source, revision: 'new revision' } };
    view = render(130); await settle();
    assert.equal(priorRevision.signal.aborted, true);
    const hidden = other.calls.at(-1)!;
    render(130, false); await settle();
    assert.equal(hidden.signal.aborted, true, 'no hidden rows retain request demand');
    fixture.advance(4000); render(130); fixture.advance(4500); render(130); render(130); await settle();
    const beforeGap = other.calls.at(-1)!;
    assert.equal(beforeGap.signal.aborted, false);
    currentTrack = { ...currentTrack, branch: { ...currentTrack.branch, gaps: [{ firstHour: 130, lastHour: 130, reason: 'Synthetic recording gap' }] } };
    view = render(130); await settle();
    assert.equal(beforeGap.signal.aborted, true, 'branch gaps cannot authenticate an old native tag segment');
    assert.equal(view.labels.pair(view.row).owner.flag, undefined);
    assert.equal(view.labels.pair(view.row).owner.flagPending, false);
    currentTrack = { ...currentTrack, branch: { ...currentTrack.branch, gaps: [] } };
    render(130); await settle();
    const beforeMissingRead = other.calls.at(-1)!;
    assert.equal(beforeMissingRead.signal.aborted, false);
    const gap = { ...data, coverage: [], pairs: data.pairs.map(pair => ({ ...pair, runs: [], tagRuns: [] })) };
    render(130, true, gap); await settle();
    assert.equal(beforeMissingRead.signal.aborted, true, 'a missing native read releases sampled demand immediately');
    const unnamed = { ...data, pairs: data.pairs.map(pair => ({ ...pair, tagRuns: [
      { firstHour: 100, lastHour: 150, ownerTag: null, targetTag: null },
    ] })) };
    active = false;
    view = render(130, true, unnamed);
    assert.equal(view.labels.pair(view.row).owner.name, 'Country name unavailable');
    assert.equal(view.labels.pair(view.row).target.name, 'Country name unavailable');
  } finally {
    for (const effect of effects.values()) effect.cleanup?.();
    await vite.close(); delete host.__cipherCountryHooks;
  }
});

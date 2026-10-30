import assert from 'node:assert/strict';
import test from 'node:test';
import { createServer } from 'vite';
import { raceRows } from './decryptionData.ts';
import type { CountryFactsReply, CountryFlagChoice, CountryFlagParams, DecryptionReply } from '../../generated/dashboard';
import type { DashboardClient, ProductReply, ProductRequest } from '../../dashboard/client';
import type { BoundTrack } from '../../dashboard/model';

type Effect = { deps: readonly unknown[]; run: () => void | (() => void); cleanup?: () => void };
const settle = () => new Promise<void>(resolve => setImmediate(resolve));

test('actual cipher labels read the held timeline, wait only for new flags to be drawn and retain only their authenticated supplying segment', async () => {
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
  // synthetic. The consumer, rich label hooks, timeline, drawings and transport are real.
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'cipher-country-request-fixture', enforce: 'pre',
      transform(source, id) {
        const path = id.replaceAll('\\', '/');
        if (['decryptionCountries', 'countryNamesAt', 'countryFlagsAt', 'countryFlags', 'useThrottled'].some(name => path.endsWith(`/${name}.ts`))) {
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
    const choice = (key: string): CountryFlagChoice => ({ keys: [key, key, key], reasons: [null, null, null] });
    // Owner and Target from the start; Owner's holder renamed at 130, under its own flag.
    const facts: CountryFactsReply = { branchId: '1', known: [[100, 150]], names: ['Owner at 100', 'Target at 100', 'Renamed at 130'],
      reasons: ['not a country at this hour'], flags: [choice('OWNER'), choice('TARGET'), choice('RENAMED')], warnings: [],
      countries: [{ tag: 'Owner', runs: [[100, 0, 0x112233, -1, 0], [130, -1, -1, 0, -1]] }, { tag: 'Target', runs: [[100, 1, 0x445566, -1, 1]] },
        { tag: 'Renamed', runs: [[100, -1, -1, 0, -1], [130, 2, 0x778899, -1, 2]] }] };
    /** A client holding `facts` as its timeline, whose flag drawings wait to be answered. */
    const transport = () => {
      const draws: { flags: CountryFlagChoice[]; signal: AbortSignal; answer: (data: unknown) => void }[] = [];
      // Until the timeline is held, names are read hour by hour; those reads are left waiting.
      const styles: AbortSignal[] = [];
      const asked: string[] = [];
      const client = {
        preparedQuery: async (_source: unknown, product: string) => {
          asked.push(product);
          return { ...facts, coverage: { firstHour: 100, lastHour: 150, complete: true }, pending: [] };
        },
        product: (input: ProductRequest<CountryFlagParams>, signal: AbortSignal) => new Promise<ProductReply<unknown>>(resolve => {
          asked.push(input.product);
          if (input.product === 'country-styles') styles.push(signal);
          else draws.push({ flags: input.params.flags, signal, answer: data => resolve({ ...input, data }) });
        }),
      } as unknown as DashboardClient;
      return { client, draws, styles, asked };
    };
    const keys = (draw: { flags: CountryFlagChoice[] }) => draw.flags.map(flag => flag.keys[0]);
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
    view = render(100); await settle();
    assert.equal(first.asked.filter(product => product === 'country-facts').length, 1, 'the timeline is read once');
    const hourReads = first.styles.length;
    assert.deepEqual(first.draws.map(keys), [['OWNER', 'TARGET']], 'the held flags are drawn once, together');
    assert.equal(view.labels.pair(view.row).owner.name, 'Owner at 100');
    assert.equal(view.labels.pair(view.row).owner.flagPending, true, 'a pair waits for its flags to be drawn');
    for (let tick = 1; tick <= 24; tick++) {
      fixture.advance(tick * 1000 / 24);
      view = render(100 + tick); await settle();
    }
    assert.equal(first.draws.length, 1, 'a slow drawing is not asked for again across playback');
    assert.equal(first.styles.length, hourReads, 'names the timeline holds are never read hour by hour');
    assert.equal(view.labels.pair(view.row).hour, 100, 'the unfinished pair keeps its sample');
    first.draws[0].answer({ size: 'panel', warnings: [], flags: [{ flag: 0, asset: 'gfx/flags/OWNER.tga', key: 'synthetic#7', width: 1, height: 1, png: 'data:synthetic-7' }],
      unavailable: [{ flag: 1, reason: 'Native target sprite unavailable' }] });
    await settle(); view = render(124);
    const shown = view.labels.pair(view.row);
    assert.deepEqual([shown.hour, shown.owner.name, shown.owner.flag, shown.owner.flagPending, shown.target.flagReason],
      [100, 'Owner at 100', 'data:synthetic-7', false, 'Native target sprite unavailable']);
    fixture.advance(1100); view = render(125); render(125); await settle(); view = render(125);
    assert.ok((view.labels.pair(view.row).hour ?? 0) > 100, 'a later sample whose flags are drawn completes at once');
    assert.equal(view.labels.pair(view.row).owner.flag, 'data:synthetic-7');
    assert.equal(first.draws.length, 1, 'a flag already drawn is never drawn again');
    view = render(130); await settle();
    assert.equal(view.labels.pair(view.row).owner.flag, undefined, 'a native tag change cannot borrow the old pair');
    assert.equal(view.labels.pair(view.row).owner.flagPending, true, 'a valid new actor awaits its deferred sample, rather than claiming unavailable');
    fixture.advance(2000); render(130); render(130); await settle(); view = render(130);
    assert.deepEqual(keys(first.draws.at(-1)!), ['RENAMED'], 'only the new flag is drawn');
    assert.equal(view.labels.pair(view.row).owner.name, 'Renamed at 130');
    active = false; view = render(130); await settle();
    assert.equal(view.labels.pair(view.row).target.flagReason, undefined);
    // Off screen nothing is awaited: an absent pending flag shows no pending indicator, as false does.
    assert.ok(!view.labels.pair(view.row).target.flagPending);
    active = true; fixture.advance(3000); render(130); render(130); await settle();
    const other = transport(); client = other.client; view = render(130); await settle(); view = render(130);
    assert.equal(view.labels.pair(view.row).owner.flag, undefined, 'another client draws its own flags');
    assert.equal(view.labels.pair(view.row).target.flag, undefined);
    for (const draw of other.draws.splice(0)) draw.answer({ size: 'panel', warnings: [], unavailable: [],
      flags: draw.flags.map((_, flag) => ({ flag, asset: 'gfx/flags/a.tga', key: 'synthetic#9', width: 1, height: 1, png: 'data:synthetic-9' })) });
    await settle(); render(130); await settle(); view = render(130);
    assert.equal(view.labels.pair(view.row).target.flag, 'data:synthetic-9');
    contentRevision(client).observe({ game: 'Synthetic game', user: 'Synthetic user', contentReady: true });
    view = render(130);
    assert.equal(view.labels.pair(view.row).target.flag, undefined, 'another content revision draws its flags anew');
    currentTrack = { ...track, source: { ...track.source, revision: 'new revision' } };
    view = render(130);
    assert.equal(view.labels.pair(view.row).target.flag, undefined, 'another recording revision holds no timeline yet');
    currentTrack = { ...currentTrack, branch: { ...currentTrack.branch, gaps: [{ firstHour: 130, lastHour: 130, reason: 'Synthetic recording gap' }] } };
    view = render(130); await settle();
    assert.equal(view.labels.pair(view.row).owner.flag, undefined, 'branch gaps cannot authenticate an old native tag segment');
    assert.equal(view.labels.pair(view.row).owner.flagPending, false);
    currentTrack = { ...currentTrack, branch: { ...currentTrack.branch, gaps: [] } };
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

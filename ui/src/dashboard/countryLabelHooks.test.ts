import assert from 'node:assert/strict';
import test from 'node:test';
import { createServer } from 'vite';
import type { DashboardClient, ProductReply, ProductRequest } from './client';
import type { BoundTrack } from './model';

type Effect = { deps: readonly unknown[]; run: () => void | (() => void); cleanup?: () => void };
const settle = () => new Promise<void>(resolve => setImmediate(resolve));

test('actual dated lane and hover hooks retain flag metadata and empty names without widening requests', async () => {
  const slots = new Map<number, unknown>(), effects = new Map<number, Effect>(), pending = new Map<number, Effect>();
  const timers = new Map<number, () => void>();
  let nextTimer = 0;
  let cursor = 0, now = Date.now();
  const same = (a: readonly unknown[], b: readonly unknown[]) => a.length === b.length && a.every((value, i) => Object.is(value, b[i]));
  const fixture = {
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
    after(run: () => void) { const id = nextTimer++; timers.set(id, run); return id; },
    cancel(id: number) { timers.delete(id); },
    useEffect(run: Effect['run'], deps: readonly unknown[]) {
      const index = cursor++, old = effects.get(index);
      if (!old || !same(old.deps, deps)) pending.set(index, { run, deps: [...deps] });
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
    unmount() {
      for (const effect of effects.values()) effect.cleanup?.();
      slots.clear(); effects.clear(); pending.clear(); timers.clear();
    },
  };
  const host = globalThis as typeof globalThis & { __countryLabelHooks?: typeof fixture };
  host.__countryLabelHooks = fixture;
  // Only the hook dispatcher and throttle clock are synthetic. Fact stores,
  // request planning, cancellation, native envelopes and flag extraction are real.
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'country-label-hooks-fixture', enforce: 'pre',
      transform(source, id) {
        const path = id.replaceAll('\\', '/');
        if (['useCountriesAt', 'useHoverLabels', 'countryNamesAt', 'countryFlagsAt', 'useThrottled'].some(name => path.endsWith(`/dashboard/${name}.ts`)))
          source = source.replace(/import \{ ([^}]+) \} from 'react';/, 'const { $1 } = globalThis.__countryLabelHooks;');
        if (path.endsWith('/dashboard/useThrottled.ts')) source = source.replace('setTimeout(show, wait)', 'globalThis.__countryLabelHooks.after(show, wait)')
          .replace('clearTimeout(timer)', 'globalThis.__countryLabelHooks.cancel(timer)');
        if (path.endsWith('/dashboard/contentRevision.ts')) return source.replace(
          'return useSyncExternalStore(revision.subscribe, revision.snapshot, revision.snapshot);', 'return revision.snapshot();');
        return source;
      },
    }] });
  const dateNow = Date.now;
  Date.now = () => now;
  try {
    const lanes = await vite.ssrLoadModule('/ui/src/dashboard/useCountriesAt.ts') as typeof import('./useCountriesAt.ts');
    const hovers = await vite.ssrLoadModule('/ui/src/dashboard/useHoverLabels.ts') as typeof import('./useHoverLabels.ts');
    const transport = () => {
      const calls: { input: ProductRequest; signal: AbortSignal; answer: (data: unknown) => void; fail: () => void }[] = [];
      const client = { product: (input: ProductRequest, signal: AbortSignal) => new Promise<ProductReply<unknown>>((resolve, reject) => {
        calls.push({ input, signal, answer: data => resolve({ ...input, data }), fail: () => reject(new Error('Flag content refused')) });
      }) } as unknown as DashboardClient;
      return { client, calls };
    };
    const track = { source: { id: 'source', revision: 'revision' }, branch: { id: 'branch', firstHour: 0, lastHour: 100,
      gaps: [{ firstHour: 50, lastHour: 50 }] } } as unknown as Pick<BoundTrack, 'source' | 'branch'>;
    const tags = Array.from({ length: 5000 }, (_, i) => `C${String(i).padStart(5, '0')}`), visible = tags.slice(0, 4);
    const image = (tag: string) => ({ tag, key: 'synthetic#7', width: 1, height: 1, png: 'data:synthetic-7' });
    const flagCalls = (calls: ReturnType<typeof transport>['calls']) => calls.filter(call => call.input.product === 'country-flags');
    const a = transport();
    const wording = { pending: 'Loading\u2026', missing: 'Not recorded' };
    const renderLane = (hour: number, shown = visible, client = a.client, words?: typeof wording) => {
      cursor = 0;
      return lanes.useCountriesAt(client, track, shown, hour, true, tags, undefined, words);
    };
    renderLane(10); fixture.commit(); await settle();
    let lane = renderLane(10);
    assert.equal(renderLane(10, visible, a.client, wording).country(visible[0]).name, wording.pending);
    assert.equal(lane.country(visible[0]).known, undefined);
    assert.equal(lane.country(visible[0]).flagPending, true);
    assert.equal(lane.country(tags[4999]).flagPending, false, 'sorting-only names do not request artwork');
    const firstFlag = flagCalls(a.calls)[0];
    assert.deepEqual(firstFlag.input.params, { branchId: 'branch', hour: 10, tags: visible, size: 'panel' });
    renderLane(10, [...visible].reverse()); fixture.commit();
    assert.equal(flagCalls(a.calls).length, 1);
    assert.equal(firstFlag.signal.aborted, false, 're-ranking retains the exact supplying read');
    firstFlag.answer({ flags: [image(visible[0])], unavailable: [{ tag: visible[1], reason: 'Native sprite was not defined' }] });
    a.calls.find(call => call.input.product === 'country-styles')!.answer({ styles: [{ tag: visible[0], name: '', color: null },
      { tag: visible[2], name: wording.pending, color: null }, { tag: visible[3], name: wording.missing, color: null }] });
    await settle(); lane = renderLane(10);
    assert.deepEqual(lane.country(visible[0]), { name: '', known: true, nameFailed: false, flag: 'data:synthetic-7', flagPending: false, flagReason: undefined });
    for (const [tag, name] of [[visible[2], wording.pending], [visible[3], wording.missing]]) {
      assert.equal(lane.country(tag).name, name);
      assert.equal(lane.country(tag).known, true, 'native UI-word names remain known');
    }
    assert.equal(lane.country(visible[1]).flagReason, 'Native sprite was not defined');
    const namedLane = renderLane(10, visible, a.client, wording);
    assert.equal(namedLane.country(visible[0]).name, '', 'wording does not replace authored empty names');
    assert.equal(namedLane.country(visible[1]).name, wording.missing);
    assert.equal(namedLane.country(visible[1]).known, false);
    assert.equal(renderLane(10).country(visible[1]).name, 'Country name unavailable', 'a missing localized name never promotes its tag to a title');
    assert.equal(lane.country(visible[2]).flagReason, 'No readable flag was returned for this country at this hour.');
    now += 1000;
    renderLane(20); fixture.commit(); renderLane(20); fixture.commit(); await settle(); lane = renderLane(20);
    assert.equal(lane.country(visible[0]).flag, 'data:synthetic-7', 'same-stretch pending artwork keeps the previous URL');
    assert.equal(lane.country(visible[0]).flagPending, true);
    assert.equal(lane.country(visible[0]).known, true, 'a held same-holder native name stays known during replacement');
    assert.equal(lane.country(visible[1]).flagReason, undefined, 'pending status belongs to this read, not the previous refusal');
    flagCalls(a.calls).at(-1)!.fail(); await settle(); lane = renderLane(20);
    assert.equal(lane.country(visible[0]).flag, undefined);
    assert.equal(lane.country(visible[0]).flagPending, false);
    assert.equal(lane.country(visible[0]).flagReason, 'The flag request failed.');
    a.calls.find(call => call.input.product === 'country-styles' && (call.input.params as { hour: number }).hour === 20)!.fail();
    await settle(); lane = renderLane(20);
    assert.equal(lane.country(visible[0]).known, false);
    assert.equal(lane.country(visible[0]).nameFailed, true);
    now += 1000;
    renderLane(50); fixture.commit(); lane = renderLane(50); fixture.commit();
    assert.equal(lane.country(visible[0]).flagPending, false);
    assert.equal(lane.country(visible[0]).name, 'Country name unavailable');
    assert.equal(lane.country(visible[0]).known, false, 'a gap clears held name status');
    now += 1000;
    renderLane(51); fixture.commit(); renderLane(51); fixture.commit(); await settle();
    const old = flagCalls(a.calls).at(-1)!, b = transport();
    lane = renderLane(51, visible, b.client);
    assert.equal(lane.country(visible[0]).flag, undefined);
    assert.equal(old.signal.aborted, true);
    old.answer({ flags: [image(visible[0])], unavailable: [] }); await settle();
    assert.equal(renderLane(51, visible, b.client).country(visible[0]).flag, undefined);
    fixture.unmount();

    const labels = await vite.ssrLoadModule('/ui/src/dashboard/countryFlagsAt.ts') as typeof import('./countryFlagsAt.ts');
    const labelTransport = transport();
    const wanted = [{ hour: 10, tag: visible[0] }];
    const renderLabels = () => { cursor = 0; return labels.useCountryLabelsAt(labelTransport.client, track, wanted, true); };
    renderLabels(); fixture.commit(); await settle();
    labelTransport.calls.find(call => call.input.product === 'country-styles')!.answer({ styles: [{ tag: visible[0], name: '', color: null }] });
    flagCalls(labelTransport.calls)[0].answer({ flags: [], unavailable: [] });
    await settle();
    const emptyName = renderLabels().labelAt(10)(visible[0]);
    assert.equal(emptyName.name, '', 'the actual name extractor preserves a defined empty name');
    assert.equal(emptyName.known, true);
    assert.equal(emptyName.nameFailed, false);
    assert.equal(emptyName.flagPending, false);
    fixture.unmount();

    const { Lineage } = await vite.ssrLoadModule('/ui/src/dashboard/identity.ts') as typeof import('./identity.ts');
    const lineage = new Lineage([{ hour: 20, from: 'A', to: 'B' }, { hour: 20, from: 'C', to: 'A' }]);
    const segmentAt = (tag: string, hour: number) => lineage.holderStart(tag, hour);
    const aliases = transport();
    const renderAlias = (hour: number, identity: (tag: string, hour: number) => number | null = segmentAt, words?: typeof wording) => {
      cursor = 0;
      return lanes.useCountriesAt(aliases.client, track, ['A'], hour, true, ['A'], identity, words);
    };
    const atHour = (product: string, hour: number) => aliases.calls.find(call => call.input.product === product
      && (call.input.params as { hour: number }).hour === hour)!;
    const answerAlias = (hour: number, name: string, pixel: number) => {
      atHour('country-styles', hour).answer({ styles: [{ tag: 'A', name, color: null }] });
      atHour('country-flags', hour).answer({ flags: [{ ...image('A'), key: `synthetic#${pixel}`, png: `data:synthetic-${pixel}` }], unavailable: [] });
    };
    renderAlias(10); fixture.commit(); await settle(); answerAlias(10, 'Original A', 7); await settle();
    assert.equal(renderAlias(10).country('A').name, 'Original A');
    renderAlias(11); fixture.commit(); renderAlias(11); fixture.commit(); await settle();
    assert.equal(renderAlias(11).country('A').flag, 'data:synthetic-7', 'same holder retains a look during the next read');
    assert.equal(renderAlias(11).country('A').known, true);
    const delayed = atHour('country-flags', 11);
    let alias = renderAlias(20); // The 400ms sample still belongs to the old A.
    assert.equal(alias.country('A').name, 'Country name unavailable');
    assert.equal(alias.country('A').known, false, 'a throttled sample cannot certify the new holder');
    assert.equal(alias.country('A').flag, undefined);
    assert.equal(alias.country('A').flagPending, false, 'an uncertified sample is not a request for the new holder');
    fixture.commit();
    assert.equal(timers.size, 1, 'the production throttle is still awaiting its next sample');
    assert.equal(delayed.signal.aborted, true);
    answerAlias(11, 'Late old A', 8); await settle();
    assert.equal(renderAlias(20).country('A').flag, undefined, 'late old-holder replies cannot restore a look');
    now += 1000;
    renderAlias(21); fixture.commit(); renderAlias(21); fixture.commit(); await settle();
    assert.equal(timers.size, 0);
    alias = renderAlias(21);
    assert.equal(alias.country('A').name, 'Loading name…');
    assert.equal(alias.country('A').flag, undefined);
    assert.equal(alias.country('A').flagPending, true);
    assert.equal(renderAlias(21, segmentAt, wording).country('A').name, wording.pending, 'seventh-slot identity remains callable with eighth-slot wording');
    assert.equal(renderAlias(21).country('A').known, undefined);
    answerAlias(21, 'Former C', 9); await settle();
    assert.equal(renderAlias(21).country('A').name, 'Former C');
    assert.equal(renderAlias(21).country('A').flag, 'data:synthetic-9');
    alias = renderAlias(10); // Backward seek while the future holder's sample is still shown.
    assert.equal(alias.country('A').name, 'Country name unavailable');
    assert.equal(alias.country('A').flag, undefined);
    fixture.commit(); now += 1000;
    renderAlias(9); fixture.commit(); renderAlias(9); fixture.commit(); await settle(); answerAlias(9, 'Original A', 7); await settle();
    assert.equal(renderAlias(9).country('A').name, 'Original A');
    assert.equal(renderAlias(50).country('A').flag, undefined, 'current gaps mask even an earlier throttled sample');
    fixture.commit();
    assert.equal(renderAlias(51).country('A').flag, undefined, 'a sample before the gap cannot return afterward');
    fixture.commit(); now += 1000;
    renderAlias(52); fixture.commit(); renderAlias(52); fixture.commit(); await settle(); answerAlias(52, 'Former C', 9); await settle();
    assert.equal(renderAlias(52).country('A').flag, 'data:synthetic-9');
    const unknown = (_tag: string, _hour: number): number | null => null;
    alias = renderAlias(52, unknown);
    assert.equal(alias.country('A').name, 'Country name unavailable', 'unavailable identity evidence does not certify a known tag');
    assert.equal(alias.country('A').flag, undefined);
    assert.equal(alias.country('A').flagPending, false);
    fixture.unmount();

    const hoverTransport = transport();
    const fallback = (tag: string) => ({ name: `Current ${tag}`, known: true, nameFailed: false, flag: 'current-flag', flagPending: false, flagReason: 'Current reason' });
    const renderHover = (missing?: string) => { cursor = 0; return hovers.useHoverLabels(hoverTransport.client, track, true, fallback, missing, () => 0); };
    let hover = renderHover(); fixture.commit();
    hover.ask(10, visible); await settle(); hover = renderHover(); fixture.commit(); await settle();
    const hourOf = (call: ReturnType<typeof transport>['calls'][number]) => (call.input.params as { hour: number }).hour;
    const hoverFlag = flagCalls(hoverTransport.calls).find(call => hourOf(call) === 10)!;
    hover.ask(10, [...visible].reverse()); await settle(); hover = renderHover(); fixture.commit();
    assert.equal(hoverFlag.signal.aborted, false);
    assert.deepEqual(flagCalls(hoverTransport.calls).map(hourOf).sort((a, b) => a - b), [0, 10], 'reordering asks nothing new; the sampled hour is read beside it');
    assert.equal(hover.labelAt(10)(visible[0]).flag, undefined);
    assert.equal(hover.labelAt(10)(visible[0]).flagPending, true);
    assert.equal(hover.labelAt(10)(visible[0]).known, undefined, 'only a dated sample can establish a pending name');
    assert.deepEqual(hover.labelAt(11)(visible[0]), fallback(visible[0]));
    assert.deepEqual(hover.labelAt(10)(tags[4999]), fallback(tags[4999]));
    const pendingRevision = hover.revision;
    hoverFlag.answer({ flags: [image(visible[0])], unavailable: [{ tag: visible[1], reason: 'Native texture unreadable' }] });
    hoverTransport.calls.find(call => call.input.product === 'country-styles' && hourOf(call) === 10)!.answer({ styles: [{ tag: visible[0], name: '', color: null }] });
    await settle(); hover = renderHover();
    assert.deepEqual(hover.labelAt(10)(visible[0]), { name: '', known: true, nameFailed: false, flag: 'data:synthetic-7', flagPending: false, flagReason: undefined });
    assert.equal(hover.labelAt(10)(visible[1]).flagReason, 'Native texture unreadable');
    assert.equal(renderHover(wording.missing).labelAt(10)(visible[1]).name, wording.missing);
    assert.equal(renderHover(wording.missing).labelAt(10)(visible[1]).known, false);
    assert.equal(renderHover().labelAt(10)(visible[1]).name, 'Country name unavailable');
    assert.equal(renderHover(wording.missing).labelAt(10)(visible[0]).name, '');
    assert.equal(hover.labelAt(10)(visible[2]).flagPending, false);
    assert.notEqual(hover.revision, pendingRevision);
    hover.ask(20, visible); await settle(); renderHover(); fixture.commit(); await settle();
    flagCalls(hoverTransport.calls).find(call => hourOf(call) === 20)!.fail(); await settle(); hover = renderHover();
    assert.equal(hover.labelAt(20)(visible[0]).flag, undefined);
    assert.equal(hover.labelAt(20)(visible[0]).flagReason, 'The flag request failed.');
    hoverTransport.calls.find(call => call.input.product === 'country-styles' && (call.input.params as { hour: number }).hour === 20)!.fail();
    await settle();
    assert.equal(renderHover(wording.missing).labelAt(20)(visible[0]).name, wording.missing);
    assert.equal(renderHover(wording.missing).labelAt(20)(visible[0]).nameFailed, true);
    assert.equal(renderHover().labelAt(20)(visible[0]).name, 'Country name unavailable', 'a failed name read does not show its country tag');
    assert.deepEqual(hover.labelAt(10)(visible[0]), fallback(visible[0]), 'a different supplying hour never borrows this answer');
    flagCalls(hoverTransport.calls).find(call => hourOf(call) === 0)!.answer({ flags: [image(visible[0])], unavailable: [] });
    hoverTransport.calls.find(call => call.input.product === 'country-styles' && hourOf(call) === 0)!.answer({ styles: [{ tag: visible[0], name: 'Sampled', color: null }] });
    hover.ask(30, visible); await settle(); renderHover(); fixture.commit(); await settle(); hover = renderHover();
    assert.equal(hover.labelAt(30)(visible[0]).name, 'Sampled', 'an hour still being read shows its sampled look at once');
    assert.equal(hover.labelAt(30)(visible[0]).flag, 'data:synthetic-7');
    hover.ask(60, visible); await settle(); renderHover(); fixture.commit(); await settle(); hover = renderHover();
    assert.deepEqual([hover.labelAt(60)(visible[0]).name, hover.labelAt(60)(visible[0]).flag], ['Loading name\u2026', undefined],
      'a sample before a gap is never held after it');
    fixture.unmount();

    // Unknown continuity cannot borrow an earlier sample or the chart's current look.
    const unproven = transport();
    const renderUnproven = () => { cursor = 0; return hovers.useHoverLabels(unproven.client, track, true, fallback); };
    let unknownHover = renderUnproven(); fixture.commit();
    unknownHover.ask(10, ['A']); await settle(); renderUnproven(); fixture.commit(); await settle();
    unknownHover = renderUnproven();
    assert.deepEqual(unproven.calls.map(hourOf), [10, 10], 'without identity evidence only the exact hour is read');
    assert.equal(unknownHover.labelAt(10)('A').name, 'Loading name\u2026');
    assert.equal(unknownHover.labelAt(10)('A').known, undefined);
    assert.equal(unknownHover.labelAt(10)('A').flag, undefined);
    fixture.unmount();

    // 100 distinct segment starts plus hour 700 must all be admitted, with 5000 contributors.
    const many = transport(), largeTrack = { ...track, branch: { ...track.branch, lastHour: 1000, gaps: [] } };
    const starts = new Map(tags.map((tag, index) => [tag, index % 100 + 1]));
    const manySegment = (tag: string) => starts.get(tag) ?? null;
    const renderMany = () => { cursor = 0; return hovers.useHoverLabels(many.client, largeTrack, true, fallback, undefined, manySegment); };
    let manyHover = renderMany(); fixture.commit();
    manyHover.ask(700, tags); await settle(); renderMany(); fixture.commit(); await settle();
    let answered = 0, peakStyles = 0, peakFlags = 0;
    for (let pass = 0; pass < 1000; pass++) {
      const going = many.calls.slice(answered);
      peakStyles = Math.max(peakStyles, going.filter(call => call.input.product === 'country-styles').length);
      peakFlags = Math.max(peakFlags, going.filter(call => call.input.product === 'country-flags').length);
      if (!going.length) break;
      answered = many.calls.length;
      for (const call of going) {
        const { tags: requested, hour } = call.input.params as { tags: string[]; hour: number };
        call.answer(call.input.product === 'country-styles'
          ? { styles: requested.map(tag => ({ tag, name: hour === 700 ? `Exact ${tag}` : `Sample ${tag}`, color: null })) }
          : { flags: [], unavailable: requested.map(tag => ({ tag, reason: `No synthetic flag at ${hour}` })) });
      }
      await settle();
    }
    manyHover = renderMany();
    for (const product of ['country-styles', 'country-flags']) {
      const calls = many.calls.filter(call => call.input.product === product);
      assert.equal(new Set(calls.map(hourOf)).size, 101, `${product} retains every distinct wanted date`);
      const exact = calls.filter(call => hourOf(call) === 700).flatMap(call => (call.input.params as { tags: string[] }).tags);
      assert.equal(new Set(exact).size, 5000, `${product} eventually reads the exact hovered hour for every contributor`);
    }
    assert.ok(peakStyles <= 2 && peakFlags <= 2, 'date admission does not increase either fact queue concurrency');
    assert.equal(manyHover.labelAt(700)(tags[4999]).name, `Exact ${tags[4999]}`);
    assert.equal(manyHover.labelAt(700)(tags[4999]).flagReason, 'No synthetic flag at 700');
  } finally {
    fixture.unmount(); Date.now = dateNow; await vite.close(); delete host.__countryLabelHooks;
  }
});

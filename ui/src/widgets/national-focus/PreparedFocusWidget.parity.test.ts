import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type { WidgetProps } from '../../dashboard/definitions';
import type { FocusReply } from '../../generated/dashboard';
import type { ParityList, ParityPreparation } from '../../dashboard/parityRows';
import type { PreparedFocusHour } from './preparedFocusData';

test('the prepared widget supplies its settled page and description to one parity provider', async () => {
  type Slot = { value?: unknown; deps?: readonly unknown[]; cleanup?: () => void };
  type Provider = Partial<ParityPreparation> & { read: () => ParityList };
  const fixture = { cursor: 0, slots: [] as Slot[], effects: [] as { slot: Slot; setup: () => unknown }[], providers: [] as Provider[],
    completions: [] as { open: boolean; onToggle: (open: boolean) => void }[], calls: [] as string[], statuses: 0,
    emptyDetails: new Map(), markup: '', oracle: undefined as FocusReply | undefined };
  const host = globalThis as typeof globalThis & { __focusCapture?: typeof fixture; __focusCaptureHook?: (name: string, ...args: any[]) => any };
  host.__focusCapture = fixture;
  host.__focusCaptureHook = (name, ...args) => {
    const slot = fixture.slots[fixture.cursor++] ??= {};
    if (name === 'useMemo' || name === 'useCallback' || name === 'useEffect') {
      const [factory, deps] = args;
      if (!slot.deps || deps.some((value: unknown, index: number) => !Object.is(value, slot.deps![index]))) {
        slot.deps = [...deps];
        if (name === 'useMemo') slot.value = factory();
        else if (name === 'useCallback') slot.value = factory;
        else fixture.effects.push({ slot, setup: factory });
      }
      return slot.value;
    }
    if (!('value' in slot)) slot.value = name === 'useRef' ? { current: args[0] } : args[0];
    if (name === 'useRef') return slot.value;
    return [slot.value, (next: unknown) => { slot.value = typeof next === 'function' ? next(slot.value) : next; }];
  };
  const replacements: Record<string, string> = {
    '/dashboard/playback.ts': 'export const usePlaybackHour = clock => clock.getHour();',
    '/dashboard/contentRevision.ts': 'export const useContentRevision = () => 0;',
    '/dashboard/useDiagnostics.ts': 'export const useDiagnostics = () => {};',
    '/components/ContextMenu.tsx': 'export const useContextMenu = () => ({ open(){}, menu:null });',
    '/dashboard/products.ts': 'export const useProduct = () => ({data:globalThis.__focusCapture.oracle}); export const useFollowedProduct = (client, request, moving, ...rest) => ({ ...useProduct(client, request, ...rest), moment: request ? JSON.stringify(moving.map(name => request.params?.[name])) : undefined });',
    '/national-focus/useFocusArtwork.ts': `export const useFocusArtwork = (_client, _track, _tag, wanted) => ({
      read: new Map(wanted.map(({tag,hour}) => [JSON.stringify([hour,tag]), {artwork:{token:tag,name:'Observed',description:null,baseDurationDays:7,icon:null},reason:null}])),
      details: globalThis.__focusCapture.emptyDetails, warnings: [] });
      export const useContinuousFocusArtwork = () => ({ read: new Map(), warnings: [], details: new Map() });`,
    '/dashboard/parityRows.ts': `export const useParityRows = (_root, read, preparation) => {
      if (_root === null) return;
      globalThis.__focusCapture.providers.push({read,...preparation}); };`,
    '/components/VirtualList.tsx': `import { createElement } from 'react';
      export const VirtualList = props => createElement('ol', {className:props.className}, props.items.map((item, index) => {
        const child = props.children(item); if (item.kind === 'completed') globalThis.__focusCapture.completions.push(child.props);
        return createElement('li', {key:index, ...props.rowProps?.(item)}, child); }));`,
    '/charts/WindowedRows.tsx': `import { createElement } from 'react';
      export default props => {
        if (props.parity !== false) globalThis.__focusCapture.providers.push({read:()=>({total:props.items.length})});
        return createElement('div', {className:props.className}, props.items.map(row => createElement('div', {key:row.key}, props.children({item:row.item})))); };`,
  };
  const vite = await createServer({ configFile: false, mode: 'parity', optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'focus-capture-fixture', enforce: 'pre',
      transform(source, id) {
        const path = id.replaceAll('\\', '/'), replacement = Object.entries(replacements).find(([suffix]) => path.endsWith(suffix));
        if (replacement) return replacement[1];
        if (['/national-focus/PreparedFocusWidget.tsx', '/national-focus/usePreparedFocusParity.ts', '/dashboard/parityPreparation.ts'].some(suffix => path.endsWith(suffix)))
          return source.replace(/\b(useMemo|useCallback|useState|useRef|useEffect)(?:<[\s\S]*?>)?\(/g,
            (_match, name) => `globalThis.__focusCaptureHook(${JSON.stringify(name)}, `);
      },
    }] });
  try {
    const { default: Widget } = await vite.ssrLoadModule('/ui/src/widgets/national-focus/PreparedFocusWidget.tsx');
    const row: PreparedFocusHour = { schema: 6, slot: 1, countryId: 7, hour: 30, lastHour: 40, orderFirstHour: 10,
      current: 'active', continuous: null, paused: true, progressRaw: '1000000', dailyGainRaw: null,
      completed: { first: '0', count: '1', rows: '2' }, at: 35 };
    const size = () => Number(row.completed.rows);
    const entry = (ordinal: number) => ({ ordinal, token: `completed-${ordinal}`, completedByHour: null });
    const tokens = () => [...Array.from({length:size()}, (_, index) => entry(index).token), ...(row.current ? [row.current] : [])];
    const staticArt = (token: string) => ({token,name:'Observed',description:null,baseDurationDays:7,icon:null});
    const client = { preparationStatus: async (request: any) => { fixture.statuses++; return { ...request, state: 'ready', sections: [] }; },
      preparedQuery: async (_source: unknown, product: string, params: any) => {
        fixture.calls.push(product);
        if (product === 'focus-hour') return { focus: { ...row, at: params.hour } };
        if (product === 'focus-order-page') return Array.from({length:Math.min(128, size()-Number(params.page)*128)}, (_, index) => entry(Number(params.page)*128+index));
        if (product === 'focus-static-text') return { texts: params.tokens.map(staticArt), warnings: [] };
        throw new Error(product);
      },
      product: async (request: any) => {
        const { mode, tokens: asked } = request.params;
        fixture.calls.push(`focus-art:${mode}`);
        assert.ok(asked.every((token: string) => tokens().includes(token)));
        return { sourceId: request.sourceId, revision: request.revision, product: request.product, data: { warnings: [], failures: [],
          art: asked.map((token: string) => ({ artwork: { ...staticArt(token), description: mode === 'text' ? 'Scoped details' : null }, reason: null })) } };
      },
    };
    const props = { widget: { id: 'focus', kind: 'national-focus', trackIds: ['main'], countries: { kind: 'tags', tags: ['opaque'] }, time: { mode: 'follow' } },
      runtime: { client, clock: { getHour: () => 35 }, tracks: [{ id: 'main', axisOffset: 0, source: { id: 'synthetic', revision: '1' },
        branch: { id: '1', firstHour: 0, lastHour: 40, gaps: [] } }] }, active: true, headerSlot: null, onDiagnostics() {} } as unknown as WidgetProps;
    const render = () => {
      fixture.cursor = 0; fixture.effects = []; fixture.providers = []; fixture.completions = [];
      const tree = Widget(props);
      fixture.markup = renderToStaticMarkup(tree);
      // As React does: every changed effect's cleanup runs before any of their setups.
      for (const { slot } of fixture.effects) slot.cleanup?.();
      for (const { slot, setup } of fixture.effects) { const cleanup = setup(); slot.cleanup = typeof cleanup === 'function' ? cleanup as () => void : undefined; }
      return tree;
    };
    const settle = async () => { for (let frame = 0; frame < 8; frame++) { await new Promise(resolve => setImmediate(resolve)); render(); } };
    const { LegacyNationalFocusWidget: Legacy } = await vite.ssrLoadModule('/ui/src/widgets/national-focus/NationalFocusWidget.tsx');
    const compareOracle = () => {
      fixture.oracle = { history: { segments: [{firstHour:10,lastHour:40,order:{completed:Array.from({length:size()}, (_, index) => entry(index)),current:row.current,continuous:null,paused:true}}],
        progress:[{firstHour:30,lastHour:40,progressRaw:'1000000'}] }, artwork:tokens().map(staticArt), staticText:tokens().map(staticArt), continuousStaticText:[], warnings:[] };
      const oracle = renderToStaticMarkup(createElement(Legacy, props));
      assert.equal(fixture.markup.replace('national-focus prepared-focus','national-focus').replace(' aria-busy="false"',''), oracle,
        'settled prepared rows and empty/current layouts match the legacy renderer');
    };
    render(); await settle();
    assert.equal(fixture.calls.length, 5, 'the visible page reads one point, order, static text and art at its two hours');
    assert.deepEqual(fixture.calls.filter(call => call.startsWith('focus-art')), ['focus-art:display', 'focus-art:display']);
    assert.equal(fixture.statuses, 0);
    assert.equal(fixture.providers.length, 1, 'the complete parent model suppresses the page-only child model');
    fixture.providers[0].prepare!(); render(); await settle();
    assert.equal(fixture.calls.length, 5, 'full-model preparation reuses the visible static text too');
    assert.equal(fixture.providers[0].ready!(), true);
    assert.equal(fixture.providers[0].read().total, 2);
    compareOracle();
    assert.doesNotMatch(fixture.markup, /Completion pages|focus-prepared-row|role="region"/);
    assert.match(fixture.markup, /<details class="focus-details"><summary class="focus-row">/);
    fixture.completions[0].onToggle(true); render(); await settle();
    assert.equal(fixture.providers[0].ready!(), true);
    assert.equal(fixture.calls.filter(call => call === 'focus-art:text').length, 1, 'reading the complete model reuses the opened row description');
    assert.equal((fixture.providers[0].read().artwork as any[])[0].art.artwork.description, 'Scoped details');
    assert.match(fixture.markup, /<details class="focus-details" open=""><summary[\s\S]*Base duration <strong>7 days<\/strong>[\s\S]*Scoped details/);
    fixture.completions[1].onToggle(true); render(); await settle();
    assert.deepEqual(fixture.completions.map(row => row.open), [true,true], 'completion disclosures open independently');
    assert.ok((fixture.providers[0].read().artwork as any[]).every(row => row.art.descriptionDemanded));
    assert.equal(fixture.calls.filter(call => call === 'focus-art:text').length, 2, 'opening another row retains the accepted first description');
    fixture.completions[0].onToggle(false); render(); await settle();
    assert.deepEqual(fixture.completions.map(row => row.open), [false,true]);
    const count = fixture.calls.length;
    render(); await settle();
    assert.equal(fixture.calls.length, count, 'settled redraws do not restart request loops');
    const reset = () => { for (const slot of fixture.slots) slot.cleanup?.(); fixture.slots = []; fixture.calls = []; };
    row.current = null;
    Object.assign(row.completed,{count:'0',rows:'0'});
    for (const mode of ['campaign','follow'] as const) {
      reset(); props.widget.time = {mode}; render(); await settle(); compareOracle();
      assert.doesNotMatch(fixture.markup, /Completion pages|<ol|focus-number/);
      assert.match(fixture.markup, /No completed focuses recorded at this point/);
      assert.deepEqual(fixture.calls,['focus-hour']);
    }
    reset(); row.current = 'active'; Object.assign(row.completed,{count:'2',rows:'129'});
    props.widget.time = {mode:'follow'}; render(); await settle();
    assert.match(fixture.markup, /aria-label="Completion pages"/);
    assert.match(fixture.markup, /Page 2 of 2/);
    assert.equal(fixture.completions.length,1);
    assert.match(fixture.markup, /aria-label="Focus 129"/);
    props.widget.time = {mode:'campaign'}; reset(); render(); await settle();
    const tree = render(), navigation = tree.props.children.find((child: any) => child?.type === 'nav');
    navigation.props.children.at(-1).props.onClick(); render(); await settle();
    assert.match(fixture.markup, /class="focus-entry is-unfinished"/);
    assert.match(fixture.markup, /focus-number" aria-hidden="true">130/);
    assert.equal(fixture.completions.length,1, 'the unfinished row is noninteractive and remains inside the ordered list');
  } finally {
    for (const slot of fixture.slots) slot.cleanup?.();
    delete host.__focusCapture; delete host.__focusCaptureHook; await vite.close();
  }
});

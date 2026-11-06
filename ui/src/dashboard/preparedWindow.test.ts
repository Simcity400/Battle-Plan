import assert from 'node:assert/strict';
import test from 'node:test';
import { createServer } from 'vite';
import type { DashboardClient, DashboardSource } from './client.ts';

test('prepared windows retain exact data while loading geometry and reject old source/content/navigation replies', async () => {
  type Cell = { value?: unknown; deps?: readonly unknown[]; cleanup?: () => void };
  const cells: Cell[] = []; let index = 0, content = 0;
  const effects: (() => void)[] = [];
  const same = (a: readonly unknown[] | undefined, b: readonly unknown[]) => a?.length === b.length && a.every((item, i) => Object.is(item, b[i]));
  const cell = () => cells[index++] ??= {};
  const memo = (build: () => unknown, deps: readonly unknown[]) => { const at = cell(); if (!same(at.deps, deps)) { at.value = build(); at.deps = deps; } return at.value; };
  const hooks = { content: () => content, useMemo: memo, useCallback: (fn: unknown, deps: readonly unknown[]) => memo(() => fn, deps),
    useRef: (value: unknown) => { const at = cell(); return at.value ??= { current: value }; },
    useState: (value: unknown) => { const at = cell(); if (!('value' in at)) at.value = value;
      return [at.value, (next: unknown) => { at.value = typeof next === 'function' ? next(at.value) : next; }]; },
    useEffect: (effect: () => (() => void) | undefined, deps: readonly unknown[]) => { const at = cell();
      if (!same(at.deps, deps)) { at.deps = deps; effects.push(() => { at.cleanup?.(); at.cleanup = effect(); }); } },
  };
  const host = globalThis as typeof globalThis & { __preparedWindow?: typeof hooks }; host.__preparedWindow = hooks;
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true }, server: { middlewareMode: true, watch: null, hmr: false, ws: false },
    plugins: [{ name: 'prepared-window-hooks', enforce: 'pre', transform(source, id) {
      const path = id.replaceAll('\\', '/');
      if (path.endsWith('/dashboard/preparedSeries.ts') || path.endsWith('/dashboard/viewRead.ts')) return source.replace(/import \{ ([^}]+) \} from 'react';/, 'const { $1 } = globalThis.__preparedWindow;');
      if (path.endsWith('/dashboard/contentRevision.ts')) return 'export const useContentRevision = () => globalThis.__preparedWindow.content();';
    } }] });
  const requests: { source: DashboardSource; product:string; params: unknown; signal: AbortSignal; resolve: (value: unknown) => void; reject: (error: Error) => void }[] = [];
  const client = { preparedQuery: (source: DashboardSource, product: string, params: unknown, options: { signal: AbortSignal }) =>
    new Promise((resolve, reject) => requests.push({ source, product, params, signal: options.signal, resolve, reject })) } as unknown as DashboardClient;
  let source = { id: 'a', revision: '1' } as DashboardSource, active = true, hour=10;
  try {
    const { usePreparedWindow } = await vite.ssrLoadModule('/ui/src/dashboard/preparedSeries.ts');
    const render = () => { index = 0; const result = usePreparedWindow(client, source, 'autonomy', { branchId: '1' }, [0,100], [hour], active, (value: unknown) => value,(a:object,b:object)=>({...a,...b}));
      for (const effect of effects.splice(0)) effect(); return result; };
    const tick = async () => { for (let i = 0; i < 6; i++) await Promise.resolve(); };
    const latest=(product:string)=>requests.filter(request=>request.product===product).at(-1)!;
    render(); latest('series-summary').resolve({geometry:'first'});latest('series-state').resolve({exact:10});await tick();
    let result=render();assert.deepEqual(result.data,{geometry:'first',exact:10});
    result.onSummaryView({from:0,to:100,width:500},[{id:'[1,10]',spans:[{tag:'A',firstHour:0,lastHour:100,startsSegment:true}]}]);
    result=render();assert.deepEqual(result.data,{geometry:'first',exact:10});assert.equal(result.geometryReady,false);
    assert.deepEqual(latest('series-summary').params,{branchId:'1',family:{kind:'autonomy'},from:0,to:100,width:500,within:{'[1,10]':[['A',0,100,true]]}});
    latest('series-summary').resolve({geometry:'selected holder'});await tick();result=render();assert.equal(result.geometryReady,true);
    const summaryCount=requests.filter(request=>request.product==='series-summary').length;
    hour=12;render();assert.equal(requests.filter(request=>request.product==='series-summary').length,summaryCount,'playback reads exact values without reloading geometry');
    assert.deepEqual(latest('series-state').params,{branchId:'1',family:{kind:'autonomy'},reads:[{hour:12}]});
    latest('series-state').resolve({exact:12});await tick();result=render();assert.deepEqual(result.data,{geometry:'selected holder',exact:12});
    const navigation=result.navigate('[1,10]',12,true),nav=latest('series-next-change');
    assert.deepEqual(nav.params,{branchId:'1',family:{kind:'autonomy'},tag:'[1,10]',within:{'[1,10]':[['A',0,100,true]]},key:'ArrowRight',from:12,firstHour:0,lastHour:100});
    source={id:'b',revision:'2'} as DashboardSource;result=render();assert.equal(result.data,undefined);
    assert.ok(nav.signal.aborted);nav.resolve({hour:13});await assert.rejects(navigation,{name:'AbortError'});
    const oldSummary=latest('series-summary'),oldState=latest('series-state');
    content++;render();assert.ok(oldSummary.signal.aborted && oldState.signal.aborted);
    oldSummary.resolve({wrong:'old content'});oldState.resolve({wrong:'old content'});
    latest('series-summary').resolve({geometry:'new content'});latest('series-state').resolve({exact:12});await tick();result=render();
    assert.deepEqual(result.data,{geometry:'new content',exact:12});
    // A failed keyboard read on the next campaign never brings back this one's exact data.
    source={id:'c',revision:'3'} as DashboardSource;result=render();assert.equal(result.data,undefined);
    latest('series-summary').resolve({geometry:'c'});await tick();result=render();
    result.onNavigateHour('ArrowRight',12,[0,100],null);latest('series-next-change').reject(new Error('navigation failed'));await tick();
    result=render();assert.deepEqual(result.data,{geometry:'c'});assert.match(String(result.error),/navigation failed/);
    result.onSummaryView({from:20,to:30,width:50});render();const departing=latest('series-summary');active=false;render();
    assert.ok(departing.signal.aborted,'leaving the screen cancels viewport reads');
  } finally { for (const at of cells) at.cleanup?.(); delete host.__preparedWindow; await vite.close(); }
});

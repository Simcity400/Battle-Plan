import { gatedHour } from '../test-support/gated.ts';
import { Profiler, useEffect, useLayoutEffect, useRef, useState, useSyncExternalStore } from 'react';
import { flushSync } from 'react-dom';
import { createRoot } from 'react-dom/client';
import type { Root } from 'react-dom/client';
import PreparedPages from './PreparedPages';
import type { PreparedPage } from './PreparedPages';
import type { DashboardClient, ProductReply, ProductRequest } from './client';
import type { PlaybackController } from './playback';
import type { WidgetRuntime } from './definitions';
import type { DashboardTemplate } from './model';
import { preparingClient, prepareGate } from './prepare.ts';
import { loadProduct, useFollowedProduct, useProduct, useProducts } from './products.ts';
import { forgetProducts } from './products.ts';
import EventFeed from '../widgets/event-feed/index.tsx';
import type { FamilyViewProps } from './definitions';
import type { BoundTrack } from './model';

const START = 30_000;

class Clock {
  hour = START;
  playing = false;
  seekSequence = 1;
  private snapshot = this.make();
  private listeners = new Set<() => void>();
  private frameListeners = new Set<() => void>();
  private make() { return { position: this.hour, hour: this.hour, fraction: 0, playing: this.playing, hoursPerSecond: 1440, sequence: 0, seekSequence: this.seekSequence, bounds: null }; }
  getSnapshot = () => this.snapshot;
  getHour = () => this.hour;
  getMotion = () => this.playing ? 1440 : undefined;
  subscribe = (listener: () => void) => { this.listeners.add(listener); return () => { this.listeners.delete(listener); }; };
  subscribeFrame = (listener: () => void) => { this.frameListeners.add(listener); return () => { this.frameListeners.delete(listener); }; };
  anchor() { this.snapshot = this.make(); for (const listener of [...this.listeners]) listener(); }
  step(hours: number) { this.hour += hours; for (const listener of [...this.frameListeners]) listener(); }
}

let clock = new Clock();
let root: Root | null = null;
let host: HTMLElement | null = null;
const requests: { product: string; hour: number; playing: boolean }[] = [];
let visibleCommits = 0;
let preparedRenders = 0;
let playingRenders = 0;

const client = {
  available: true,
  product<T>(request: ProductRequest, signal: AbortSignal): Promise<ProductReply<T>> {
    requests.push({ product: request.product, hour: (request.params as { hour: number }).hour, playing: clock.playing });
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => resolve({ sourceId: request.sourceId, revision: request.revision, product: request.product,
        data: { rows: Array.from({ length: 400 }, (_, index) => index) } as T }), 5);
      signal.addEventListener('abort', () => { clearTimeout(timer); reject(new DOMException('Request cancelled', 'AbortError')); }, { once: true });
    });
  },
} as unknown as DashboardClient;

const ask = (product: string, hour: number) => ({ sourceId: 'fixture', revision: '1', product, params: { hour: gatedHour(hour) } }) as ProductRequest;

function ShownPage() {
  const blockOf = () => Math.floor(clock.getHour() / 2880) * 2880;
  const [block, setBlock] = useState(blockOf);
  useLayoutEffect(() => clock.subscribeFrame(() => setBlock(blockOf)), []);
  const reply = useProduct<{ rows: number[] }>(client, ask('shown', block), true);
  const text = useRef<HTMLSpanElement>(null);
  useLayoutEffect(() => clock.subscribeFrame(() => { if (text.current) text.current.textContent = String(clock.getHour()); }), []);
  return <div><span ref={text} />{reply.data ? <i>{reply.data.rows.length}</i> : null}</div>;
}

function OtherPage({ page }: { page: PreparedPage }) {
  preparedRenders++;
  if (useSyncExternalStore(page.runtime.clock.subscribe, page.runtime.clock.getSnapshot).playing) playingRenders++;
  const reply = useProduct<{ rows: number[] }>(page.runtime.client, ask(page.id, clock.getHour()), true);
  return <ul>{reply.data?.rows.map(row => <li key={row}>{page.id} {row}</li>)}</ul>;
}

const frame = () => new Promise<number>(resolve => requestAnimationFrame(resolve));

function pages(): PreparedPage[] {
  const runtime = { client: preparingClient(client), clock: clock as unknown as PlaybackController } as unknown as WidgetRuntime;
  return ['politics', 'economy'].map(id => ({ id, template: { widgets: [] } as unknown as DashboardTemplate, runtime }));
}

const fixture = {
  async eventFeedKinds() {
    root?.unmount(); host?.remove();
    clock = new Clock();
    let count = 0;
    const counting = { available: true, product: async (request: ProductRequest) => {
      if (request.product !== 'event-feed-kinds') throw new Error(`Unexpected product ${request.product}`);
      count++;
      return { sourceId: request.sourceId, revision: request.revision, product: request.product,
        data: { kinds: [{ kind: 'warDeclared', category: 'warsPeace', firstObserved: START, unavailable: null }] } };
    } } as unknown as DashboardClient;
    const track = { id: 'main', source: { id: 'kinds', revision: '1' }, axisOffset: 0, branch: { id: 'branch', firstHour: START, lastHour: START + 1000 } } as BoundTrack;
    const runtime = { client: counting, tracks: [track], clock } as unknown as WidgetRuntime;
    const prepared = [{ id: 'feed', template: { widgets: [] } as unknown as DashboardTemplate,
      runtime: { ...runtime, client: preparingClient(counting) } }];
    host = document.createElement('div');
    document.body.append(host);
    root = createRoot(host);
    function Feeds({ runtime }: { runtime: WidgetRuntime }) {
      return <>{['first', 'second'].map(id => <EventFeed.render key={id} {...{
        widget: { id, trackIds: ['main'], time: { mode: 'follow' }, countries: { kind: 'all' } },
        runtime, active: true, options: { activityStrip: false },
      } as unknown as FamilyViewProps} />)}</>;
    }
    const draw = (visible: boolean) => root!.render(<>
      {visible ? <Feeds runtime={runtime} /> : null}
      <PreparedPages pages={prepared} clock={runtime.clock} client={counting} viewport={host} enabled
        draw={page => <Feeds runtime={page.runtime} />} />
    </>);
    const prepare = async () => {
      let seen = false;
      for (let at = 0; at < 300; at++) {
        await frame();
        seen ||= fixture.hostShown();
        if (seen && !fixture.hostShown() && prepareGate(counting).pending === 0) return;
      }
      throw new Error('The event feed page did not finish preparing');
    };
    draw(false);
    await prepare();
    draw(true);
    await frame(); await frame();
    const before = count;
    clock.playing = true; clock.anchor();
    for (let at = 0; at < 12; at++) { clock.step(15); await frame(); }
    clock.playing = false; clock.anchor();
    await prepare();
    for (const hour of [START + 7, START + 31]) {
      clock.hour = hour; clock.seekSequence++; clock.anchor();
      await prepare();
    }
    root.unmount(); root = null; host.remove(); host = null;
    return { before, after: count };
  },
  async eventFeedKindsState(state: 'pending' | 'failed' | 'missing' | 'ready') {
    let deliver = () => {};
    const delivered = new Promise<void>(resolve => { deliver = resolve; });
    const reading = { available: true, product: async (request: ProductRequest) => {
      if (state === 'pending') return new Promise(() => {});
      await delivered;
      if (state === 'failed') throw new Error('Kinds read failed');
      return { sourceId: request.sourceId, revision: request.revision, product: request.product,
        data: state === 'missing' ? {} : { kinds: [{ kind: 'warDeclared', category: 'warsPeace', firstObserved: START, unavailable: null }] } };
    } } as unknown as DashboardClient;
    const slot = document.body.appendChild(document.createElement('div'));
    const container = document.body.appendChild(document.createElement('div'));
    const view = createRoot(container);
    view.render(<EventFeed.render {...{
      widget: { id: 'state', trackIds: ['main'], time: { mode: 'follow' }, countries: { kind: 'all' } },
      runtime: { client: reading, tracks: [{ id: 'main', source: { id: 'state', revision: '1' }, axisOffset: 0, branch: { id: 'branch' } }], clock },
      active: true, options: { activityStrip: false, politics: false }, filterSlot: slot,
    } as unknown as FamilyViewProps} />);
    for (let at = 0; at < 300 && !slot.querySelector('button'); at++) await frame();
    slot.querySelector('button')?.click();
    const panel = () => document.querySelector('[role="dialog"][aria-label="Filter kinds"]');
    for (let at = 0; at < 300 && !panel()?.querySelector('[role="status"]'); at++) await frame();
    if (!panel()?.querySelector('[role="status"]')) throw new Error('The kinds picker did not show its loading state');
    deliver();
    const expected = state === 'pending' ? '[role="status"]' : state === 'ready' ? '[role="option"]' : '[role="alert"]';
    for (let at = 0; at < 300 && !panel()?.querySelector(expected); at++) await frame();
    if (!panel()?.querySelector(expected)) throw new Error(`The kinds picker did not reach ${state}`);
    const result = { chip: slot.querySelector('button')?.textContent?.trim(), status: panel()?.querySelector('[role="status"], [role="alert"]')?.textContent ?? null,
      choices: [...(panel()?.querySelectorAll('[role="option"]') ?? [])].map(item => item.textContent) };
    view.unmount(); slot.remove(); container.remove();
    return result;
  },
  async eventFeedKindsAvailability() {
    clock = new Clock();
    let requests = 0;
    let deliver = () => {};
    let kindsDelivered = false;
    const delivered = new Promise<void>(resolve => { deliver = resolve; });
    const changes: string[] = [];
    let excludedKinds = 'newsEventFired';
    const reading = { available: true, product: async (request: ProductRequest) => {
      requests++;
      await delivered;
      return { sourceId: request.sourceId, revision: request.revision, product: request.product, data: { kinds: [
        { kind: 'warDeclared', category: 'warsPeace', firstObserved: START, unavailable: null },
        { kind: 'newsEventFired', category: 'politics', firstObserved: START + 90, unavailable: null },
        { kind: 'lawChanged', category: 'politics', firstObserved: null, unavailable: 'Ideas were not recorded.' },
      ] } };
    } } as unknown as DashboardClient;
    const track = { id: 'main', source: { id: 'kind-availability', revision: '1' }, axisOffset: -100,
      branch: { id: 'branch', firstHour: START, lastHour: START + 1000 } } as BoundTrack;
    clock.hour = START - track.axisOffset;
    const slot = document.body.appendChild(document.createElement('div'));
    const container = document.body.appendChild(document.createElement('div'));
    const view = createRoot(container);
    const draw = () => view.render(<EventFeed.render {...{
      widget: { id: 'availability', trackIds: ['main'], time: { mode: 'follow' }, countries: { kind: 'all' } },
      runtime: { client: reading, tracks: [track], clock }, active: true, options: { activityStrip: false, excludedKinds }, filterSlot: slot,
      setOption: (key: string, value: string) => { if (key === 'excludedKinds') { changes.push(value); excludedKinds = value; draw(); } },
    } as unknown as FamilyViewProps} />);
    draw();
    const panel = () => document.querySelector('[role="dialog"][aria-label="Filter kinds"]');
    const options = () => [...(panel()?.querySelectorAll<HTMLElement>('[role="option"]') ?? [])];
    const open = async () => {
      slot.querySelector('button')!.click();
      if (!kindsDelivered) {
        for (let at = 0; at < 300 && !panel()?.querySelector('[role="status"]'); at++) await frame();
        if (!panel()?.querySelector('[role="status"]')) throw new Error('The availability picker did not show its loading state');
        deliver();
        kindsDelivered = true;
      }
      for (let at = 0; at < 300 && !options().length; at++) await frame();
      if (!options().length) throw new Error('The availability picker did not reach its ready state');
    };
    const close = async () => { slot.querySelector('button')!.click(); await frame(); };
    const snapshot = () => options().map(item => ({ label: item.querySelector('.tf-picker-label')?.textContent,
      disabled: item.getAttribute('aria-disabled') === 'true', checked: item.getAttribute('aria-selected') === 'true',
      reason: item.querySelector('[data-value-state="missing"]')?.getAttribute('aria-label') ?? null }));
    for (let at = 0; at < 300 && !slot.querySelector('button'); at++) await frame();
    await open();
    const before = snapshot();
    const resetBefore = !!panel()?.querySelector('.tf-picker-reset');
    const beforeRequests = requests;
    options().find(item => item.textContent?.includes('Law changed'))!.click();
    await close();
    clock.playing = true; clock.anchor();
    clock.step(100);
    await open();
    const later = snapshot();
    options().find(item => item.textContent?.includes('News events'))!.click();
    await frame();
    await close();
    clock.hour = START - track.axisOffset; clock.seekSequence++; clock.anchor();
    await open();
    const soughtBack = snapshot();
    options().find(item => item.textContent?.includes('News events'))!.click();
    const afterRequests = requests;
    view.unmount(); slot.remove(); container.remove();
    return { before, later, soughtBack, changes, beforeRequests, afterRequests, resetBefore };
  },
  async mount(lane: boolean) {
    root?.unmount(); host?.remove();
    forgetProducts(client, 'fixture');
    clock = new Clock();
    requests.length = 0; visibleCommits = 0; preparedRenders = 0; playingRenders = 0;
    host = document.createElement('div');
    host.style.cssText = 'width: 640px; height: 400px; overflow: auto;';
    document.body.append(host);
    root = createRoot(host);
    const viewport = host;
    const prepared = pages();
    root.render(<>
      <Profiler id="shown" onRender={() => { visibleCommits++; }}><ShownPage /></Profiler>
      <PreparedPages pages={prepared} clock={clock as unknown as PlaybackController} client={client} viewport={viewport} enabled={lane}
        draw={page => <OtherPage page={page} />} />
    </>);
    for (let at = 0; at < 600; at++) {
      await frame();
      if (!lane && at > 30) break;
      if (lane && at > 10 && !document.querySelector('[data-prepare]') && prepareGate(client).pending === 0 && requests.some(item => item.product === 'economy')) break;
    }
    await frame(); await frame();
    return { asked: requests.map(item => item.product), shownOnPause: requests.filter(item => !item.playing).length };
  },
  async play(frames: number, hours: number) {
    clock.playing = true; clock.anchor();
    let laid = false;
    await frame(); laid ||= fixture.hostShown();
    await frame(); laid ||= fixture.hostShown();
    const before = { requests: requests.length, commits: visibleCommits, prepared: preparedRenders };
    await frame(); laid ||= fixture.hostShown();
    for (let at = 0; at < frames; at++) {
      clock.step(hours);
      await frame();
      laid ||= fixture.hostShown();
    }
    clock.playing = false; clock.anchor();
    const asked = requests.slice(before.requests);
    return {
      commits: visibleCommits - before.commits,
      preparedRenders: preparedRenders - before.prepared,
      asked: asked.map(item => item.product),
      hostWhilePlaying: laid,
    };
  },
  hostShown: () => !!document.querySelector('[data-prepare]'),
  async playWhilePreparing() {
    await fixture.mount(true);
    clock.hour += 2_880; clock.seekSequence++; clock.anchor();
    let laid = false;
    for (let at = 0; at < 300 && !laid; at++) { await frame(); laid = !!document.querySelector('[data-prepare] li'); }
    clock.playing = true; clock.anchor();
    await frame();
    const shown = fixture.hostShown();
    await frame(); await frame();
    clock.playing = false; clock.anchor();
    return { laid, playingRenders, shown };
  },
  async heldFirstRender() {
    root?.unmount(); host?.remove();
    forgetProducts(client, 'fixture');
    const held = ask('held', 7);
    await loadProduct(preparingClient(client), held, new AbortController().signal);
    const first: Record<string, boolean> = {};
    function Reader({ name, data }: { name: string; data: unknown }) {
      if (!(name in first)) first[name] = data !== undefined;
      return null;
    }
    function One() { return <Reader name="product" data={useProduct(client, held, true).data} />; }
    function Followed() { return <Reader name="followed" data={useFollowedProduct(client, held, ['hour'], true).data} />; }
    host = document.createElement('div');
    document.body.append(host);
    root = createRoot(host);
    root.render(<><One /><Followed /></>);
    await frame(); await frame();
    return first;
  },
  async heldBetweenRenders() {
    root?.unmount(); host?.remove();
    forgetProducts(client, 'fixture');
    const held = ask('between', 9);
    const ran: Record<string, boolean> = {};
    const nudges: ((value: number) => void)[] = [];
    function useNudged(name: string, data: unknown) {
      const [, nudge] = useState(0);
      nudges.push(nudge);
      useEffect(() => { if (data !== undefined) ran[name] = true; }, [data]);
    }
    function One() { useNudged('product', useProduct(client, held, true).data); return null; }
    function Many() { useNudged('products', useProducts(client, [held], true).data); return null; }
    function Followed() { useNudged('followed', useFollowedProduct(client, held, ['hour'], true).data); return null; }
    host = document.createElement('div');
    document.body.append(host);
    root = createRoot(host);
    root.render(<><One /><Many /><Followed /></>);
    await frame();
    await loadProduct(client, held, new AbortController().signal);
    flushSync(() => { for (const nudge of nudges) { nudge(1); nudge(0); } });
    await frame(); await frame();
    return { product: !!ran.product, products: !!ran.products, followed: !!ran.followed };
  },
};

Object.assign(window, { prepareLane: fixture });

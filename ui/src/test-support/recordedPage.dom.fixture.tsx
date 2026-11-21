import { attributeCommits } from '../shell/commitCount.ts';
import { createRoot } from 'react-dom/client';
import type { Root } from 'react-dom/client';
import '@fontsource-variable/inter';
import '@fontsource-variable/roboto';
import '@fontsource-variable/roboto-condensed';
import '@fontsource-variable/source-serif-4';
import '@fontsource-variable/cascadia-code';
import '../styles.css';
import '../shell/shell.css';
import '../dashboard/dashboard.css';
import '../widgets';
import DashboardGrid, { bodiesWaiting } from '../dashboard/DashboardGrid';
import PreparedPages from '../dashboard/PreparedPages';
import type { PreparedPage } from '../dashboard/PreparedPages';
import { preparingClient } from '../dashboard/prepare.ts';
import type { DashboardClient } from '../dashboard/client';
import { loadWidgetRenderer } from '../widgets/rendererLoaders';
import { bindTracks, clockBounds, defaultTemplate } from '../dashboard/model';
import type { DashboardTemplate } from '../dashboard/model';
import { PAGES } from '../dashboard/pages';
import type { PageId } from '../dashboard/pages';
import { PlaybackController } from '../dashboard/playback';
import { MapViewsStore } from '../dashboard/geometry';
import { LiveMotion } from '../charts/liveMotion';
import { busyCount } from '../shell/busy.ts';
import { measureWith, paramsFrom } from '../shell/perf.ts';
import type { PerfRecorder } from '../shell/perf.ts';
import { applySavedAppearance } from '../shell/appearance/useAppearance';
import type { WidgetRuntime } from '../dashboard/definitions';
import { loadRecorded } from './recorded.ts';
import type { Recorded, RecordedAsk, RecordedClient, RecordedMark } from './recorded.ts';

applySavedAppearance();
const native = window.requestAnimationFrame.bind(window);
const frame = () => new Promise<number>(resolve => native(now => resolve(now)));

const shown = new Map<string, number | null>();
const paints = new Map<string, number[]>();
const asked = { layout: 0, measure: 0 };
let counting = false;
measureWith(new Proxy({
  shown: (widget: string, hour: number | null) => { shown.set(widget, hour); },
  paint: (widget: string, ms: number) => { if (counting) { const held = paints.get(widget); if (held) held.push(ms); else paints.set(widget, [ms]); } },
}, { get: (target, name) => (target as Record<string | symbol, unknown>)[name] ?? (() => {}) }) as unknown as PerfRecorder);
for (const [owner, names] of [[Element.prototype, ['clientWidth', 'clientHeight', 'scrollTop', 'scrollHeight']], [HTMLElement.prototype, ['offsetWidth', 'offsetHeight']]] as const) {
  for (const name of names) {
    const property = Object.getOwnPropertyDescriptor(owner, name)!;
    Object.defineProperty(owner, name, { ...property, get(this: Element) { if (counting) asked.layout++; return property.get!.call(this); } });
  }
}
const computed = window.getComputedStyle;
window.getComputedStyle = ((...args: Parameters<typeof computed>) => { if (counting) asked.layout++; return computed.apply(window, args); }) as typeof computed;
const rect = Element.prototype.getBoundingClientRect;
Element.prototype.getBoundingClientRect = function () { if (counting) asked.layout++; return rect.call(this); };
type Context = CanvasRenderingContext2D | OffscreenCanvasRenderingContext2D;
for (const proto of [CanvasRenderingContext2D.prototype, OffscreenCanvasRenderingContext2D.prototype] as Context[]) {
  const measureText = proto.measureText;
  proto.measureText = function (this: Context, text: string) { if (counting) asked.measure++; return measureText.call(this, text); };
}

const commits = new Map<string, number>();
const why = new Map<string, Map<string, number>>();
attributeCommits((zones, causes) => {
  if (!counting) return;
  for (const zone of zones) if (zone.startsWith('widget:')) commits.set(zone.slice(7), (commits.get(zone.slice(7)) ?? 0) + 1);
  for (const [zone, named] of causes) {
    const id = zone.replace('widget:', '');
    let held = why.get(id);
    if (!held) why.set(id, held = new Map());
    for (const [cause, count] of named) held.set(cause, (held.get(cause) ?? 0) + count);
  }
});

let recorded: Recorded | null = null;
let root: Root | null = null, host: HTMLDivElement | null = null;
let clock: PlaybackController | null = null, client: RecordedClient | null = null, laneClient: DashboardClient | null = null;
let template: DashboardTemplate | null = null;
let shownPage: string | null = null;
let mountedAt = 0;

const pageId = (label: string): PageId => {
  const page = PAGES.find(item => item.label === label);
  if (!page) throw new Error(`no page ${label}`);
  return page.id;
};

async function settle(least = 2, timeout = 8_000): Promise<boolean> {
  const until = performance.now() + timeout;
  for (let frames = 0, quiet = 0; performance.now() < until; frames++) {
    await frame();
    quiet = busyCount() === 0 ? quiet + 1 : 0;
    if (quiet >= 2 && frames >= least) return true;
  }
  return false;
}

function receivedAround(mark: RecordedMark, to: RecordedMark): { hour: number | null; bytes: Uint8Array }[] {
  const marks = recorded!.entries.filter((entry): entry is RecordedMark => entry.t === 'mark');
  const before = marks.filter(item => item.at < mark.at).at(-1) ?? { ...mark, at: -Infinity };
  return recorded!.messages(before, to);
}

function nextMark(mark: RecordedMark): RecordedMark {
  const marks = recorded!.entries.filter((entry): entry is RecordedMark => entry.t === 'mark');
  return marks[marks.indexOf(mark) + 1] ?? mark;
}

const scroller = () => host!.querySelector<HTMLElement>('.dash-grid-scroll')!;

function widgetStates() {
  return [...host!.querySelectorAll<HTMLElement>('article.dash-widget')].map(article => {
    const state = article.querySelector<HTMLElement>('.dash-widget-content .widget-state');
    return { id: article.dataset.widget!, kind: [...article.classList].find(name => name.startsWith('kind-'))!.slice(5), title: article.getAttribute('aria-label'),
      state: state?.dataset.state ?? null, text: state?.textContent?.trim() ?? null, detail: state?.dataset.detail ?? null, shown: shown.get(article.dataset.widget!) ?? null,
      notices: [...article.querySelectorAll<HTMLElement>('.held-failure, .dash-inline-warning')].map(item => item.textContent?.trim() ?? '') };
  });
}

const summarise = (items: readonly RecordedAsk[]) => items.map(ask => `${ask.product} ${JSON.stringify(ask.params).slice(0, 240)}`);

export const fixture = {
  async load() {
    recorded ??= await loadRecorded();
    client ??= recorded.client();
    laneClient ??= preparingClient(client);
    client.hourOf = () => clock?.getHour() ?? null;
    return { file: recorded.file, app: recorded.header.app.revision, entries: recorded.entries.length,
      marks: recorded.entries.filter(entry => entry.t === 'mark').map(mark => `${(mark as RecordedMark).label} ${(mark as RecordedMark).page ?? ''}`) };
  },

  async mount(page: string, label: string, speed?: number) {
    await fixture.load();
    const mark = recorded!.mark(label, page, item => speed === undefined || (item.speed === speed && !item.trace));
    if (mark.head === null) throw new Error(`the ${label} mark of ${page} holds no hour`);
    mountedAt = client!.asked.length;
    const received = receivedAround(mark, label === 'play' ? mark : nextMark(mark));
    client!.window = received.map(item => item.bytes);
    client!.at = mark.at;
    client!.windowEnd = nextMark(mark).at;
    if (shownPage === page && label === 'page') {
      clock!.seek(mark.head);
      for (const item of received) if (item.hour === mark.head) client!.deliver(item.bytes);
      await settle(8);
      return { hour: mark.head, widgets: template!.widgets.length };
    }
    root?.unmount(); host?.remove(); clock?.stop();
    shown.clear();
    const layout = mark.layout?.grid ? mark.layout : recorded!.marks('page', page)[0]?.layout;
    if (!layout?.scroll || !layout.grid) throw new Error(`the ${label} mark of ${page} holds no layout`);
    const source = recorded!.source;
    shownPage = page;
    template = defaultTemplate(pageId(page));
    const tracks = bindTracks(template, new Map([['primary', source]]));
    clock = new PlaybackController();
    clock.setBounds(clockBounds(tracks));
    clock.seek(mark.head);
    const served = client!;
    const maps = new MapViewsStore(served, clock);
    const runtime: WidgetRuntime = { campaignTracks: template.tracks, defaultRoleId: 'primary', tracks, clock, maps, onMapBranch() {}, client: served, alignment: template.alignment,
      nativeHidden: false, campaigns: [{ id: 'primary', path: source.path, label: 'Main campaign' }], recordings: [], assignCampaign: async () => {}, requestSeek: () => false,
      addWidgetBelow() {} };
    host = document.body.appendChild(document.createElement('div'));
    host.style.cssText = `position: fixed; left: ${layout.scroll.x}px; top: ${layout.scroll.y}px;`;
    const shell = host.appendChild(document.createElement('div'));
    shell.className = 'dash-grid-scroll';
    shell.style.cssText = `width: ${layout.scroll.width}px; height: ${layout.scroll.height}px; overflow: auto;`;
    const kinds = new Set(PAGES.flatMap(item => defaultTemplate(item.id).widgets.map(widget => widget.kind)));
    await Promise.all([...kinds].map(async kind => { if (!await loadWidgetRenderer(kind)) throw new Error(`no renderer for ${kind}`); }));
    const ids: PageId[] = PAGES.map(item => item.id), at = ids.indexOf(pageId(page));
    const prepared: PreparedPage[] = [...ids.slice(at), ...ids.slice(0, at)].map(id => {
      const paged = defaultTemplate(id);
      return { id, template: paged, runtime: { ...runtime, client: laneClient!, tracks: bindTracks(paged, new Map([['primary', source]])), campaignTracks: paged.tracks,
        alignment: paged.alignment } };
    });
    const drawPrepared = (item: PreparedPage) =>
      <DashboardGrid prepared template={item.template} runtime={item.runtime} editing={false} onLayouts={noop} onApply={noop} onRemove={noop} />;
    root = createRoot(shell);
    const noop = () => {};
    const shownTemplate = template;
    root.render(<>
      <div style={{ width: layout.grid.width }}>
        <LiveMotion.Provider value={clock.getMotion}><DashboardGrid template={shownTemplate} runtime={runtime} editing={false} onLayouts={noop} onApply={noop} onRemove={noop} /></LiveMotion.Provider>
      </div>
      <PreparedPages pages={prepared} clock={clock} client={client!} viewport={shell} enabled draw={drawPrepared} mounting={bodiesWaiting} />
    </>);
    await settle(8);
    return { hour: mark.head, widgets: template.widgets.length };
  },

  async scroll() {
    const view = scroller();
    if (view.scrollTop + view.clientHeight >= view.scrollHeight - 1) return false;
    view.scrollTop += Math.round(view.clientHeight * 0.9);
    await settle(4);
    return true;
  },

  async glanced(page: string) {
    const mark = recorded!.mark('page', page);
    await settle(4);
    const states = widgetStates();
    scroller().scrollTop = 0;
    await settle(2);
    const asked = client!.asked.slice(mountedAt), shownAsks = asked.filter(ask => !ask.lane);
    return { hour: mark.head, states, asked: shownAsks.length, misses: summarise(shownAsks.filter(ask => !ask.served)),
      lane: { asked: asked.length - shownAsks.length, misses: asked.filter(ask => ask.lane && !ask.served).length } };
  },

  async play(page: string, until = Infinity) {
    const start = recorded!.mark('play', page, mark => mark.speed === 1440 && !mark.trace);
    const end = recorded!.mark('pause', page, mark => mark.speed === 1440 && !mark.trace && mark.at > start.at);
    const messages = recorded!.messages(start, end).filter(item => item.hour !== null);
    const before = client!.asked.length;
    commits.clear(); why.clear(); paints.clear(); asked.layout = 0; asked.measure = 0;
    counting = true;
    const began = performance.now();
    client!.at = start.at;
    client!.windowEnd = end.at;
    clock!.setSpeed(1440);
    clock!.setPlaying(true);
    let next = 0, frames = 0;
    while (next < messages.length && clock!.getHour() < until && performance.now() - began < 25_000) {
      await frame();
      frames++;
      const hour = clock!.getHour();
      while (next < messages.length && messages[next]!.hour! <= hour) {
        const message = messages[next++]!;
        client!.at = message.at;
        client!.deliver(message.bytes);
      }
      if (!clock!.getSnapshot().playing) break;
    }
    const elapsed = performance.now() - began;
    clock!.setPlaying(false);
    counting = false;
    if (Number.isFinite(until)) {
      clock!.seek(until);
      for (const item of messages) if (item.hour === until) client!.deliver(item.bytes);
    }
    const head = clock!.getHour();
    await settle(Number.isFinite(until) ? 30 : 4, 15_000);
    const during = client!.asked.slice(before).filter(ask => !ask.lane && ask.at - began <= elapsed);
    return { from: start.head, head, frames, elapsed, messages: messages.length, delivered: next, lastHour: messages.at(-1)?.hour ?? null,
      commits: Object.fromEntries(commits), why: Object.fromEntries([...why].map(([id, causes]) => [id, Object.fromEntries(causes)])),
      asked: during.map(ask => ({ product: ask.product, params: ask.params, from: ask.asksFrom ?? paramsFrom(ask.params), head: ask.head, ms: ask.at - began, served: ask.served })),
      misses: summarise(during.filter(ask => !ask.served)), layout: asked.layout, measure: asked.measure,
      paints: Object.fromEntries(paints), shown: Object.fromEntries(shown), states: widgetStates() };
  },

  async viewport() {
    await fixture.load();
    const layout = recorded!.entries.find((entry): entry is RecordedMark => entry.t === 'mark' && !!entry.layout)?.layout;
    if (!layout) throw new Error(`${recorded!.file} holds no layout`);
    return layout.viewport;
  },
  async start() {
    await fixture.load();
    const [start] = recorded!.marks('start');
    if (!start?.page) return null;
    return fixture.mount(start.page, 'start');
  },
  async pages() {
    await fixture.load();
    return recorded!.marks('page').map(mark => mark.page!);
  },
  async windowHours(page: string) {
    await fixture.load();
    const start = recorded!.mark('play', page, mark => mark.speed === 1440 && !mark.trace);
    const end = recorded!.mark('pause', page, mark => mark.speed === 1440 && !mark.trace && mark.at > start.at);
    return [...new Set(recorded!.messages(start, end).flatMap(item => item.hour === null ? [] : [item.hour]))];
  },
  states: () => widgetStates(),
  values: () => Object.fromEntries([...host!.querySelectorAll<HTMLElement>('article.dash-widget')].map(article => [article.dataset.widget!, {
    kind: [...article.classList].find(name => name.startsWith('kind-'))!.slice(5),
    text: (article.querySelector<HTMLElement>('.dash-widget-content')?.innerText ?? '').replace(/\s+/g, ' ').trim(),
    shown: shown.get(article.dataset.widget!) ?? null }])),
  sizes(page: string, label = 'page') {
    const layout = recorded!.mark(label, page).layout!;
    return [...host!.querySelectorAll<HTMLElement>('article.dash-widget')].flatMap(article => {
      const app = layout.widgets.find(item => item.id === article.dataset.widget);
      const box = rect.call(article);
      return app && Math.abs(app.width - box.width) <= 1 && Math.abs(app.height - box.height) <= 1 ? []
        : [{ id: article.dataset.widget!, app: app ? [app.width, app.height] : null, here: [box.width, box.height] }];
    });
  },
};
(window as unknown as { recordedPage: typeof fixture }).recordedPage = fixture;

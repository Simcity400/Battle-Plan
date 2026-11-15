/** A dashboard page as the app lays it out, its widgets served only the replies and followed messages the app received
 * on the 13-year recording (`recorded.ts`): the page's own grid at the recorded size and hour, so every request a widget
 * makes is one the app made, or a miss named. Paused at a mark of the run, or playing one of its 1,440 h/s windows. */
// Before react-dom: the hook it reports each commit to, for which widget committed and why.
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
// Every widget's renderer registered, as the dashboard does.
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

/** What a widget reported shown and painted, by widget; only while `counting`, layout read back and text measured. */
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

/** Commits each widget made while counting, and why. */
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
/** The page laid out now: shown again at a later mark, it is sought there, as the app's clock sought it, not laid out anew. */
let shownPage: string | null = null;
/** Where this mount's requests begin in the client's: one client lives through every mount, as the app's session did,
 * so what the app held from a page before is held here too. */
let mountedAt = 0;

const pageId = (label: string): PageId => {
  const page = PAGES.find(item => item.label === label);
  if (!page) throw new Error(`no page ${label}`);
  return page.id;
};

/** Until nothing is in flight for two frames running (at least `least` frames, at most `timeout` ms). */
async function settle(least = 2, timeout = 8_000): Promise<boolean> {
  const until = performance.now() + timeout;
  for (let frames = 0, quiet = 0; performance.now() < until; frames++) {
    await frame();
    quiet = busyCount() === 0 ? quiet + 1 : 0;
    if (quiet >= 2 && frames >= least) return true;
  }
  return false;
}

/** The followed messages the app received from the mark before `mark` up to `to`: what a follow made here is answered from. */
function receivedAround(mark: RecordedMark, to: RecordedMark): { hour: number | null; bytes: Uint8Array }[] {
  const marks = recorded!.entries.filter((entry): entry is RecordedMark => entry.t === 'mark');
  const before = marks.filter(item => item.at < mark.at).at(-1) ?? { ...mark, at: -Infinity };
  return recorded!.messages(before, to);
}

/** The mark after `mark` in the run, or `mark` itself when it is the last. */
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

  /** `page` paused at `mark`, laid out at the size the app had, its follows answered as the app's were then; already shown,
   * the clock seeks there. */
  async mount(page: string, label: string, speed?: number) {
    await fixture.load();
    const mark = recorded!.mark(label, page, item => speed === undefined || (item.speed === speed && !item.trace));
    if (mark.head === null) throw new Error(`the ${label} mark of ${page} holds no hour`);
    mountedAt = client!.asked.length;
    // Paused at a page's mark, the app went on receiving until its next step: follows made while scrolling were answered
    // then. A window played starts from what was received by its mark.
    const received = receivedAround(mark, label === 'play' ? mark : nextMark(mark));
    client!.window = received.map(item => item.bytes);
    if (shownPage === page && label === 'page') {
      clock!.seek(mark.head);
      // What the service pushed for the hour sought, to the follows already made.
      for (const item of received) if (item.hour === mark.head) client!.deliver(item.bytes);
      await settle(8);
      return { hour: mark.head, widgets: template!.widgets.length };
    }
    root?.unmount(); host?.remove(); clock?.stop();
    shown.clear();
    // A mark taken before the grid was laid out (the run's start) takes the page's layout from its next mark.
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
    // The page's code first, as the shipped bundle has it at hand: what is timed and counted is the widgets', not the
    // test server compiling them.
    const kinds = new Set(PAGES.flatMap(item => defaultTemplate(item.id).widgets.map(widget => widget.kind)));
    await Promise.all([...kinds].map(async kind => { if (!await loadWidgetRenderer(kind)) throw new Error(`no renderer for ${kind}`); }));
    // The prepare lane as the dashboard runs it: the page shown, then every other page from the next, laid out unseen and
    // asked through the lane, so a page comes to be shown holding what the app's lane had asked for it.
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

  /** As the sweep's glance did, a step a call: the page scrolled a screen further, each step settled; false at its bottom. */
  async scroll() {
    const view = scroller();
    if (view.scrollTop + view.clientHeight >= view.scrollHeight - 1) return false;
    view.scrollTop += Math.round(view.clientHeight * 0.9);
    await settle(4);
    return true;
  },

  /** Once scrolled to its bottom: the follows made as the app scrolled answered as they were before its next mark, and
   * what each widget shows. */
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

  /** `page` (mounted at its 1,440 h/s window's mark) playing that window: the clock running at that speed from the window's
   * hour, each recorded message handed over once the clock reaches its hour; what each widget committed, asked, painted
   * and showed, times in ms from play. With `until` (one of the window's message hours, `windowHours`), it stops there and
   * the clock seeks to exactly that hour, its settled answer handed over as the service pushes it: two replays to the same
   * hour show the same, frame timing aside. */
  async play(page: string, until = Infinity) {
    const start = recorded!.mark('play', page, mark => mark.speed === 1440 && !mark.trace);
    const end = recorded!.mark('pause', page, mark => mark.speed === 1440 && !mark.trace && mark.at > start.at);
    const messages = recorded!.messages(start, end).filter(item => item.hour !== null);
    const before = client!.asked.length;
    commits.clear(); why.clear(); paints.clear(); asked.layout = 0; asked.measure = 0;
    counting = true;
    const began = performance.now();
    clock!.setSpeed(1440);
    clock!.setPlaying(true);
    let next = 0, frames = 0;
    while (next < messages.length && clock!.getHour() < until && performance.now() - began < 25_000) {
      await frame();
      frames++;
      const hour = clock!.getHour();
      while (next < messages.length && messages[next]!.hour! <= hour) client!.deliver(messages[next++]!.bytes);
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

  /** The app's window as it recorded: the browser takes its size before a page is mounted. */
  async viewport() {
    await fixture.load();
    const layout = recorded!.entries.find((entry): entry is RecordedMark => entry.t === 'mark' && !!entry.layout)?.layout;
    if (!layout) throw new Error(`${recorded!.file} holds no layout`);
    return layout.viewport;
  },
  /** Where the run began: the page shown as the recording opened, at its opening hour; nothing when the run marked none. */
  async start() {
    await fixture.load();
    const [start] = recorded!.marks('start');
    if (!start?.page) return null;
    return fixture.mount(start.page, 'start');
  },
  /** The pages the run glanced at, in its order. */
  async pages() {
    await fixture.load();
    return recorded!.marks('page').map(mark => mark.page!);
  },
  /** The hours of `page`'s 1,440 h/s window that a message was pushed for, in order. */
  async windowHours(page: string) {
    await fixture.load();
    const start = recorded!.mark('play', page, mark => mark.speed === 1440 && !mark.trace);
    const end = recorded!.mark('pause', page, mark => mark.speed === 1440 && !mark.trace && mark.at > start.at);
    return [...new Set(recorded!.messages(start, end).flatMap(item => item.hour === null ? [] : [item.hour]))];
  },
  states: () => widgetStates(),
  /** What each widget shows now, by id: the text it draws (painted cells included) and the hour it reports showing. The
   * old code and the new, mounted from the same recording at the same mark or window hour, should give the same. */
  values: () => Object.fromEntries([...host!.querySelectorAll<HTMLElement>('article.dash-widget')].map(article => [article.dataset.widget!, {
    kind: [...article.classList].find(name => name.startsWith('kind-'))!.slice(5),
    text: (article.querySelector<HTMLElement>('.dash-widget-content')?.innerText ?? '').replace(/\s+/g, ' ').trim(),
    shown: shown.get(article.dataset.widget!) ?? null }])),
  /** Each widget whose size differs from the app's at `page`'s mark by more than a pixel: the app's, then this page's. */
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

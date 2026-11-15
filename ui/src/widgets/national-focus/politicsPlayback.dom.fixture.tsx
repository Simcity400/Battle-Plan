/** The Politics page's own widgets at 1,440 game hours a second in a real browser: National focus and Stability & War
 * support of one country, following a clock the test moves 15 hours a frame, served one recording in which a focus
 * completes every 35 days and stability is followed at the head; their commits, the requests they make (with the hour
 * each asks from against the head it was sent at) and the hours they report. */
// Before react-dom: a production-shaped hook it reports each commit to, for why each widget committed.
import { attributeCommits } from '../../shell/commitCount.ts';
import { Component, Profiler } from 'react';
import type { ReactNode } from 'react';
import { createRoot } from 'react-dom/client';
import type { Root } from 'react-dom/client';
// The app's own stylesheet: its tokens, box sizing and row motion, as the widgets are drawn in it.
import '../../styles.css';
import NationalFocusWidget from './NationalFocusWidget';
import StabilityWidget from '../stability/StabilityWidget';
import PoliticalPowerSpendingWidget from '../political-power-spending/PoliticalPowerSpendingWidget';
import { MeasuredWidget } from '../../dashboard/widgetPerf.ts';
import { setPageClock } from '../../dashboard/structurePace.ts';
import { measureWith } from '../../shell/perf.ts';
import type { PerfRecorder } from '../../shell/perf.ts';
import { message } from '../../dashboard/follow/messages.fixture.ts';
import { TILE_BINS } from '../../dashboard/tiles/tile.ts';
import type { WidgetProps } from '../../dashboard/definitions';
import type { FocusEntry, FocusReply, FocusSegment } from '../../generated/dashboard';

const native = window.requestAnimationFrame.bind(window);
const frame = () => new Promise<void>(resolve => native(() => resolve()));
async function settle(frames = 4) { for (let at = 0; at < frames; at++) await frame(); }

const shown = new Map<string, number | null>();
measureWith(new Proxy({ shown: (widget: string, hour: number | null) => { shown.set(widget, hour); } }, {
  get: (target, name) => (target as Record<string | symbol, unknown>)[name] ?? (() => {}),
}) as unknown as PerfRecorder);

class Failed extends Component<{ children: ReactNode }, { error: Error | null }> {
  state = { error: null as Error | null };
  static getDerivedStateFromError(error: Error) { return { error }; }
  render() { return this.state.error ? <pre className="fixture-error">{this.state.error.stack}</pre> : this.props.children; }
}

const HOURS = 40_000;
export const START = 9_000;
/** A focus completes every 35 days. */
export const FOCUS_HOURS = 840;
const track = { id: 'main', axisOffset: 0, selectedTag: 'GER', source: { id: 'source', revision: '1', metadata: { countries: [] } },
  branch: { id: '1', firstHour: 0, lastHour: HOURS - 1, gaps: [] } };
const identities = [{ firstHour: 0, lastHour: HOURS - 1, slot: 1, countryId: 1 }];
const GROUPS = { branchId: '1', humans: [], factions: [], wars: [], warStarts: [], identityChanges: [], warnings: [], capitulations: [],
  countryIdentities: [{ tag: 'GER', identities }], unobserved: { humans: [], factions: [], wars: [] } };

const token = (at: number) => `GER_focus_${at}`;
const segments: FocusSegment[] = [];
const completions: FocusEntry[] = [];
for (let at = 0; at * FOCUS_HOURS < HOURS; at++) {
  segments.push({ firstHour: at * FOCUS_HOURS, lastHour: Math.min(HOURS - 1, (at + 1) * FOCUS_HOURS - 1),
    order: { completed: completions.slice(), current: token(at), continuous: null, paused: false } });
  completions.push({ ordinal: at, token: token(at), completedByHour: (at + 1) * FOCUS_HOURS });
}
/** Progress toward the current focus, read once a day. */
const progress = Array.from({ length: HOURS / 24 }, (_, day) => ({ firstHour: day * 24, lastHour: day * 24 + 23,
  progressRaw: String(((day * 24) % FOCUS_HOURS) / 24 * 100_000) }));
const artwork = (name: string) => ({ token: name, name: null, description: null, baseDurationDays: 35, icon: null });
/** Half the focuses carry a country-independent title; the rest are titled by a dated read. */
const titled = (at: number) => at % 2 === 0;
export const focusReply: FocusReply = { history: { segments, progress }, missing: [], warnings: [],
  artwork: segments.map((_, at) => artwork(token(at))),
  staticText: segments.map((_, at) => ({ ...artwork(token(at)), baseDurationDays: null, name: titled(at) ? `Focus ${at}` : null })), continuousStaticText: [] };
const focusCompletions = { completions, warnings: [] };

/** Political power spent: a one-off payment every ten days (every other one a decision naming Poland), an upkeep
 * charged every day, and the day's gain. */
const PAYMENT_HOURS = 240;
const spendItem = (id: string, category: string, recurring: boolean, targetTag: string | null = null) => ({ id, groupId: id, cause: id, category, recurring,
  firstHour: 0, firstRead: 0, name: id, action: '', icon: null, character: null, targetTag, targetName: targetTag ? 'Poland' : null, unknown: null, tooltipToken: null });
function spending(params: { tag: string; currency: string; from: number; to: number }) {
  const { from, to } = params;
  const payments = [], days = [], gains = [];
  for (let hour = Math.ceil((from - 12) / PAYMENT_HOURS) * PAYMENT_HOURS + 12; hour <= to; hour += PAYMENT_HOURS) {
    const at = (hour - 12) / PAYMENT_HOURS;
    payments.push({ item: at % 2, hour, read: hour, sequence: String(at), ordinal: 0, knownRaw: '5000000', payments: 1, unknown: 0, uncovered: false });
  }
  for (let day = Math.ceil(from / 24) * 24; day <= to; day += 24) {
    days.push({ item: 2, day, read: day, days: 1, chargeRaw: '10000', payments: 1, unknown: 0, uncovered: 0, realizedRaw: '10000', credits: 0 });
    gains.push({ read: day, gainedRaw: '200000', cappedRaw: '0', days: 1, unknownDays: 0, unmatched: 0 });
  }
  return { branchId: '1', tag: params.tag, currency: params.currency, from, to, decimals: 5, warnings: [], icons: [], ideaTooltips: null,
    items: [spendItem('advisor', 'advisors', false), spendItem('decision', 'decisions', false, 'POL'), spendItem('upkeep', 'upkeep', true)],
    categories: [{ id: 'advisors', name: 'Advisors', identified: true }, { id: 'decisions', name: 'Decisions', identified: true }, { id: 'upkeep', name: 'Upkeep', identified: true }],
    payments, days, gains, coverage: [{ firstHour: from, lastHour: to, covered: true, reason: null }] };
}

let clock: Clock;
class Clock {
  position = START;
  playing = false;
  hoursPerSecond = 1440;
  private readonly frames = new Set<() => void>();
  private readonly anchors = new Set<() => void>();
  getSnapshot = () => ({ position: this.position, hour: Math.floor(this.position), fraction: this.position % 1, playing: this.playing,
    hoursPerSecond: this.hoursPerSecond, sequence: 0, seekSequence: 0, bounds: { firstHour: 0, lastHour: HOURS - 1 } });
  getHour = () => Math.floor(this.position);
  getMotion = () => this.playing ? this.hoursPerSecond : undefined;
  positionAt = () => this.position;
  subscribe = (listener: () => void) => { this.anchors.add(listener); return () => { this.anchors.delete(listener); }; };
  subscribeFrame = (listener: () => void) => { this.frames.add(listener); return () => { this.frames.delete(listener); }; };
  subscribeHour = () => () => {};
  step(hours: number) { this.position += hours; for (const listener of [...this.frames]) listener(); }
  anchor() { for (const listener of [...this.anchors]) listener(); }
}

/** Stability and war support of GER at `hour`, fixed5 per hundred. */
const stabilityAt = (hour: number, measure: string) => (measure === 'stability' ? 50_000 : 30_000) + (Math.floor(hour / 24) % 40) * 500;

function answer(product: string, params: Record<string, unknown>): unknown {
  if (product === 'follow') {
    const query = (params as { query: { product?: string } }).query;
    return query.product === 'series-state' ? { available: true } : { available: false, reason: 'not followed here' };
  }
  if (product === 'unfollow' || product === 'follow-clock') return {};
  if (product === 'country-groups') return GROUPS;
  if (product === 'national-focus') return focusReply;
  if (product === 'power-spending') return spending(params as never);
  if (product === 'focus-completions') return focusCompletions;
  if (product === 'focus-artwork') {
    const { hour, tag, database, tokens } = params as { hour: number; tag: string; database: string; tokens: string[] };
    return { branchId: '1', hour, tag, database, warnings: [], items: tokens.map(name => ({ reason: null,
      artwork: { ...artwork(name), name: `Dated ${name}`, description: params.includeDescriptions ? 'About it' : null } })) };
  }
  if (product === 'country-facts') return { branchId: '1', known: [[0, HOURS - 1]], names: [], reasons: [], flags: [], countries: [], warnings: [] };
  if (product === 'country-styles') return { branchId: '1', hour: params.hour, styles: [], warnings: [] };
  if (product === 'series-tiles') {
    const { binHours, index, tags } = params as { binHours: number; index: number; tags: string[] };
    const first = index * TILE_BINS * binHours, last = first + TILE_BINS * binHours - 1;
    return { branchId: '1', measure: 'stability', decimals: 5, binHours, index, firstHour: first, immutable: true, filled: last, rawValues: true, hourBytes: 1,
      lines: tags.map(tag => ({ tag, missing: [{ firstHour: first, lastHour: last, reason: 'not recorded' }], parts: [] })), bins: btoa('\0'.repeat(TILE_BINS * tags.length)) };
  }
  throw new Error(`${product} is not served here`);
}

/** The first hour a request asks from, as the perf log names it. */
function askedFrom(request: { asksFrom?: number; params: Record<string, unknown> }): number | null {
  if (typeof request.asksFrom === 'number') return request.asksFrom;
  for (const value of [request.params.from, request.params.firstHour, request.params.hour, request.params.at]) if (typeof value === 'number') return value;
  return null;
}

const FOLLOW = new Set(['follow', 'unfollow', 'follow-clock']);
/** Chart tiles are judged by their own bar (one per chart per 256 bins of travel), not as values at the head. */
const JUDGED_APART = new Set([...FOLLOW, 'series-tiles']);

interface Asked { product: string; from: number | null; head: number; playing: boolean; params: Record<string, unknown> }
const requests: Asked[] = [];
const renders: Record<string, number> = {};
const followed = new Map<number, string>();
let receive: ((bytes: ArrayBuffer | Uint8Array) => void) | null = null;
let root: Root | null = null;
let draw: (endLabels: boolean) => void = () => {};
let host: HTMLDivElement | null = null;
const makeClient = () => ({
  available: true,
  product: async (request: { sourceId: string; revision: string; product: string; params: Record<string, unknown>; asksFrom?: number }) => {
    requests.push({ product: request.product, from: askedFrom(request), head: clock.getHour(), playing: clock.playing, params: request.params });
    if (request.product === 'follow') {
      const { id, query } = request.params as { id: number; query: { params?: { family?: { measure?: string } } } };
      followed.set(id, query.params?.family?.measure ?? '');
    }
    if (request.product === 'unfollow') followed.delete((request.params as { id: number }).id);
    return { sourceId: request.sourceId, revision: request.revision, product: request.product, data: answer(request.product, request.params) };
  },
  onFollowMessages: async (handler: (bytes: ArrayBuffer | Uint8Array) => void) => { receive = handler; return () => { if (receive === handler) receive = null; }; },
  setMapViews: async () => {}, onMapStates: async () => () => {}, releaseSource: async () => {}, openSource: async () => { throw new Error('none'); },
});

/** The service's followed answers at `hour`: GER's stability and war support. */
function push(hour: number) {
  const answers = [...followed].map(([id, measure]) => ({ id, answer: { keys: [1n], columns: 1, values: [BigInt(stabilityAt(hour, measure))] } }));
  if (answers.length) receive?.(message(hour, answers));
}

/** Why each widget committed while playing: its components that began the work, and what changed in them. */
const why = new Map<string, Map<string, number>>();
let counting = false;
attributeCommits((_zones, causes) => {
  if (!counting) return;
  for (const [zone, named] of causes) {
    const id = zone.replace('widget:', '');
    let held = why.get(id);
    if (!held) why.set(id, held = new Map());
    for (const [cause, count] of named) held.set(cause, (held.get(cause) ?? 0) + count);
  }
});

/** Each widget as the grid draws it: measured. */
const counted = (id: string, children: ReactNode) => {
  return <MeasuredWidget value={id}>
    <Profiler id={id} onRender={() => { renders[id] = (renders[id] ?? 0) + 1; }}><Failed>{children}</Failed></Profiler>
  </MeasuredWidget>;
};

export const fixture = {
  async mount() {
    root?.unmount(); host?.remove();
    clock = new Clock(); requests.length = 0; shown.clear(); followed.clear();
    // As the dashboard's grid does: the page's data updates and followed structure wait for its quarter-second slot.
    setPageClock(clock);
    for (const key of Object.keys(renders)) delete renders[key];
    host = document.body.appendChild(document.createElement('div'));
    host.style.cssText = 'width: 1200px; display: flex; gap: 8px; align-items: stretch;';
    root = createRoot(host);
    const runtime = { tracks: [track], clock, client: makeClient(), addWidgetBelow() {}, campaigns: [], recordings: [], requestSeek() {} };
    const props = (spec: unknown) => ({ widget: spec, runtime, active: true, onDiagnostics() {} }) as unknown as WidgetProps;
    const focus = { id: 'focus', kind: 'national-focus', trackIds: ['main'], countries: { kind: 'tags', tags: ['GER'] }, time: { mode: 'follow' }, options: {} };
    const spendingSpec = { id: 'spending', kind: 'political-power-spending', trackIds: ['main'], countries: { kind: 'tags', tags: ['GER'] }, time: { mode: 'follow' }, options: {} };
    draw = (endLabels: boolean) => {
      const stability = { id: 'stability', kind: 'stability', trackIds: ['main'], countries: { kind: 'tags', tags: ['GER'] }, time: { mode: 'follow' },
        options: { markWars: false, markFactions: false, markGovernment: false, markFocus: true, endLabels } };
      root!.render(<>
        <div id="focus" style={{ width: '380px', height: '720px', display: 'flex', flexDirection: 'column' }}>{counted('focus', <NationalFocusWidget {...props(focus)} />)}</div>
        <div style={{ display: 'flex', flexDirection: 'column', gap: '8px' }}>
          <div id="stability" style={{ width: '780px', height: '320px', display: 'flex', flexDirection: 'column' }}>{counted('stability', <StabilityWidget {...props(stability)} />)}</div>
          <div id="spending" style={{ width: '780px', height: '380px', display: 'flex', flexDirection: 'column' }}>{counted('spending', <PoliticalPowerSpendingWidget {...props(spendingSpec)} />)}</div>
        </div>
      </>);
    };
    draw(true);
    await settle(30);
    push(clock.getHour());
    // As the suite does, playback starts once nothing is in flight.
    await fixture.quiet();
  },

  /** Waits until no request has been made for 20 frames. */
  async quiet() {
    for (let still = 0, seen = requests.length, frames = 0; still < 20 && frames < 2000; frames++) {
      await frame();
      if (requests.length === seen) still++; else { still = 0; seen = requests.length; }
    }
  },
  /** Plays `frames` frames of `hours` each; at frame `restyleAt` a setting changes (the values at line ends), so
   * Stability renders while playing, as any interaction renders it. */
  async play(frames: number, hours: number, restyleAt = -1) {
    clock.playing = true; clock.anchor();
    await settle(30);
    const before = { requests: requests.length, renders: { ...renders } };
    why.clear(); counting = true;
    for (let at = 0; at < frames; at++) {
      clock.step(hours);
      if (at === restyleAt) draw(false);
      // The service writes the followed answers every frame while playing.
      push(clock.getHour());
      await frame();
    }
    counting = false;
    const head = clock.getHour();
    const rendered = Object.fromEntries(Object.entries(renders).map(([id, count]) => [id, count - (before.renders[id] ?? 0)]));
    const asked = requests.slice(before.requests).filter(request => !JUDGED_APART.has(request.product));
    const causes = Object.fromEntries([...why].map(([id, named]) => [id, Object.fromEntries([...named].sort((a, b) => b[1] - a[1]))]));
    return { head, rendered, causes, atHead: asked.filter(request => request.from === null || request.from <= request.head).map(describe),
      ahead: asked.filter(request => request.from !== null && request.from > request.head).map(describe) };
  },
  pause() { clock.playing = false; clock.anchor(); },
  /** Requests made since mount, described. */
  asked: () => requests.filter(request => !FOLLOW.has(request.product)).map(describe),
  shown: () => Object.fromEntries(shown),
  /** The focuses whose dated titles were read (text-only reads), with whether each has a static title. */
  titleReads: () => [...new Set(requests.filter(request => request.product === 'focus-artwork' && request.params.textOnly === true && !request.params.iconsOnly)
    .flatMap(request => request.params.tokens as string[]))].map(name => ({ name, static: titled(Number(name.slice(token(0).length - 1))) })),
  errors: () => [...document.querySelectorAll('.fixture-error')].map(element => element.textContent),
  /** The spending table's rows shown (not held ahead), top first, and its painted total spent. */
  spending: () => ({ rows: [...document.querySelectorAll<HTMLElement>('#spending tbody tr')].filter(row => row.dataset.reveal !== 'ahead').length,
    spent: document.querySelector('#spending .politics-spending-summary b')?.textContent ?? null }),
  /** The current focus's name and the number of completed focuses the list holds. */
  focus: () => ({ current: document.querySelector('#focus .focus-now-text > strong')?.textContent ?? null,
    completed: document.querySelectorAll('#focus .focus-entry').length,
    newest: document.querySelector('#focus .focus-entry.is-newest .focus-label strong')?.textContent ?? null }),
  /** Each mounted completion row's height, and the list's gap and width. */
  rows: () => ({ heights: [...document.querySelectorAll<HTMLElement>('#focus .focus-entry')].map(row => row.getBoundingClientRect().height),
    gap: getComputedStyle(document.querySelector('#focus .focus-list')!).rowGap, scroll: document.querySelector<HTMLElement>('#focus .focus-scroll')?.clientWidth }),
  /** The segment the reply holds at `hour`, by its completed count. */
  completedAt: (hour: number) => segments.find(segment => segment.firstHour <= hour && hour <= segment.lastHour)?.order.completed.length ?? null,
};
function describe(request: Asked) {
  const params = request.params as { tokens?: string[]; tags?: string[] };
  return `${request.product} from ${request.from} head ${request.head}${params.tokens ? ` ${params.tokens.join(',')}` : ''}${params.tags ? ` ${params.tags.join(',')}` : ''}`;
}
(window as unknown as { politics: typeof fixture }).politics = fixture;

/** The scoreboard on the playback bars, in a real browser: its top three followed by rank and every column followed for
 * them, on a clock the test moves 15 hours a frame, fed one followed message a frame as the service writes them; its
 * renders, the requests it makes, the hour it reports shown and the cells' painted text. */
// Before react-dom: a production-shaped hook it reports each commit to, for why a commit happened.
import { attributeCommits } from '../../shell/commitCount.ts';
import { Profiler } from 'react';
import { createRoot } from 'react-dom/client';
import type { Root } from 'react-dom/client';
import ScoreboardWidget from './ScoreboardWidget';
import { CELL } from '../../dashboard/follow/message.ts';
import { message } from '../../dashboard/follow/messages.fixture.ts';
import type { Spec } from '../../dashboard/follow/messages.fixture.ts';
import { MeasuredWidget } from '../../dashboard/widgetPerf.ts';
import { measureWith } from '../../shell/perf.ts';
import type { PerfRecorder } from '../../shell/perf.ts';
import type { WidgetProps } from '../../dashboard/definitions';
import type { ScoreboardColumn, ScoreboardColumnsReply } from '../../generated/dashboard';
import { setPageClock } from '../../dashboard/structurePace.ts';

const native = window.requestAnimationFrame.bind(window);
const frame = () => new Promise<void>(resolve => native(() => resolve()));
async function settle(frames = 4) { for (let at = 0; at < frames; at++) await frame(); }

const shown: (number | null)[] = [];
measureWith(new Proxy({ shown: (_widget: string, hour: number | null) => { shown.push(hour); } }, {
  get: (target, name) => (target as Record<string | symbol, unknown>)[name] ?? (() => {}),
}) as unknown as PerfRecorder);

const HOURS = 120_000;
const track = { id: 'main', axisOffset: 0, source: { id: 'source', revision: '1', metadata: { countries: [] } }, branch: { id: '1', firstHour: 0, lastHour: HOURS - 1, gaps: [] } };
const TAGS = ['GER', 'SOV', 'ITA'];
/** As a long campaign ranks them: six countries whose values trade places, so the top three's members and order move. */
const POOL = ['GER', 'SOV', 'ITA', 'USA', 'ENG', 'JAP'];
let churn = false;
/** The countries ranked without churn, and the top the widget follows. */
let countries = TAGS, top = 3;
const why = new Map<string, number>();
attributeCommits((_zones, causes) => { for (const named of causes.values()) for (const [cause, count] of named) why.set(cause, (why.get(cause) ?? 0) + count); });
const family = (kind: string, measure: string) => ({ kind, measure });
const COLUMNS: ScoreboardColumn[] = [
  { key: 'factories', label: 'Factories', unit: 'factories', decimals: 0, available: true, reason: null, family: family('country-factories', 'free7'), partial: null },
  { key: 'casualtiesDealt', label: 'Casualties dealt', unit: 'people', decimals: 0, available: true, reason: null, family: family('casualty-totals', 'dealt'),
    partial: family('casualty-totals', 'partial') },
];
const REGISTRY: ScoreboardColumnsReply = { branchId: '1', defaultColumns: ['factories', 'casualtiesDealt'], defaultSort: 'factories', columns: COLUMNS, warnings: [] };
/** A country's value of a measure at an hour: the order GER, SOV, ITA always, the values moving every hour. */
const value = (measure: string, at: number, hour: number) => BigInt(churn ? 1_000 + ((at * 7 + Math.floor(hour / 480)) % 6) * 100 + (hour % 97)
  : (countries.length - at) * 1_000 + (hour % 97)) * (measure === 'dealt' ? 10n : 1n);
/** The top at `hour`, greatest first. */
const ranked = (hour: number) => (churn ? POOL : countries).map((tag, at) => ({ tag, at, value: value('free7', at, hour) }))
  .sort((a, b) => (b.value > a.value ? 1 : b.value < a.value ? -1 : a.at - b.at)).slice(0, top);

function reply(product: string, params: Record<string, unknown>): unknown {
  if (product === 'follow') return { available: true };
  if (product === 'unfollow' || product === 'follow-clock') return {};
  if (product === 'scoreboard-columns') return REGISTRY;
  if (product === 'country-styles') return { branchId: '1', hour: params.hour, styles: [], warnings: [] };
  if (product === 'country-facts') return { branchId: '1', known: [[0, HOURS - 1]], names: [], reasons: [], flags: [], countries: [], warnings: [] };
  throw new Error(`${product} is not served here`);
}

let clock = null as unknown as Clock;
let receive: ((bytes: ArrayBuffer | Uint8Array) => void) | null = null;
const requests: { product: string; params: Record<string, unknown> }[] = [];
let renders = 0;
let root: Root | null = null;
let host: HTMLDivElement | null = null;

class Clock {
  position = 30_000;
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

const client = {
  available: true,
  product: async (request: { sourceId: string; revision: string; product: string; params: Record<string, unknown> }) => {
    requests.push({ product: request.product, params: request.params });
    return { sourceId: request.sourceId, revision: request.revision, product: request.product, data: reply(request.product, request.params) };
  },
  onFollowMessages: async (handler: (bytes: ArrayBuffer | Uint8Array) => void) => { receive = handler; return () => { if (receive === handler) receive = null; }; },
  setMapViews: async () => {}, onMapStates: async () => () => {}, releaseSource: async () => {}, openSource: async () => { throw new Error('none'); },
};

/** Each live follow by id: the ranked query ('rank') or the measure its reads follow, with the countries it reads. */
function followed(): Map<number, { measure: string; tags: string[] }> {
  const out = new Map<number, { measure: string; tags: string[] }>();
  for (const request of requests) {
    const id = request.params.id as number;
    if (request.product === 'unfollow') out.delete(id);
    if (request.product !== 'follow') continue;
    const params = (request.params.query as { params: { top?: number; family: { measure: string }; reads?: { tag: string }[] } }).params;
    out.set(id, { measure: params.top ? 'rank' : params.family.measure, tags: (params.reads ?? []).map(read => read.tag) });
  }
  return out;
}
const follows = () => new Map([...followed()].map(([id, follow]) => [id, follow.measure]));

/** One message at `hour`: the rank's three countries and each measure for them, as the service writes them. */
function deliver(hour: number) {
  const answers: { id: number; answer: Spec }[] = [];
  const pool = churn ? POOL : countries;
  for (const [id, { measure, tags }] of followed()) {
    const of = measure === 'rank' ? 'free7' : measure;
    const rows = measure === 'rank' ? ranked(hour).map(row => row.at) : churn ? tags.map(tag => pool.indexOf(tag)) : countries.map((_, at) => at);
    answers.push({ id, answer: { keys: rows.map(at => BigInt(at + 1)), columns: 1, names: rows.map(at => pool[at]),
      values: rows.map(at => of === 'partial' ? BigInt(at === 1 ? 1 : 0) : value(of, at, hour)), states: rows.map(() => CELL.value) } });
  }
  receive?.(message(hour, answers));
}

export const fixture = {
  /** `count` countries ranked, the widget following its top `count`. */
  async mount(churning = false, count = 3) {
    root?.unmount(); host?.remove();
    clock = new Clock(); requests.length = 0; shown.length = 0; renders = 0; churn = churning;
    countries = count > TAGS.length ? [...TAGS, ...Array.from({ length: count - TAGS.length }, (_, at) => `X${String(at).padStart(4, '0')}`)] : TAGS;
    top = count;
    // As the grid does: the page's clock owns when structure is taken while it plays.
    setPageClock(clock as never);
    host = document.body.appendChild(document.createElement('div'));
    host.style.cssText = 'width: 1200px; height: 360px; display: flex; flex-direction: column;';
    root = createRoot(host);
    const runtime = { tracks: [track], clock, client, addWidgetBelow() {}, campaigns: [], recordings: [] };
    const widget = { id: 'scoreboard', kind: 'scoreboard', trackIds: ['main'], countries: { kind: 'group', group: { type: 'top', count: top } }, time: { mode: 'follow' }, options: {} };
    root.render(<MeasuredWidget value="scoreboard"><Profiler id="scoreboard" onRender={() => { renders++; }}>
      <ScoreboardWidget {...({ widget, runtime, active: true, onDiagnostics() {} }) as unknown as WidgetProps} />
    </Profiler></MeasuredWidget>);
    await settle(12);
    // The service answers a new follow at the head: the rank first, then the columns it names the rows for.
    deliver(clock.getHour());
    await settle(12);
    deliver(clock.getHour());
    await settle(12);
  },
  async play(frames: number, hours: number) {
    clock.playing = true; clock.anchor();
    await settle(30);
    const before = { requests: requests.length, renders };
    why.clear();
    let members = 0, orders = 0, last = ranked(clock.getHour());
    for (let at = 0; at < frames; at++) {
      clock.step(hours);
      deliver(clock.getHour());
      const now = ranked(clock.getHour());
      if (now.map(row => row.tag).sort().join() !== last.map(row => row.tag).sort().join()) members++;
      else if (now.map(row => row.tag).join() !== last.map(row => row.tag).join()) orders++;
      last = now;
      await frame();
    }
    await settle();
    const asked = requests.slice(before.requests).filter(request => !['follow', 'unfollow', 'follow-clock'].includes(request.product)).map(request => request.product);
    return { head: clock.getHour(), asked, rendered: renders - before.renders, why: Object.fromEntries(why), members, orders };
  },
  pause() { clock.playing = false; clock.anchor(); },
  /** Waits `ms` of wall time: what the table shows once the structure of its mount has settled. */
  wait: (ms: number) => new Promise<void>(resolve => setTimeout(resolve, ms)),
  follows: () => [...follows().values()].sort(),
  /** The countries each column's follow reads, by measure. */
  followedTags: () => Object.fromEntries([...followed().values()].filter(follow => follow.measure !== 'rank').map(follow => [follow.measure, follow.tags.length])),
  shown: () => shown.at(-1) ?? null,
  /** Each row's country cell and painted cells, in the order shown. */
  cells: () => [...(host?.querySelectorAll('[role="row"]') ?? [])].map(row => [...row.querySelectorAll('[role="cell"], [role="gridcell"]')].map(cell => cell.textContent?.trim() ?? '')),
  text: () => host?.textContent ?? '',
};
(window as unknown as { scoreboard: typeof fixture }).scoreboard = fixture;

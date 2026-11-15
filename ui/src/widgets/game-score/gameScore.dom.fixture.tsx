/** Game score's table following the head in a real browser, as a long campaign ranks it: a top three of six countries
 * whose total scores trade places every 480 hours, the panel pushed four times a second at a new hour (its rows every
 * country, in tag order, as the service writes them), every score a live cell a frame at a time at 15 hours a frame;
 * its commits and why, what it asked and the hours it reported shown. */
// Before react-dom: a production-shaped hook it reports each commit to, for why a commit happened.
import { attributeCommits } from '../../shell/commitCount.ts';
import { Profiler } from 'react';
import { createRoot } from 'react-dom/client';
import type { Root } from 'react-dom/client';
import GameScoreWidget from './GameScoreWidget';
import { CELLS } from '../../dashboard/follow/liveCells';
import { message } from '../../dashboard/follow/messages.fixture.ts';
import { MeasuredWidget } from '../../dashboard/widgetPerf.ts';
import { measureWith } from '../../shell/perf.ts';
import type { PerfRecorder } from '../../shell/perf.ts';
import type { WidgetProps } from '../../dashboard/definitions';
import { SCORE_PARTS } from './gameScoreData.ts';
import '../../styles.css';
import { setPageClock } from '../../dashboard/structurePace.ts';

const native = window.requestAnimationFrame.bind(window);
const frame = () => new Promise<void>(resolve => native(() => resolve()));
async function settle(frames = 8) { for (let at = 0; at < frames; at++) await frame(); }

const shown: (number | null)[] = [];
measureWith(new Proxy({ shown: (_widget: string, hour: number | null) => { shown.push(hour); } }, {
  get: (target, name) => (target as Record<string | symbol, unknown>)[name] ?? (() => {}),
}) as unknown as PerfRecorder);

const HOURS = 120_000;
const track = { id: 'main', axisOffset: 0, source: { id: 'source', revision: '1', metadata: { countries: [] } }, branch: { id: '1', firstHour: 0, lastHour: HOURS - 1, gaps: [] } };
const TAGS = ['ENG', 'GER', 'ITA', 'JAP', 'SOV', 'USA'];
const SCORES = CELLS + 21;

class Clock {
  position = 30_000;
  playing = false;
  private readonly frames = new Set<() => void>();
  private readonly anchors = new Set<() => void>();
  getSnapshot = () => ({ position: this.position, hour: Math.floor(this.position), fraction: 0, playing: this.playing, hoursPerSecond: 1440, sequence: 0, seekSequence: 0,
    bounds: { firstHour: 0, lastHour: HOURS - 1 } });
  getHour = () => Math.floor(this.position);
  getMotion = () => this.playing ? 1440 : undefined;
  positionAt = () => this.position;
  subscribe = (listener: () => void) => { this.anchors.add(listener); return () => { this.anchors.delete(listener); }; };
  subscribeFrame = (listener: () => void) => { this.frames.add(listener); return () => { this.frames.delete(listener); }; };
  subscribeHour = () => () => {};
  step(hours: number) { this.position += hours; for (const listener of [...this.frames]) listener(); }
  anchor() { for (const listener of [...this.anchors]) listener(); }
}

let clock = new Clock();
let receive: ((bytes: ArrayBuffer | Uint8Array) => void) | null = null;
let panel: number | null = null;
const requests: string[] = [];
let commits = 0;
const why = new Map<string, number>();
attributeCommits((_zones, causes) => { for (const named of causes.values()) for (const [cause, count] of named) why.set(cause, (why.get(cause) ?? 0) + count); });
let root: Root | null = null;
let host: HTMLDivElement | null = null;

/** Country `at`'s score of part `part` at `hour`: the totals trade places every 480 hours, every score moves each hour. */
const score = (at: number, part: number, hour: number) => (1_000 + ((at * 7 + Math.floor(hour / 480)) % 6) * 100) * (part + 1) + hour % 97;
/** The top three by total at `hour`, greatest first. */
const top = (hour: number) => TAGS.map((tag, at) => ({ tag, total: score(at, 0, hour) })).sort((a, b) => b.total - a.total || (a.tag < b.tag ? -1 : 1)).slice(0, 3).map(row => row.tag);

function panelData() {
  const cell = { missing: null };
  return { rows: TAGS.map((tag, at) => ({ tag, countryId: at + 1, ...Object.fromEntries(SCORE_PARTS.map(part => [part, cell])) })), missing: null, warnings: [],
    multipliers: SCORE_PARTS.map(part => ({ part, raw: '100000', missing: null })), hints: {},
    live: { groups: [{ table: 80, columns: SCORE_PARTS.map((_, at) => at + 1), keys: TAGS.map((_, at) => String(at + 1)), follow: SCORES }], rows: TAGS,
      kinds: SCORE_PARTS.map(id => ({ id, op: 'value', scale: 1 })),
      cells: TAGS.flatMap((_, row) => SCORE_PARTS.flatMap((_, column) => [row, column, 1, 0, row, column])) } };
}

function reply(product: string, params: Record<string, unknown>): unknown {
  if (product === 'follow') {
    const query = params.query as { panel?: string };
    if (query.panel === 'game-score') panel = params.id as number;
    return { available: true };
  }
  if (product === 'unfollow' || product === 'follow-clock') return {};
  if (product === 'game-score') return panelData();
  if (product === 'country-styles') return { branchId: '1', hour: params.hour, styles: [], warnings: [] };
  if (product === 'country-facts') return { branchId: '1', known: [[0, HOURS - 1]], names: [], reasons: [], flags: [], countries: [], warnings: [] };
  if (product === 'country-names') return { branchId: '1', names: [], warnings: [] };
  if (product === 'country-flags') return { branchId: '1', flags: [], warnings: [] };
  throw new Error(`${product} is not served here`);
}

const client = {
  available: true,
  product: async (request: { sourceId: string; revision: string; product: string; params: Record<string, unknown> }) => {
    requests.push(request.product);
    return { sourceId: request.sourceId, revision: request.revision, product: request.product, data: reply(request.product, request.params) };
  },
  onFollowMessages: async (handler: (bytes: ArrayBuffer | Uint8Array) => void) => { receive = handler; return () => { if (receive === handler) receive = null; }; },
  setMapViews: async () => {}, onMapStates: async () => () => {}, releaseSource: async () => {}, openSource: async () => { throw new Error('none'); },
};

function push(hour: number) {
  receive?.(new TextEncoder().encode(`PNL1${JSON.stringify({ panels: [{ id: panel, hour, data: panelData(), refused: null }] })}`));
}

function values(hour: number) {
  receive?.(message(hour, [{ id: SCORES, answer: { keys: TAGS.map((_, at) => BigInt(at + 1)), columns: SCORE_PARTS.length,
    values: TAGS.flatMap((_, at) => SCORE_PARTS.map((_, part) => BigInt(score(at, part, hour)))) } }]));
}

export const fixture = {
  async mount() {
    root?.unmount(); host?.remove();
    clock = new Clock(); requests.length = 0; shown.length = 0; commits = 0; panel = null;
    // As the grid does: the page's clock owns when structure is taken while it plays.
    setPageClock(clock as never);
    host = document.body.appendChild(document.createElement('div'));
    host.style.cssText = 'width: 900px; height: 360px; display: flex; flex-direction: column;';
    root = createRoot(host);
    const runtime = { tracks: [track], clock, client, addWidgetBelow() {}, campaigns: [], recordings: [] };
    const widget = { id: 'score', kind: 'game-score', trackIds: ['main'], countries: { kind: 'group', group: { type: 'top', count: 3 } }, time: { mode: 'follow' }, options: {} };
    root.render(<MeasuredWidget value="score"><Profiler id="score" onRender={() => { commits++; }}>
      <GameScoreWidget {...({ widget, runtime, active: true, onDiagnostics() {}, setOption() {} } as unknown as WidgetProps)} />
    </Profiler></MeasuredWidget>);
    for (let at = 0; at < 120 && panel === null; at++) await frame();
    if (panel === null) throw new Error(`the panel was not followed; asked ${requests.join(', ')}`);
    push(clock.getHour());
    await settle(12);
    values(clock.getHour());
    await settle(30);
    return panel;
  },
  /** Plays `frames` frames of `hours` each: the scores every frame, the panel every `every` frames at the head's hour. */
  async play(frames: number, hours: number, every: number) {
    clock.playing = true; clock.anchor();
    await settle(30);
    const before = { requests: requests.length, commits };
    why.clear();
    let members = 0, orders = 0, last = top(clock.getHour());
    const started = performance.now();
    for (let at = 0; at < frames; at++) {
      clock.step(hours);
      if (at % every === 0) push(clock.getHour());
      values(clock.getHour());
      const now = top(clock.getHour());
      if ([...now].sort().join() !== [...last].sort().join()) members++;
      else if (now.join() !== last.join()) orders++;
      last = now;
      await frame();
    }
    await settle();
    return { head: clock.getHour(), slots: Math.ceil((performance.now() - started) / 250) + 1, members, orders, commits: commits - before.commits,
      why: Object.fromEntries(why), asked: requests.slice(before.requests).filter(product => !['follow', 'unfollow', 'follow-clock'].includes(product)) };
  },
  pause() { clock.playing = false; clock.anchor(); },
  /** The rows in the order shown (by their glide), each its cells' text. */
  rows: () => [...(host?.querySelectorAll<HTMLElement>('tbody tr:not([aria-hidden="true"])') ?? [])]
    .map(row => ({ y: new DOMMatrixReadOnly(getComputedStyle(row).transform).m42, cells: [...row.querySelectorAll<HTMLElement>('td')].map(cell => cell.textContent?.trim() ?? '') }))
    .sort((a, b) => a.y - b.y).map(row => row.cells),
  lastShown: () => shown.at(-1) ?? null,
  text: () => `${host?.textContent ?? ''} | asked ${requests.join(', ')}`,
};
(window as unknown as { gameScore: typeof fixture }).gameScore = fixture;

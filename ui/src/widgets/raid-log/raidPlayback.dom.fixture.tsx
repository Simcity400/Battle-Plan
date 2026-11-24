import { createRoot } from 'react-dom/client';
import type { Root } from 'react-dom/client';
import RaidLogWidget from './RaidLogWidget';
import { syntheticEpisode, syntheticRaidLog } from '../../dashboard/raids.fixtures.ts';
import { WidgetBody } from '../../dashboard/widgets.tsx';
import { registerWidgets } from '../../dashboard/definitions.ts';
import { definition as log } from './definition.ts';
import type { WidgetProps } from '../../dashboard/definitions';

const native = window.requestAnimationFrame.bind(window);
const frame = () => new Promise<void>(resolve => native(() => resolve()));
async function settle(frames = 4) { for (let at = 0; at < frames; at++) await frame(); }

registerWidgets([{ ...log, render: RaidLogWidget }]);

const HOURS = 120_000;
const START = 30_000;
const track = { id: 'main', axisOffset: 0, source: { id: 'source', revision: '1', metadata: { countries: [] } }, branch: { id: '1', firstHour: 0, lastHour: HOURS - 1, gaps: [] } };
const RAIDERS = ['GER', 'SOV', 'ITA'];
const RAIDS = syntheticRaidLog([
  ...RAIDERS.map((tag, at) => syntheticEpisode(`early${at}`, 1_000 + at, tag, 1_100 + at, 1_200 + at, true)),
  ...Array.from({ length: 60 }, (_, at) => syntheticEpisode(String(at), START + at * 150, RAIDERS[at % 3], START + at * 150 + 50, START + at * 150 + 100, at % 2 === 0)),
]);

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

function reply(product: string, params: Record<string, unknown>): unknown {
  if (product === 'raid-log') return RAIDS;
  if (product === 'place-names') return { warnings: [], places: [], states: ((params.states ?? []) as { stateId: number; hour: number }[]).map(state => ({ ...state, name: 'Kent', missing: null })) };
  if (product === 'country-styles') return { branchId: '1', hour: params.hour, styles: [], warnings: [] };
  if (product === 'country-facts') return { branchId: '1', known: [[0, HOURS - 1]], names: [], reasons: [], flags: [], countries: [], warnings: [] };
  if (product === 'country-groups') return { branchId: '1', humans: [], factions: [], wars: [], warStarts: [], identityChanges: [], warnings: [], countryIdentities: [],
    capitulations: [], unobserved: { humans: [], factions: [], wars: [] } };
  throw new Error(`${product} is not served here`);
}

let clock = new Clock();
const requests: { product: string; params: Record<string, unknown> }[] = [];
let root: Root | null = null;
let host: HTMLDivElement | null = null;

const client = {
  available: true,
  product: async (request: { sourceId: string; revision: string; product: string; params: Record<string, unknown> }) => {
    requests.push({ product: request.product, params: request.params });
    return { sourceId: request.sourceId, revision: request.revision, product: request.product, data: reply(request.product, request.params) };
  },
  setMapViews: async () => {}, onMapStates: async () => () => {}, releaseSource: async () => {}, openSource: async () => { throw new Error('none'); },
};

export const fixture = {
  async mount() {
    root?.unmount(); host?.remove();
    clock = new Clock(); requests.length = 0;
    host = document.body.appendChild(document.createElement('div'));
    host.style.cssText = 'width: 640px; display: flex; flex-direction: column;';
    root = createRoot(host);
    const runtime = { tracks: [track], clock, client, addWidgetBelow() {}, campaigns: [], recordings: [] };
    const log = { id: 'log', kind: 'raid-log', title: 'Raid log', configVersion: 1, trackIds: ['main'], countries: { kind: 'all' }, time: { mode: 'follow' }, options: {} };
    const props = (widget: unknown) => ({ widget, runtime, active: true, onDiagnostics() {}, setOption() {} }) as unknown as WidgetProps;
    root.render(
      <div style={{ height: '320px', display: 'flex' }}><WidgetBody {...props(log)} /></div>,
    );
    await settle(30);
  },
  async playing(playing: boolean) {
    clock.playing = playing;
    clock.anchor();
    await settle();
  },
  async advance() {
    const before = requests.length;
    clock.step(15);
    await settle();
    return requests.slice(before).map(request => request.product);
  },
  close() {
    root?.unmount(); root = null;
    host?.remove(); host = null;
  },
  states: () => [...host!.querySelectorAll('[role="status"]')].map(element => element.textContent),
  text: () => host?.textContent ?? '',
};
(window as unknown as { raidPlayback: typeof fixture }).raidPlayback = fixture;

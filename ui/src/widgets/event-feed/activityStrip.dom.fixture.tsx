import { createRoot } from 'react-dom/client';
import type { DashboardClient, ProductRequest } from '../../dashboard/client';
import type { FamilyViewProps, WidgetRuntime } from '../../dashboard/definitions';
import { createWidget, type BoundTrack, type CountrySelection } from '../../dashboard/model';
import { PlaybackController } from '../../dashboard/playback';
import { datedPanel } from '../../dashboard/follow/messages.fixture.ts';
import { packet, splitForLanes } from '../../dashboard/follow/split.ts';
import type { EngineFrame, FrameEngine } from '../../engine/frame';
import type { FamilyPlanContext } from '../../families/contract';
import type { CountryGroupReply } from '../../generated/dashboard';
import EventFeedActivityStrip from './EventFeedActivityStrip';
import { eventFeedDeclaration } from './eventFeedProjection';
import EventFeed from './index';
import '../../styles.css';

const FIRST = 100, LAST = 1300;
const identities = [
  { tag: 'AAA', identities: [{ slot: 7, countryId: 101, firstHour: FIRST, lastHour: 119 }] },
  { tag: 'BBB', identities: [{ slot: 7, countryId: 101, firstHour: 120, lastHour: LAST }] },
  { tag: 'CCC', identities: [{ slot: 8, countryId: 102, firstHour: FIRST, lastHour: LAST }] },
];
const members = [{ tag: 'AAA', firstHour: 110, lastHour: 119 }, { tag: 'BBB', firstHour: 120, lastHour: 132 }];
const groups = { branchId: '1', countryIdentities: identities, humans: members,
  factions: [{ id: '31', firstHour: FIRST, lastHour: LAST, name: 'Probe group', nameReason: null, recordedName: 'Probe group', names: [], members, leaders: [], incomplete: [] }],
  wars: members.map(run => ({ first: 'CCC', second: run.tag, firstHour: run.firstHour, lastHour: run.lastHour })),
  unobserved: { humans: [], factions: [], wars: [], unattributed: [] }, warnings: [], warStarts: [], capitulations: [], identityChanges: [] } as unknown as CountryGroupReply;
const frame = () => new Promise<void>(resolve => requestAnimationFrame(() => resolve()));

async function run(selection: CountrySelection, integrated: boolean, hours = 0, unknownMembership = false, hoursPerFrame = 1) {
  const asked: ProductRequest[] = [];
  let ports: readonly MessagePort[] = [];
  let follow: { id: number; route: number; query: Record<string, any>; previous?: string } | null = null;
  let now = 1000;
  performance.now = () => now;
  const tasks = new Set<{ phase: string; run: (frame: EngineFrame) => boolean | void }>();
  const engine = { every: (phase: string, run: (frame: EngineFrame) => boolean | void) => {
    const task = { phase, run };
    tasks.add(task); return { wake() {}, stop() { tasks.delete(task); } };
  } } as unknown as FrameEngine;
  const clock = new PlaybackController(engine);
  clock.setBounds({ firstHour: FIRST, lastHour: LAST }); clock.seek(FIRST); clock.setSpeed(1);
  const source = { id: `strip-${integrated}-${JSON.stringify(selection)}`, revision: '1', path: '',
    metadata: { campaignKey: 'strip', branches: [{ id: '1', firstHour: FIRST, lastHour: LAST, gaps: [] }], playerCountryId: '7' } };
  const track = { id: 'main', roleId: 'primary', label: 'Main', branchIndex: 0, anchorOffset: 0, source, branch: source.metadata.branches[0], axisOffset: 0, selectedSlot: 7 } as unknown as BoundTrack;
  const activity = (from: number, binHours: number, hour: number) => {
    const firstHour = Math.floor(from / binHours) * binHours;
    return { branchId: '1', firstHour, binHours,
      lines: [{ category: 'politics', before: '0', bins: hour > 143 ? [[Math.floor((hour - firstHour) / binHours), 1]] : [] }],
      missing: [{ firstHour: 110, lastHour: 119, reason: 'Holder unknown' }, { firstHour: 120, lastHour: 143, reason: 'Holder unknown' }], warnings: [] };
  };
  const push = () => {
    if (!follow || !ports.length) return;
    const hour = clock.getHour();
    const data = activity(FIRST, follow.query.binHours, hour);
    const value = JSON.stringify(data);
    const bytes = datedPanel(follow.id, hour, follow.previous === value ? undefined : data, clock.getSnapshot().seekSequence, follow.route);
    follow.previous = value;
    const split = splitForLanes(bytes, ports.length);
    split.lanes.forEach((part, lane) => {
      if (part) ports[lane]!.postMessage(packet('panels', part, undefined), [part.buffer]);
    });
  };
  const client = { available: true, openFollow: async (opened: readonly MessagePort[]) => { ports = opened; }, async product(request: ProductRequest) {
    asked.push(request);
    const params = request.params as Record<string, any>;
    let data: unknown;
    if (request.product === 'event-feed-activity') data = activity(params.from, params.binHours, params.hour);
    else if (request.product === 'follow-clock') data = { available: true };
    else if (request.product === 'follow' && params.query?.panel === 'event-feed-activity') {
      follow = { id: params.id, route: params.route, query: params.query };
      queueMicrotask(push);
      data = { available: true, route: params.route };
    }
    else if (request.product === 'unfollow') {
      if (follow?.id === params.id) follow = null;
      data = null;
    }
    else if (integrated && request.product === 'country-groups') data = unknownMembership ? { ...groups,
      unobserved: Object.fromEntries(['humans', 'factions', 'wars'].map(key => [key, [{ firstHour: 113, lastHour: 115 }]])) } : groups;
    else if (integrated && request.product === 'country-identities') data = { branchId: '1', known: [], countries: identities.map(entry => ({ ...entry, unresolved: [] })), unknown: [], warnings: [] };
    else if (request.product === 'event-feed-kinds') data = { kinds: [] };
    else if (request.product === 'follow') data = { available: false, reason: 'No followed rows in this fixture' };
    else if (request.product === 'event-feed-tail') data = { branchId: '1', rows: [], total: '0', offset: '0', live: '0', counts: [], missing: [], nextChange: null, unavailable: [], warnings: [] };
    else return new Promise(() => {});
    return { sourceId: request.sourceId, revision: request.revision, product: request.product, data };
  } } as unknown as DashboardClient;
  const runtime = { client, tracks: [track], campaignTracks: [track], clock, alignment: 'calendar', nativeHidden: false,
    requestSeek: () => false } as unknown as WidgetRuntime;
  const widget = { ...createWidget('event-feed', 'main', 'strip'), countries: selection,
    ...(integrated && !hours ? { time: { mode: 'range' as const, firstOffset: 0, lastOffset: 43 } } : {}) };
  const context = { track, widget, countries: { selectedSlots: [7, 'AAA'], all: false, unobserved: [] } } as unknown as FamilyPlanContext;
  const container = document.body.appendChild(document.createElement('div'));
  container.style.cssText = 'width:800px;height:600px';
  const root = createRoot(container);
  root.render(integrated
    ? <EventFeed.render {...{ widget, runtime, active: true, options: { politics: true, warsPeace: false, diplomacy: false, milestones: false }, manifestDeclaration: eventFeedDeclaration } as unknown as FamilyViewProps} />
    : <EventFeedActivityStrip {...{ widget, track, runtime, active: true, categories: ['politics'], from: 100, pinnedHour: 191, context }} />);
  for (let at = 0; at < 100; at++) {
    await frame();
    if (container.querySelector('.efs-strip') || integrated && container.querySelector('[data-state="refused"]')) break;
  }
  const before = asked.filter(request => request.product === 'event-feed-activity').length;
  clock.setPlaying(true);
  for (let played = 0; played < hours; played += hoursPerFrame) {
    now += 1000 * Math.min(hoursPerFrame, hours - played);
    for (const task of [...tasks]) if (task.phase === 'clock') task.run({ now } as EngineFrame);
    push();
    for (const task of [...tasks]) if (task.phase === 'paint') task.run({ now } as EngineFrame);
    await frame();
  }
  for (let at = 0; at < 100 && hours > 0; at++) {
    for (const task of [...tasks]) if (task.phase === 'paint') task.run({ now } as EngineFrame);
    await frame();
    const count = container.querySelectorAll('.efs-bars > span').length;
    if (count === Math.floor((clock.getHour() - 96) / 24) + 1) break;
  }
  const result = {
    hour: clock.getSnapshot().hour,
    before, count: asked.filter(request => request.product === 'event-feed-activity').length,
    activities: asked.filter(request => request.product === 'event-feed-activity').map(request => request.params),
    followed: asked.flatMap(request => {
      const query = (request.params as { query?: Record<string, unknown> } | undefined)?.query;
      return request.product === 'follow' && query?.panel === 'event-feed-activity' ? [query] : [];
    }),
    bins: container.querySelectorAll('.efs-bars > span').length,
    tails: asked.filter(request => request.product === 'event-feed-tail').map(request => request.params),
    groups: asked.filter(request => request.product === 'country-groups').length,
    strip: container.querySelector('.efs-strip')?.textContent,
    statuses: [...container.querySelectorAll('.efs-strip[role="status"], .widget-state-title')].map(node => node.textContent),
    partial: container.querySelectorAll('.efs-bar.is-partial').length,
    missing: container.querySelectorAll('.efs-gap[title="Not recorded"]').length,
  };
  clock.stop(); root.unmount(); container.remove();
  return result;
}

Object.assign(window, { activityStripFixture: { run } });

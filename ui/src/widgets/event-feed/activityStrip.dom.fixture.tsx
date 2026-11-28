import { createRoot } from 'react-dom/client';
import { flushSync } from 'react-dom';
import type { DashboardClient, ProductRequest } from '../../dashboard/client';
import type { FamilyViewProps, WidgetRuntime } from '../../dashboard/definitions';
import { createWidget, type BoundTrack, type CountrySelection } from '../../dashboard/model';
import { PlaybackController } from '../../dashboard/playback';
import { datedPanel } from '../../dashboard/follow/messages.fixture.ts';
import { packet, splitForLanes } from '../../dashboard/follow/split.ts';
import { installedModel, pipelineOf } from '../../dashboard/follow/useFollowed.ts';
import type { EngineFrame, FrameEngine } from '../../engine/frame';
import type { FamilyPlanContext } from '../../families/contract';
import type { CountryGroupReply } from '../../generated/dashboard';
import { measureWith, PerfRecorder } from '../../shell/perf.ts';
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
function whenShown(container: HTMLElement, ready: () => Element | null) {
  if (ready()) return Promise.resolve();
  return new Promise<void>(resolve => {
    const observer = new MutationObserver(() => {
      if (!ready()) return;
      observer.disconnect();
      resolve();
    });
    observer.observe(container, { childList: true, subtree: true, attributes: true });
  });
}

async function run(selection: CountrySelection, integrated: boolean, hours = 0, unknownMembership = false, hoursPerFrame = 1, audited = false) {
  const asked: ProductRequest[] = [];
  let ports: readonly MessagePort[] = [];
  let follow: { id: number; route: number; query: Record<string, any>; previous?: string } | null = null;
  let lastConfirmation: number | null = null;
  let now = 1000;
  const realNow = performance.now;
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
    const confirmed = follow.previous === value;
    if (confirmed) lastConfirmation = hour;
    const bytes = datedPanel(follow.id, hour, confirmed ? undefined : data, clock.getSnapshot().seekSequence, follow.route);
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
    if (request.product === 'event-feed-activity') {
      const exact = activity(params.from, params.binHours, params.hour);
      if (audited && params.hour === 118) exact.lines[0]!.bins = [[0, 1]];
      if (audited && params.hour === 119) exact.missing[0]!.lastHour = 118;
      data = exact;
    }
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
  const perf = audited ? new PerfRecorder() : null;
  if (perf) measureWith(perf);
  root.render(integrated
    ? <EventFeed.render {...{ widget, runtime, active: true, options: { politics: true, warsPeace: false, diplomacy: false, milestones: false }, manifestDeclaration: eventFeedDeclaration } as unknown as FamilyViewProps} />
    : <EventFeedActivityStrip {...{ widget, track, runtime, active: true, categories: ['politics'], from: 100, pinnedHour: hours ? null : 191, context }} />);
  const pipeline = pipelineOf(clock, client);
  try {
    await whenShown(container, () => container.querySelector(hours > 0 || !integrated ? '.efs-strip' : '.efs-strip, [data-state="refused"]'));
    await perf?.beginAudit();
    const before = asked.filter(request => request.product === 'event-feed-activity').length;
    const beforeBins = container.querySelectorAll('.efs-bars > span').length;
    if (hours > 0) {
      const placement = [...pipeline.page.widgets.keys()].find(id => installedModel(pipeline.view(id), follow?.id));
      if (placement === undefined) throw new Error('The followed activity strip has no installed panel');
      const target = FIRST + hours;
      let stop = () => {};
      const delivered = new Promise<void>(resolve => {
        stop = pipeline.onDate(placement, () => {
          if (pipeline.view(placement)?.through === target) resolve();
        });
      });
      const runPhase = (phase: string) => {
        for (const task of [...tasks]) if (task.phase === phase) task.run({ now } as EngineFrame);
      };
      let tick: number;
      const advance = () => {
        const hour = clock.getHour();
        const elapsed = audited && pipeline.view(placement)?.through !== hour ? 0 : Math.min(hoursPerFrame, target - hour);
        now += 1000 * elapsed;
        runPhase('clock');
        if (elapsed > 0) push();
        runPhase('paint');
        tick = requestAnimationFrame(advance);
      };
      clock.setPlaying(true);
      tick = requestAnimationFrame(advance);
      try {
        await delivered;
        flushSync(() => {});
      } finally {
        cancelAnimationFrame(tick);
        stop();
      }
    }
    const duringPlayback = asked.filter(request => request.product === 'event-feed-activity').length;
    const audits = await perf?.auditPictures();
    return {
      hour: clock.getSnapshot().hour, beforeBins, lastConfirmation,
      before, count: duringPlayback, audits,
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
  } finally {
    clock.stop(); root.unmount(); container.remove(); pipeline.close();
    if (perf) measureWith(null);
    performance.now = realNow;
  }
}

Object.assign(window, { activityStripFixture: { run } });

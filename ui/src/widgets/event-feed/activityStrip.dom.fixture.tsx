import { createRoot } from 'react-dom/client';
import type { DashboardClient, ProductRequest } from '../../dashboard/client';
import type { FamilyViewProps, WidgetRuntime } from '../../dashboard/definitions';
import { createWidget, type BoundTrack, type CountrySelection } from '../../dashboard/model';
import { PlaybackController } from '../../dashboard/playback';
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

async function run(selection: CountrySelection, integrated: boolean, hours = 0, unknownMembership = false) {
  const asked: ProductRequest[] = [];
  let now = 1000;
  performance.now = () => now;
  const tasks = new Set<(frame: EngineFrame) => boolean | void>();
  const engine = { every: (_phase: string, run: (frame: EngineFrame) => boolean | void) => {
    tasks.add(run); return { wake() {}, stop() { tasks.delete(run); } };
  } } as unknown as FrameEngine;
  const clock = new PlaybackController(engine);
  clock.setBounds({ firstHour: FIRST, lastHour: LAST }); clock.seek(FIRST); clock.setSpeed(1);
  const source = { id: `strip-${integrated}-${JSON.stringify(selection)}`, revision: '1', path: '',
    metadata: { campaignKey: 'strip', branches: [{ id: '1', firstHour: FIRST, lastHour: LAST, gaps: [] }], playerCountryId: '7' } };
  const track = { id: 'main', roleId: 'primary', label: 'Main', branchIndex: 0, anchorOffset: 0, source, branch: source.metadata.branches[0], axisOffset: 0, selectedSlot: 7 } as unknown as BoundTrack;
  const client = { available: true, async product(request: ProductRequest) {
    asked.push(request);
    const params = request.params as Record<string, any>;
    let data: unknown;
    if (request.product === 'event-feed-activity') data = { branchId: '1', firstHour: Math.floor(params.from / params.binHours) * params.binHours,
      binHours: params.binHours, lines: [{ category: 'politics', before: '0', bins: [] }],
      missing: [{ firstHour: 110, lastHour: 119, reason: 'Holder unknown' }, { firstHour: 120, lastHour: 143, reason: 'Holder unknown' }], warnings: [] };
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
  for (let hour = 0; hour < hours; hour++) {
    now += 1000;
    for (const run of [...tasks]) run({ now } as EngineFrame);
    await frame();
  }
  const result = {
    hour: clock.getSnapshot().hour,
    before, count: asked.filter(request => request.product === 'event-feed-activity').length,
    activities: asked.filter(request => request.product === 'event-feed-activity').map(request => request.params),
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

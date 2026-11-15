/** The widgets that read the hour at the head, mounted in a real browser on a playing clock: what renders and
 * what is asked of the service while the hour moves. */
import { Profiler } from 'react';
import type { ReactNode } from 'react';
import { createRoot } from 'react-dom/client';
import type { DashboardClient, DashboardSource, ProductRequest } from './client';
import type { BoundTrack, WidgetSpec } from './model';
import type { WidgetRuntime } from './definitions';
import { PlaybackController } from './playback';
import { PlaybackStrip } from './controls';
import CountryChip from './CountryChip';
import { CountriesSection } from './settings/CountriesSection';
import MapWidget from '../widgets/map/MapWidget';
import { useIdentityHour } from './atHead.ts';
import { askTimeline, identitySince } from './countryTimeline.ts';
import type { CountryTimeline } from './countryTimeline';
import { followsHead, panelStructure, useFallbackHour, useFollow } from './follow/useFollow.ts';
import { firstHourOfMonth, gameDate, monthNames } from './calendar.ts';
import { useLayoutEffect, useMemo, useState } from 'react';
import { followStore } from './follow/connection.ts';
import { panelReply } from './follow/panel.ts';
import DatedCountryChip from './DatedCountryChip';
import { useIdentityKey } from './atHead.ts';
import { useCountryLabelsAt } from './countryFlagsAt';
import { useFrameProduct } from '../widgets/army-size/frameProduct.ts';
import { pacedKey, useHeadHour } from '../widgets/army-size/headHour.ts';

const frame = () => new Promise<void>(resolve => requestAnimationFrame(() => resolve()));
async function settle() { for (let i = 0; i < 6; i++) await frame(); }

/** Hours at which the faction is renamed and Germany's identity changes. */
const RENAMED = 5000;
const LAST = 100_000;

const asked: string[] = [];
const source = { id: 'lease', path: 'campaign.battleplan', revision: 'r1', metadata: { campaignKey: 'c', playerCountryId: null,
  branches: [{ id: '1', firstHour: 0, lastHour: LAST }], countries: [{ id: '0', tag: 'GER', name: 'Germany', observedBranchId: '1', observedHour: 0 }] } } as unknown as DashboardSource;
const track = { id: 't', roleId: 'role', label: 'Campaign', selectedTag: 'GER', axisOffset: 0, branch: { id: '1', firstHour: 0, lastHour: LAST, gaps: [] }, source } as unknown as BoundTrack;
const unheld = { ...track, branch: { ...track.branch, id: '2' } } as BoundTrack;

const span = { firstHour: 0, lastHour: LAST };
const GROUPS = { branchId: '1', warnings: [], humans: [], wars: [], warStarts: [], capitulations: [], identityChanges: [], countryIdentities: [],
  unobserved: { humans: [], factions: [], wars: [] },
  factions: [{ id: '1', ...span, name: 'Axis Powers', nameReason: null, recordedName: 'Axis', members: [{ ...span, tag: 'GER' }], leaders: [{ ...span, tag: 'GER' }], incomplete: [],
    names: [{ firstHour: 0, lastHour: RENAMED - 1, name: 'Axis', reason: null }, { firstHour: RENAMED, lastHour: LAST, name: 'Axis Powers', reason: null }] }] };
const FACTS = { branchId: '1', known: [[0, LAST]], names: ['Germany', 'German Reich'], reasons: [], flags: [], warnings: [],
  countries: [{ tag: 'GER', runs: [[0, 0, 0x336699, -1, -1], [RENAMED, 1, 0x336699, -1, -1]] }] };

const rosterReply = (data: unknown) => data as { sections: string[]; warnings: string[] };
/** Whether the service shows panels (a playhead), the panel it follows last, and who hears pushed messages. */
const panels = { on: true, id: 0, receivers: new Set<(bytes: ArrayBuffer | Uint8Array) => void>() };

function follow(params: { id: number; query: { panel?: string } }) {
  if (!params.query.panel) return { available: false, reason: 'not followed here' };
  panels.id = params.id;
  return panels.on ? { available: true } : { available: false, reason: 'This recording has no playhead in the viewer' };
}

const client = {
  available: true, nativeMaps: true,
  product: async (request: ProductRequest) => {
    asked.push(request.product);
    const data = request.product === 'country-groups' ? GROUPS : request.product === 'country-facts' ? FACTS
      : request.product === 'template-roster' ? roster((request.params as { hour: number }).hour)
        : request.product === 'follow' ? follow(request.params as never) : { available: true };
    return { sourceId: request.sourceId, revision: request.revision, product: request.product, data };
  },
  onFollowMessages: async (receive: (bytes: ArrayBuffer | Uint8Array) => void) => { panels.receivers.add(receive); return () => panels.receivers.delete(receive); },
} as unknown as DashboardClient;

/** The roster as the service answers it: one set of templates through the rename, another after. */
function roster(hour: number) {
  const before = hour < RENAMED;
  return { sections: [before ? 'Infantry division' : 'Armoured division'], warnings: [] };
}

const clock = new PlaybackController();
clock.setBounds({ firstHour: 0, lastHour: LAST });
const maps = { register: () => () => {}, paint: () => () => {}, retry() {}, schedule() {}, subscribe: () => () => {}, getState: () => null,
  subscribeHidden: () => () => {}, isHidden: () => false };
const runtime = { client, clock, maps, tracks: [track], campaignTracks: [track], defaultRoleId: 'role' } as unknown as WidgetRuntime;
const faction: WidgetSpec = { id: 'chip', kind: 'political-power', configVersion: 1, title: 'Power', campaign: 'default', trackIds: ['t'], time: { mode: 'follow' },
  countries: { kind: 'group', group: { type: 'faction', id: '1' }, membership: 'hour', aggregate: false }, options: {} } as unknown as WidgetSpec;
const map = { ...faction, id: 'map', kind: 'map', countries: { kind: 'all' } } as unknown as WidgetSpec;

const renders: Record<string, number> = {};
const counted = (id: string, child: ReactNode) => <Profiler id={id} onRender={() => { renders[id] = (renders[id] ?? 0) + 1; }}>{child}</Profiler>;
const sameHour = (hour: number) => hour;

function Identity({ of }: { of: BoundTrack }) {
  const hour = useIdentityHour(client, of, ['GER'], clock, true, sameHour);
  return <span data-identity={of.branch.id}>{hour}</span>;
}

function Fallback({ on }: { on: boolean }) {
  return <span data-fallback>{useFallbackHour(clock, on)}</span>;
}

const host = document.body.appendChild(document.createElement('div'));
const root = createRoot(host);

const rosterPasses = { renders: 0, commits: 0 };

/** The army breakdown's roster as its live call site wires it: a text panel the viewer pushes, and where no playhead
 * shows it, asked from the frame at the paced hour. */
function Roster() {
  const [element, setElement] = useState<HTMLElement | null>(null);
  const root = useMemo(() => ({ current: element }), [element]);
  const store = useMemo(() => followStore(client, source, clock), []);
  const query = useMemo(() => ({ panel: 'template-roster', branchId: '1', tag: 'GER' }), []);
  const followed = useFollow(store, query, root, () => {}, panelStructure);
  const pushed = followsHead(followed);
  const paced = useMemo(() => pacedKey(clock), []);
  const asked = useFrameProduct(client, clock, !pushed, at => ({ key: `GER|${paced(Math.floor(at))}`, request: { sourceId: 'lease', revision: 'r1', product: 'template-roster',
    params: { branchId: '1', tag: 'GER', hour: Math.floor(at) } } }),
  rosterReply, source, (a, b) => JSON.stringify(a.sections) === JSON.stringify(b.sections));
  const fromPanel = useMemo(() => panelReply(followed.entry?.panel, rosterReply), [followed.entry, followed.structure]);
  const shown = pushed ? fromPanel : asked;
  rosterPasses.renders++;
  useLayoutEffect(() => { rosterPasses.commits++; });
  return <span ref={setElement} data-roster data-status={`${followed.status}:${followed.reason ?? ''}`} data-error={shown.error?.message}>{shown.data?.sections.join('; ') ?? ''}</span>;
}

/** The pinned breakdown's header as `BreakdownHeader` keys it: the country and its identity, the date painted live. */
function BreakdownChip() {
  const paced = useMemo(() => pacedKey(clock), []);
  const identity = useIdentityKey(client, track, true);
  const shown = useHeadHour(clock, true, at => `GER|${identity(Math.floor(at), ['GER']) ?? paced(at)}`);
  const hour = Math.floor(shown);
  const labels = useCountryLabelsAt(client, track, useMemo(() => [{ tag: 'GER', hour }], [hour]), true);
  const live = useMemo(() => ({ read: () => clock.getHour(), subscribe: clock.subscribeFrame }), []);
  return <DatedCountryChip label={labels.labelAt(hour)('GER')} hour={hour} reason={null} live={live} />;
}

// On screen whatever the parts above it take: a followed panel follows only while shown.
const breakdownHost = document.body.appendChild(Object.assign(document.createElement('div'), { style: 'position: fixed; top: 0; left: 0; z-index: 1' }));
const breakdownRoot = createRoot(breakdownHost);

/** A strip of its own on a clock in the game's calendar (from January 1936), for its date picker. */
const JANUARY_1936 = 1936 * 12;
const calendarClock = new PlaybackController();
const calendarFirst = firstHourOfMonth(JANUARY_1936), calendarLast = firstHourOfMonth(JANUARY_1936 + 120);
calendarClock.setBounds({ firstHour: calendarFirst, lastHour: calendarLast });
const calendarTrack = { ...track, id: 'c', roleId: 'calendar', branch: { ...track.branch, id: '3', firstHour: calendarFirst, lastHour: calendarLast } } as BoundTrack;
const calendarHost = document.body.appendChild(document.createElement('div'));
const calendarRoot = createRoot(calendarHost);
const popover = () => document.querySelector('.date-time-popover:popover-open');
const button = (selector: string) => { const found = document.querySelector<HTMLButtonElement>(selector); if (!found) throw new Error(`no ${selector}`); return found; };

export const fixture = {
  /** Every caller mounted at `hour`, paused, with the country timeline read. */
  async mount(hour: number) {
    askTimeline(client, { sourceId: 'lease', revision: 'r1', branchId: '1', content: 0 }, source, hour);
    clock.seek(hour);
    root.render(<>
      {counted('strip', <PlaybackStrip clock={clock} tracks={[track]} alignment="calendar" defaultRoleId="role" />)}
      {counted('chip', <CountryChip widget={faction} runtime={runtime} active />)}
      {counted('section', <CountriesSection widget={faction} reference={track} runtime={runtime} limits={{ max: 10, ranked: false, aggregate: false } as never} onChange={() => {}} />)}
      {counted('map', <MapWidget widget={map} runtime={runtime} active mapOptions={{ fog: true } as never} headerSlot={null} onDiagnostics={() => {}} />)}
      {counted('identity', <Identity of={track} />)}
      {counted('unheld', <Identity of={unheld} />)}
      {counted('fallback', <Fallback on />)}
      {counted('off', <Fallback on={false} />)}
    </>);
    await settle();
  },
  /** Plays at `speed` for `frames` frames, then pauses: the frames whose hour changed, the renders and requests
   * between the first and last of them, and each hour-changing frame's painted date. */
  async play(speed: number, frames: number) {
    clock.setSpeed(speed);
    clock.setPlaying(true);
    await frame(); await frame();
    const before = { ...renders }, asking = asked.length, from = clock.getHour(), started = performance.now();
    const dates: string[] = [], tooltips: string[] = [];
    let changes = 0, last = clock.getHour();
    const stop = clock.subscribeFrame(() => {
      if (clock.getHour() === last) return;
      last = clock.getHour();
      changes++;
      dates.push(document.querySelector('.header-date .date-time-value')?.textContent ?? '');
      tooltips.push(document.querySelector('[data-breakdown-chip] .dash-widget-country')?.getAttribute('data-tooltip') ?? '');
    });
    for (let at = 0; at < frames; at++) await frame();
    stop();
    const during = Object.fromEntries(Object.keys(renders).map(id => [id, (renders[id] ?? 0) - (before[id] ?? 0)]));
    const requests = asked.slice(asking);
    const elapsed = performance.now() - started, to = clock.getHour();
    clock.setPlaying(false);
    await settle();
    return { changes, from, to, elapsed, renders: during, requests, dates: new Set(dates).size, tooltips: new Set(tooltips).size,
      chip: document.querySelector('.dash-widget-country')?.textContent ?? '', identity: document.querySelector('[data-identity="1"]')?.textContent ?? '' };
  },
  /** One seek while paused: the renders it caused. */
  async seek(hour: number) {
    const before = { ...renders };
    clock.seek(hour);
    await settle();
    return Object.fromEntries(Object.keys(renders).map(id => [id, (renders[id] ?? 0) - (before[id] ?? 0)]));
  },
  /** The roster and the breakdown chip mounted beside the rest. */
  /** The roster's committed renders and its render passes since last asked. */
  rosterPasses: () => { const passes = { ...rosterPasses }; rosterPasses.renders = 0; rosterPasses.commits = 0; return passes; },
  /** The service pushes the roster panel's text (as the viewer does when it differs). */
  async pushRoster(hour: number, sections: string[]) {
    const bytes = new TextEncoder().encode(`PNL1${JSON.stringify({ panels: [{ id: panels.id, hour, data: { sections, warnings: [] }, refused: null }] })}`);
    for (const receive of panels.receivers) receive(bytes);
    await settle();
  },
  /** Whether the service shows text panels; the breakdown is mounted again, to follow anew. */
  async panelsShown(on: boolean) {
    panels.on = on;
    breakdownRoot.render(<></>);
    await settle();
    await fixture.mountBreakdown();
  },
  async mountBreakdown() {
    breakdownRoot.render(<>{counted('roster', <Roster />)}<span data-breakdown-chip>{counted('breakdownChip', <BreakdownChip />)}</span></>);
    await settle();
  },
  breakdown: () => ({ roster: document.querySelector('[data-roster]')?.textContent ?? '', status: document.querySelector('[data-roster]')?.getAttribute('data-status') ?? '', error: document.querySelector('[data-roster]')?.getAttribute('data-error') ?? '',
    chip: document.querySelector('[data-breakdown-chip] .dash-widget-country')?.textContent ?? '',
    tooltip: document.querySelector('[data-breakdown-chip] .dash-widget-country')?.getAttribute('data-tooltip') ?? '' }),
  /** The calendar's strip mounted at the first hour of the month `month` months after January 1936, paused. */
  async mountCalendar(month: number) {
    calendarClock.seek(firstHourOfMonth(JANUARY_1936 + month));
    calendarRoot.render(counted('calendar', <PlaybackStrip clock={calendarClock} tracks={[calendarTrack]} alignment="calendar" defaultRoleId="calendar" />));
    await settle();
  },
  /** Opens or closes the date picker. */
  async toggleCalendar() {
    const trigger = calendarHost.querySelector<HTMLButtonElement>('.header-date .date-time-trigger');
    if (!trigger) throw new Error('no date picker');
    trigger.click();
    await settle();
  },
  /** What the open calendar shows, and the clock's head: each as month, day and time. */
  calendar() {
    const body = popover();
    const selected = body?.querySelectorAll('[role="gridcell"][aria-selected="true"]') ?? [];
    const date = gameDate(calendarClock.getHour())!;
    return { open: !!body?.querySelector('.calendar-grid'), month: body?.querySelector('.calendar-grid')?.getAttribute('aria-label') ?? '',
      days: [...selected].map(cell => Number(cell.querySelector('[data-day]')?.getAttribute('data-day'))),
      time: body?.querySelector('.calendar-time .select-trigger > span')?.textContent ?? '',
      head: { month: `${monthNames[date.month - 1]} ${date.year}`, day: date.day, hour: date.hour } };
  },
  /** Plays the calendar's clock at `speed` for `frames` frames, then pauses: the calendar renders, the hours, and each
   * hour-changing frame's highlighted day against the head's. */
  async playCalendar(speed: number, frames: number) {
    calendarClock.setSpeed(speed);
    calendarClock.setPlaying(true);
    await frame(); await frame();
    const before = renders.calendar ?? 0, from = calendarClock.getHour();
    const seen: { head: number; shown: number[] }[] = [];
    let last = calendarClock.getHour();
    const stop = calendarClock.subscribeFrame(() => {
      if (calendarClock.getHour() === last) return;
      last = calendarClock.getHour();
      const shown = [...popover()?.querySelectorAll('[role="gridcell"][aria-selected="true"] [data-day]') ?? []].map(day => Number(day.getAttribute('data-day')));
      seen.push({ head: gameDate(last)!.day, shown });
    });
    for (let at = 0; at < frames; at++) await frame();
    stop();
    const during = (renders.calendar ?? 0) - before, to = calendarClock.getHour();
    calendarClock.setPlaying(false);
    await settle();
    return { renders: during, changes: seen.length, from: gameDate(from), to: gameDate(to), seen };
  },
  /** Starts the calendar's clock and leaves it playing for `frames` frames; `pauseCalendar` stops it. */
  async startCalendar(speed: number, frames: number) {
    calendarClock.setSpeed(speed);
    calendarClock.setPlaying(true);
    for (let at = 0; at < frames; at++) await frame();
  },
  async pauseCalendar() { calendarClock.setPlaying(false); await settle(); },
  /** The user pages to the next month, or picks a day. */
  async nextMonth() { button('.date-time-popover:popover-open [aria-label="Next month"]').click(); await settle(); },
  async pickDay(day: number) { button(`.date-time-popover:popover-open [data-day="${day}"]`).click(); await settle(); },
  /** `identitySince` over `count` tags with `runs` runs each: the median microseconds of one call, a frame's work. */
  identityCost(count: number, runs: number) {
    const tags = Array.from({ length: count }, (_, at) => `T${at}`);
    const timeline = { final: true, colours: new Map(), reply: { known: [[0, LAST]] },
      runs: new Map(tags.map((tag, at) => [tag, Array.from({ length: runs }, (_, run) => [run * 1000 + at, run, -1, -1, -1] as const)])) } as unknown as CountryTimeline;
    const times: number[] = [];
    // Many calls a sample: the page's clock is coarse.
    for (let round = 0; round < 100; round++) {
      const started = performance.now();
      for (let call = 0; call < 2000; call++) identitySince(timeline, 10_000 + round * 37 + call, tags);
      times.push((performance.now() - started) / 2000 * 1000);
    }
    return times.sort((a, b) => a - b)[50]!;
  },
};
(window as unknown as { atHeadFixture: typeof fixture }).atHeadFixture = fixture;

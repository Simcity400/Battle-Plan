/** The acceptance suite's playback bars on the widgets that follow the head, in a real browser: political power, the
 * army race, air missions (a pushed panel whose counts are live cells) and a log mounted on a clock the test moves 15 hours a frame (1,440 h/s at 96 frames a second), fed one followed
 * message a frame as the service writes them; their renders, the requests made and the hours they report shown. */
// Before react-dom: a production-shaped hook it reports each commit to, for why each widget committed.
import { attributeCommits } from '../shell/commitCount.ts';
import { Component, Profiler, useCallback, useLayoutEffect, useMemo, useState } from 'react';
import type { ReactNode } from 'react';
import { createRoot } from 'react-dom/client';
import type { Root } from 'react-dom/client';
import MeasureWidget from '../widgets/country-measure/MeasureWidget';
import ArmySizeWidget from '../widgets/army-size/ArmySizeWidget';
import GameScoreWidget from '../widgets/game-score/GameScoreWidget';
import AirMissionsWidget from '../widgets/air-missions/AirMissionsWidget';
import OccupationTrendWidget from '../widgets/occupation-trend/OccupationTrendWidget';
import AirSuperiorityRegionWidget from '../widgets/air-superiority-region/AirSuperiorityRegionWidget';
import AirSuperiorityTimelineWidget from '../widgets/air-superiority-timeline/AirSuperiorityTimelineWidget';
import { CELLS } from './follow/liveCells.ts';
import WarsWidget from '../widgets/wars/WarsWidget';
import WarContributionWidget from '../widgets/war-contribution/WarContributionWidget';
import { aheadEnd, useRevealClock } from '../charts/reveal';
import { useAheadHour } from './aheadHour';
import LogList from '../charts/log/LogList';
import type { LogListSection } from '../charts/log/LogList';
import type { LogPage, LogRow } from '../charts/log/logData';
import BattleSides from '../widgets/battles/BattleSides';
import type { BattleCountryLabel, BattleRow, BattleSideSummary } from '../widgets/battles/battleRows';
import '../widgets/battles/battles.css';
import FactionsWidget from '../widgets/factions/FactionsWidget';
import Table from '../charts/table/Table';
import type { TableColumn } from '../charts/table/Table';
import type { Live, LiveValues } from '../charts/live';
import type { SortValue } from '../charts/table/logic';
import { useHeadHour } from '../widgets/army-size/headHour';
import TerritoryBreakdownWidget from '../widgets/territory-breakdown/TerritoryBreakdownWidget';
import { CELL } from './follow/message.ts';
import { message } from './follow/messages.fixture.ts';
import type { Spec } from './follow/messages.fixture.ts';
import { TILE_BINS } from './tiles/tile.ts';
import { MeasuredWidget, useShownEachFrame } from './widgetPerf.ts';
import { setPageClock } from './structurePace.ts';
import { inFrame } from './frameUpdates.ts';
import { measureWith } from '../shell/perf.ts';
import type { PerfRecorder } from '../shell/perf.ts';
import type { WidgetProps } from './definitions';

const native = window.requestAnimationFrame.bind(window);
const frame = () => new Promise<void>(resolve => native(() => resolve()));
async function settle(frames = 4) { for (let at = 0; at < frames; at++) await frame(); }

// What the widgets report shown, by widget, in order; every other measurement is dropped.
const shown = new Map<string, (number | null)[]>();
measureWith(new Proxy({ shown: (widget: string, hour: number | null) => { shown.set(widget, [...shown.get(widget) ?? [], hour]); } }, {
  get: (target, name) => (target as Record<string | symbol, unknown>)[name] ?? (() => {}),
}) as unknown as PerfRecorder);

const HOURS = 120_000;
const branch = { id: '1', firstHour: 0, lastHour: HOURS - 1, gaps: [] };
const track = { id: 'main', axisOffset: 0, source: { id: 'source', revision: '1', metadata: { countries: [], branches: [branch] } }, branch };
const TAGS = ['GER', 'SOV', 'ITA'];
/** The army score race's countries: four for a top three, its members trading places nearly every hour. */
const SCORE_TAGS = ['GER', 'SOV', 'ITA', 'ENG'];
const scoreRows = (hour: number) => SCORE_TAGS.map((tag, at) => [at + 1, 100 + ((at * 37 + hour * 7) % 50), tag] as const).sort((a, b) => b[1] - a[1] || a[0] - b[0]);

/** A clock the test moves: its frames are told when the test steps it. */
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

/** One faction through the whole run, GER leading and ITA joining at 5,000: no faction or roster changes while playing. */
const AXIS = { id: '5', name: 'Axis', recordedName: 'Axis', firstHour: 0, lastHour: HOURS - 1, nameReason: null,
  names: [{ firstHour: 0, lastHour: HOURS - 1, name: 'Axis', reason: null }], members: [{ tag: 'GER', firstHour: 0, lastHour: HOURS - 1 }, { tag: 'ITA', firstHour: 5000, lastHour: HOURS - 1 }],
  leaders: [{ tag: 'GER', firstHour: 0, lastHour: HOURS - 1 }], incomplete: [] };
const GROUPS = { branchId: '1', humans: [], factions: [AXIS], wars: [], warStarts: [], identityChanges: [], warnings: [],
  countryIdentities: TAGS.map((tag, slot) => ({ tag, identities: [{ firstHour: 0, lastHour: HOURS - 1, slot: slot + 1, countryId: slot + 1 }] })),
  capitulations: [], unobserved: { humans: [], factions: [], wars: [] } };

/** One war going on through the whole run of play: GER against SOV and ITA, from hour 1,000. */
const run = (tag: string, side: string) => ({ tag, side, firstHour: 1000, lastHour: HOURS - 1, joinedBy: false, leftBy: false });
const WARS = { branchId: '1', available: true, viewpoints: [{ tag: 'GER', firstHour: 0, lastHour: HOURS - 1 }], countryNames: [], unobserved: [],
  capitulationsUnobserved: [], warnings: [], wars: [{ id: '0', firstHour: 1000, lastHour: HOURS - 1, startedBy: false, endedBy: true, capitulations: [],
    observations: [{ firstHour: 1000, lastHour: HOURS - 1, viewpointTag: 'GER', viewpointSide: ['GER'], opposingSide: ['SOV', 'ITA'], attackersAreViewpoint: true,
      representative: ['GER', 'SOV'], name: { text: 'War', known: true, reason: null, key: null } }],
    participants: [run('GER', 'attacker'), run('ITA', 'defender'), run('SOV', 'defender')] }] };

/** Air superiority's moment: two sides in two regions, each side's friendly score a live cell of RaidAirScores. */
const SUPERIORITY_ROWS = ['ENG@10', 'ENG@11', 'GER@10', 'GER@11'];
const SUPERIORITY_KEYS = [[1, 10], [1, 11], [3, 10], [3, 11]].map(([country, region]) => (BigInt(country!) << 32n) | BigInt(region!));
const SUPERIORITY_FOLLOW = 2 ** 31 + 77;
const superiorityPart = (side: number, tag: string, region: number) => ({ side, tag, live: `${tag}@${region}` });
const superiorityCell = (region: number) => ({ kind: 'sides', parts: [superiorityPart(0, 'ENG', region), superiorityPart(1, 'GER', region)], viewpoints: [], unknown: [],
  noAirObject: [], filtered: false, excluded: 0 });
const SUPERIORITY = {
  missing: null, viewpoint: null, scale: 100_000, badBelow: 40_000, goodAbove: 60_000, warnings: [],
  sides: [{ key: 'f:7', faction: '7', colourFaction: null, factions: ['7'], tag: null, leader: 'ENG', coBelligerents: 0, anchor: null },
    { key: 'c:GER', faction: null, colourFaction: null, factions: [], tag: 'GER', leader: 'GER', coBelligerents: 0, anchor: null }],
  regions: [10, 11].map(id => ({ id, name: `Region ${id}`, nameMissing: null, active: true, wings: 'none', cell: superiorityCell(id) })),
  fallback: { kind: 'sides', parts: [], viewpoints: [], unknown: [], noAirObject: [], filtered: false, excluded: 0 },
};
const SUPERIORITY_LIVE = { groups: [{ table: 238, columns: [2], keys: SUPERIORITY_KEYS.map(String), follow: SUPERIORITY_FOLLOW }], rows: SUPERIORITY_ROWS,
  kinds: [{ id: 'friendly', op: 'value', scale: 1 }], cells: SUPERIORITY_ROWS.flatMap((_, row) => [row, 0, 1, 0, row, 0]) };
/** Region 10 is always the closer contest, so the order by contest holds while the scores move; its activity (its total)
 * crosses region 11's every hour, so the order by activity flips every hour, as regions' activity does on a real
 * recording. */
const superiorityScores = (hour: number) => [300n + BigInt(hour % 50), 900n, 400n + BigInt((hour % 2) * 600), 100n];
const superiorityShares = (hour: number) => { const eng = 300 + hour % 50, ger = 400 + (hour % 2) * 600; return { eng, ger }; };
/** The timeline's head day: the same moment, followed under its own cells. */
const TIMELINE_FOLLOW = 2 ** 31 + 78;
const TIMELINE_LIVE = { ...SUPERIORITY_LIVE, groups: [{ ...SUPERIORITY_LIVE.groups[0]!, follow: TIMELINE_FOLLOW }] };
/** The timeline over the asked days: regions 10 and 11, ENG ahead every day. */
function timeline(params: Record<string, unknown>) {
  const tile = params.tile as number;
  const first = Math.floor(Math.max(params.firstHour as number, tile * 6144) / 24) * 24, last = params.lastHour as number;
  const days = Math.floor((last - first) / 24) + 1;
  const cell = { kind: 'sides', parts: [{ side: 0, tag: 'ENG', friendly: '300' }, { side: 1, tag: 'GER', friendly: '100' }], viewpoints: [], unknown: [], noAirObject: [], filtered: false };
  return { firstHour: first, lastHour: last, dayHours: 24, days: Array.from({ length: days }, (_, day) => Math.min(last, first + day * 24 + 23)), excluded: Array(days).fill(0),
    sides: SUPERIORITY.sides, viewpoint: null, scale: 100_000, badBelow: 40_000, goodAbove: 60_000, warnings: [],
    rows: [10, 11].map(region => ({ region, name: `Region ${region}`, nameMissing: null, cells: [[days, cell]] })) };
}

function reply(product: string, params: Record<string, unknown>): unknown {
  if (product === 'wars') return WARS;
  if (product === 'war-contributions') return contributions(params.hour as number);
  if (product === 'air-superiority') return SUPERIORITY;
  if (product === 'air-superiority-timeline') return timeline(params);
  if (product === 'air-superiority-ranking') return { rows: [10, 11].map(region => ({ region, name: `Region ${region}`, nameMissing: null, cells: [] })), warnings: [] };
  if (product === 'follow') return { available: true };
  if (product === 'unfollow' || product === 'follow-clock') return {};
  if (product === 'country-groups') return GROUPS;
  if (product === 'country-styles') return { branchId: '1', hour: params.hour, styles: [], warnings: [] };
  if (product === 'country-facts') return { branchId: '1', known: [[0, HOURS - 1]], names: [], reasons: [], flags: [], countries: [], warnings: [] };
  if (product === 'series-tiles') {
    const { binHours, index, tags } = params as { binHours: number; index: number; tags: string[] };
    const first = index * TILE_BINS * binHours, last = first + TILE_BINS * binHours - 1;
    return { branchId: '1', measure: 'politicalPower', decimals: 5, binHours, index, firstHour: first, immutable: last < HOURS - 1, filled: Math.min(last, HOURS - 1),
      rawValues: true, hourBytes: 1, lines: tags.map(tag => ({ tag, missing: [{ firstHour: first, lastHour: last, reason: 'not recorded' }], parts: [] })),
      bins: btoa('\0'.repeat(TILE_BINS * tags.length)) };
  }
  if (product === 'series-state' && (params.family as { kind?: string } | undefined)?.kind === 'country-game-scores') return { branchId: '1', measure: 'army', decimals: 5,
    states: scoreRows(params.hour as number).slice(0, 3).map(([, value, tag]) => ({ tag, hour: params.hour, run: { firstHour: params.hour, lastHour: params.hour, valueRaw: String(value * 100_000) },
      drop: null, missing: null })) };
  if (product === 'army-size') return { branchId: '1', available: true, measures: ['divisions', 'battalions'], types: [], warnings: [], windows: [], window: null, interval: null, series: [] };
  throw new Error(`${product} is not served here`);
}

/** Air missions' panel at the head: two regions, the counts in one followed group of cells (`wings`, `m0`, `m1` of
 * each region; region 10 flies no `m1`). */
const AIR_CELLS = CELLS + 7;
const AIR_PANEL = { readHour: 30_000, missing: null, theatres: [], theatresMissing: null, warnings: [],
  missions: [{ id: '0', name: 'Air superiority', icon: null }, { id: '1', name: 'Close air support', icon: null }],
  regions: [{ region: '10', name: 'North Sea', missing: null, missions: ['0'] }, { region: '11', name: 'Rhineland', missing: null, missions: ['0', '1'] }],
  live: { groups: [{ table: 1, columns: [0, 1, 2], keys: ['10', '11'], follow: AIR_CELLS }], rows: ['10', '11'],
    kinds: [{ id: 'wings', op: 'value', scale: 1 }, { id: 'm0', op: 'value', scale: 1 }, { id: 'm1', op: 'value', scale: 1 }],
    cells: [0, 0, 1, 0, 0, 0, 0, 1, 1, 0, 0, 1, 1, 0, 1, 0, 1, 0, 1, 1, 1, 0, 1, 1, 1, 2, 1, 0, 1, 2] } };

/** Territory breakdown's panel at the head: GER and ITA controlling parts of region 7, the states each holds a live cell
 * over its denominator, pushed under one followed group. */
const TERRITORY_CELLS = CELLS + 9;
const TERRITORY_KEYS = [(1n << 32n) | 7n, (3n << 32n) | 7n];
const TERRITORY_PANEL = { measure: 'states', rule: 'controlled', regions: [{ id: 7, name: 'North', missing: null }], missing: null, areaReason: null, warnings: [],
  rows: [{ key: String(TERRITORY_KEYS[0]), tag: 'GER', slot: 1, region: 7 }, { key: String(TERRITORY_KEYS[1]), tag: 'ITA', slot: 3, region: 7 }],
  live: { groups: [{ table: 510, columns: [3, 4], keys: TERRITORY_KEYS.map(String), follow: TERRITORY_CELLS }], rows: TERRITORY_KEYS.map(String),
    kinds: [{ id: 'value', op: 'value', scale: 1 }, { id: 'denominator', op: 'value', scale: 1 }],
    cells: [0, 0, 1, 0, 0, 0, 0, 1, 1, 0, 0, 1, 1, 0, 1, 0, 1, 0, 1, 1, 1, 0, 1, 1] } };

/** Events of the log: three long over before the head, then one every 150 hours of the played span, each prepared,
 * launched 50 hours later and over 50 after that. */
const EVENTS = [...[1_000, 1_001, 1_002], ...Array.from({ length: 60 }, (_, at) => 30_000 + at * 150)]
  .map((start, at) => ({ id: String(at), phases: [start, start + 50, start + 100] }));
const PHASES = ['prepared', 'launched', 'over'];
const CHANGES = EVENTS.flatMap(event => event.phases).sort((a, b) => a - b);

/** Each event's phases shown at some hour from `from` to before `until`, as a list holds them ahead: dated by the phase,
 * a row moving on to its next phase takes a new key (the old one fades out, `leftHour`) and goes to the top, newest
 * first. */
function logRowsAhead(from: number, until: number, states = false): LogRow<string>[] {
  return EVENTS.flatMap(event => event.phases.flatMap((since, phase) => {
    const left = event.phases[phase + 1];
    if (since >= until || (left !== undefined && left <= from)) return [];
    return [{ key: JSON.stringify([event.id, since]), ...(states ? { id: event.id } : {}), hour: since, ...(left === undefined ? {} : { leftHour: left }), item: `Event ${event.id} ${PHASES[phase]}` }];
  })).sort((a, b) => b.hour - a.hour || a.key.localeCompare(b.key));
}

/** Each event begun by `hour`, dated by the phase it shows: what the list shows at `hour`. */
function logRows(hour: number): LogRow<string>[] {
  return EVENTS.flatMap(event => {
    let phase = event.phases.length - 1;
    while (phase >= 0 && event.phases[phase] > hour) phase--;
    if (phase < 0) return [];
    const since = event.phases[phase];
    return [{ id: event.id, key: JSON.stringify([event.id, since]), hour: since, item: `Event ${event.id} ${PHASES[phase]}` }];
  }).sort((a, b) => b.hour - a.hour || a.id.localeCompare(b.id));
}

/** The commits each counted widget rendered itself, apart from the commits of everything under it. */
const owned: Record<string, number> = {};
/** Why each widget committed while measured: its components that began the work, and what changed in them. */
const why = new Map<string, Map<string, number>>();
attributeCommits((_zones, causes) => {
  for (const [zone, named] of causes) {
    const id = zone.replace('widget:', '');
    let held = why.get(id);
    if (!held) why.set(id, held = new Map());
    for (const [cause, count] of named) held.set(cause, (held.get(cause) ?? 0) + count);
  }
});

/** A log widget as the log widgets are built: a block of rows rendered at once (`useAheadHour`), those past the head
 * drawn ahead and revealed by the clock each frame (`reveal`), by the shared list with motion and months, the hour shown
 * reported each frame. */
function PlaybackLog({ clock: at }: { clock: Clock }) {
  const end = useCallback((hour: number) => aheadEnd(CHANGES, hour, at.hoursPerSecond, at.playing), [at]);
  const block = useAheadHour(at, true, end);
  const rows = useMemo(() => logRowsAhead(block.hour, block.until), [block.hour, block.until]);
  const reveal = useRevealClock(at, true);
  useShownEachFrame(at, () => at.getHour());
  useLayoutEffect(() => { owned.log = (owned.log ?? 0) + 1; });
  return <LogList rows={rows} label="Synthetic events" density="comfortable" months animate playhead={block.hour} reveal={reveal} columns="minmax(0, 1fr)"
    renderRow={row => <span>{row.item}</span>} describe={row => row.item} />;
}

/** The same log with each event's phases as one row's states (`states`): one row per event, its phase changing in place
 * and gliding to its place at the phase's hour. */
function PlaybackStateLog({ clock: at }: { clock: Clock }) {
  const end = useCallback((hour: number) => aheadEnd(CHANGES, hour, at.hoursPerSecond, at.playing), [at]);
  const block = useAheadHour(at, true, end);
  const rows = useMemo(() => logRowsAhead(block.hour, block.until, true), [block.hour, block.until]);
  const reveal = useRevealClock(at, true);
  useLayoutEffect(() => { owned.stateLog = (owned.stateLog ?? 0) + 1; });
  return <LogList rows={rows} label="Synthetic event states" density="comfortable" months animate playhead={block.hour} reveal={reveal} states columns="minmax(0, 1fr)"
    renderRow={row => <span>{row.item}</span>} describe={row => row.item} />;
}

/** Rows leaving with no successor, as battles end and move out of view: sixty events begun long before the head, the
 * newest leaving first, one every 150 hours of the played span, none arriving. */
const LEAVING = Array.from({ length: 60 }, (_, at) => ({ id: `Leaving ${at}`, start: 1_000 + at, left: 30_000 + (59 - at) * 150 }));
const LEFT = LEAVING.map(event => event.left).sort((a, b) => a - b);
/** The leaving events not gone by `from`, newest first, each with the hour it leaves. */
const leavingRows = (from: number): LogRow<string>[] => LEAVING.filter(event => event.left > from)
  .map(event => ({ key: event.id, hour: event.start, leftHour: event.left, item: event.id })).sort((a, b) => b.hour - a.hour);

/** A log whose rows leave from the top of the view while playing: a block rendered at once, each row hidden at its hour
 * by the frame loop, the rows below closing up. */
function PlaybackLeavingLog({ clock: at }: { clock: Clock }) {
  const end = useCallback((hour: number) => aheadEnd(LEFT, hour, at.hoursPerSecond, at.playing), [at]);
  const block = useAheadHour(at, true, end);
  const rows = useMemo(() => leavingRows(block.hour), [block.hour]);
  const reveal = useRevealClock(at, true);
  useLayoutEffect(() => { owned.leaving = (owned.leaving ?? 0) + 1; });
  return <LogList rows={rows} label="Synthetic leaving events" density="comfortable" months={false} animate anchorRows playhead={block.hour} reveal={reveal}
    columns="minmax(0, 1fr)" renderRow={row => <span>{row.item}</span>} describe={row => row.item} />;
}

/** A battle log as the real recording shapes it, month headers off (the default): a battle begins every 200 hours and
 * lasts 600, each a row whose state (its losses so far) changes every 25 hours while it goes on, the newest battle at
 * the top; a battle beginning is revealed at the top of the view, above the first row shown. */
const BATTLES = Array.from({ length: 60 }, (_, at) => ({ id: `B${String(at).padStart(2, '0')}`, start: 29_000 + at * 200, end: 29_600 + at * 200 }));
const BATTLE_CHANGES = BATTLES.flatMap(battle => [...Array.from({ length: 24 }, (_, round) => battle.start + round * 25), battle.end]).sort((a, b) => a - b);
/** Each battle's states shown from `from` to before `until`, as the battle log holds them ahead: one row id a battle. */
function battleRowsAhead(from: number, until: number): LogRow<string>[] {
  return BATTLES.flatMap(battle => Array.from({ length: 24 }, (_, round) => {
    const since = battle.start + round * 25, left = Math.min(battle.start + (round + 1) * 25, battle.end);
    return since < until && left > from ? [{ key: `${battle.id}#${round}`, id: battle.id, hour: battle.start, revealedHour: since, leftHour: left,
      item: `${battle.id} round ${round}` }] : [];
  }).flat()).sort((a, b) => b.hour - a.hour || a.id!.localeCompare(b.id!) || a.revealedHour! - b.revealedHour!);
}
/** The battles going on at `hour`, newest first, each at the round it reached. */
const battlesAt = (hour: number) => BATTLES.filter(battle => battle.start <= hour && hour < battle.end).sort((a, b) => b.start - a.start)
  .map(battle => `${battle.id} round ${Math.floor((hour - battle.start) / 25)}`);

/** A naval battle's sides as its row draws them: flags measured to the room they have, and the ships of each side. */
const SIDE_TAGS = [['ENG', 'FRA', 'HOL', 'BEL'], ['GER', 'ITA', 'JAP']];
const battleSides = (round: number) => ({ id: '', provinceId: 0, windowIndex: 0, sides: SIDE_TAGS.map((tags, side) => ({ label: side ? 'Defenders' : 'Attackers',
  countries: tags.map(tag => ({ tag, hour: 29_000 })), countriesUnknown: false, losses: [],
  counts: [{ label: 'Ships', value: 10 + round + side, unknown: null }] })) }) as unknown as BattleRow;
const SIDES_SUMMARY: BattleSideSummary = { label: 'Sides', countLabel: 'Ships', separator: 'v', unknown: 'Not recorded',
  formatCount: (value, compact) => `${value.value ?? '—'}${compact ? '' : ' ships'}` };
const sideCountry = (tag: string) => ({ name: tag, flag: null, flagReason: 'no art in this fixture' }) as unknown as ReturnType<BattleCountryLabel>;

function PlaybackBattleLog({ clock: at }: { clock: Clock }) {
  const end = useCallback((hour: number) => aheadEnd(BATTLE_CHANGES, hour, at.hoursPerSecond, at.playing), [at]);
  const block = useAheadHour(at, true, end);
  const rows = useMemo(() => battleRowsAhead(block.hour, block.until), [block.hour, block.until]);
  const reveal = useRevealClock(at, true);
  useLayoutEffect(() => { owned.battles = (owned.battles ?? 0) + 1; });
  // Each state draws the battle's sides as Naval battles does: a state shown or a battle revealed is laid out and measured.
  return <LogList rows={rows} label="Synthetic battles" density="comfortable" months={false} animate anchorRows playhead={block.hour} reveal={reveal} states
    columns="minmax(0, 1fr) minmax(0, 1fr)" renderRow={row => <><BattleSides row={battleSides(Number(row.key.split('#')[1]))} country={sideCountry} summary={SIDES_SUMMARY} compact={false} />
      <span>{row.item}</span></>} describe={row => row.item} />;
}

/** A log of headed sections as the espionage log draws it: operations under way filling the view, and under it the
 * completed operations' page, its newest rows held ahead of the clock (no height until revealed) just past the view's
 * bottom edge, within the paged list's margin. */
const UNDER_WAY: LogRow<string>[] = Array.from({ length: 7 }, (_, at) => ({ key: `u${at}`, hour: 1_000 + at, item: `Under way ${at}` }));
const COMPLETED: LogPage<string> = { key: '0', start: 0, end: 39, count: 40, rows: Array.from({ length: 40 }, (_, at) => ({ key: `c${at}`, hour: 29_800 + at * 10, item: `Completed ${at}` })) };
const SECTIONS: LogListSection<string>[] = [{ key: 'under-way', label: 'Under way', months: false, rows: UNDER_WAY, note: 'None under way' },
  { key: 'completed', label: 'Completed', pages: [COMPLETED], note: 'None completed' }];

function PlaybackSectionLog({ clock: at }: { clock: Clock }) {
  const reveal = useRevealClock(at, true);
  useLayoutEffect(() => { owned.sections = (owned.sections ?? 0) + 1; });
  return <LogList sections={SECTIONS} label="Synthetic sections" density="comfortable" months={false} animate anchorRows playhead={at.getHour()} reveal={reveal}
    columns="minmax(0, 1fr)" renderRow={row => <span>{row.item}</span>} describe={row => row.item} />;
}

/** A ranked table as the scoreboard, game score and spending tables are on the real recording: its values painted each
 * frame, rows joining it while playing (a country entering the top, a payment read), sorted so that each one joining moves
 * every row below it (a rank marker on each), rows wrapped as the app wraps them by default. */
const TABLE_ROWS = Array.from({ length: 36 }, (_, at) => ({ tag: `C${String(at).padStart(2, '0')}`, joins: at < 16 ? 0 : 30_300 + (at - 16) * 600 }));
const tableRowsAt = (hour: number) => TABLE_ROWS.filter(row => row.joins <= hour);
type TableRow = (typeof TABLE_ROWS)[number];
const TABLE_COLUMNS: readonly TableColumn<TableRow>[] = [
  { key: 'name', header: 'Country', label: 'Country', width: 'minmax(0, 2fr)', value: row => row.tag },
  { key: 'value', header: 'Value', label: 'Value', numeric: true, width: '6em', bar: 'visible', value: () => null, paint: value => String(value) },
];
function PlaybackTable({ clock: at }: { clock: Clock }) {
  const hour = useHeadHour(at, true, head => String(tableRowsAt(head).length));
  const rows = useMemo(() => tableRowsAt(hour), [hour]);
  const live = useMemo<Live<LiveValues<SortValue>>>(() => ({
    read: () => { const head = at.getHour(); return { get: (key: string) => 100 + (head * 7 + key.charCodeAt(2) * 13) % 900, missing: () => 'Not recorded' }; },
    subscribe: changed => at.subscribeFrame(changed),
  }), [at]);
  useLayoutEffect(() => { owned.table = (owned.table ?? 0) + 1; });
  return <Table label="Synthetic ranked table" rows={rows} columns={TABLE_COLUMNS} rowKey={row => row.tag} rowRevision={row => [row]} live={live}
    sort={{ column: 'name', direction: 'descending' }} rank rankMarkers />;
}

/** Data landing every frame, as replies, names and measurements land through the page's deferred updates on the real
 * recording (run 13: a second and third commit after each structure commit, outside every widget's pace). */
function PlaybackLandings({ clock: at }: { clock: Clock }) {
  const [landed, setLanded] = useState(0);
  useLayoutEffect(() => at.subscribeFrame(() => inFrame(() => setLanded(count => count + 1))), [at]);
  useLayoutEffect(() => { owned.landings = (owned.landings ?? 0) + 1; });
  return <span data-case="landings">{landed}</span>;
}

let clock = new Clock();
let receive: ((bytes: ArrayBuffer | Uint8Array) => void) | null = null;
const requests: { product: string; params: Record<string, unknown> }[] = [];
const renders: Record<string, number> = {};
let root: Root | null = null;
let host: HTMLDivElement | null = null;

const client = {
  available: true,
  product: async (request: { sourceId: string; revision: string; product: string; params: Record<string, unknown> }) => {
    requests.push({ product: request.product, params: request.params });
    return { sourceId: request.sourceId, revision: request.revision, product: request.product, data: reply(request.product, request.params) };
  },
  onFollowMessages: async (handler: (bytes: ArrayBuffer | Uint8Array) => void) => { receive = handler; return () => { if (receive === handler) receive = null; }; },
  setMapViews: async () => {}, onMapStates: async () => () => {}, releaseSource: async () => {}, openSource: async () => { throw new Error('none'); },
};

/** The ids the store followed each widget's head by. */
function followed(): { power: number | null; army: number | null; air: number | null; resistance: number | null; compliance: number | null; contribution: number | null;
  superiority: number | null; timeline: number | null; score: number | null; territory: number | null } {
  let power: number | null = null, army: number | null = null, air: number | null = null, resistance: number | null = null, compliance: number | null = null,
    contribution: number | null = null, superiority: number | null = null, timeline: number | null = null, score: number | null = null, territory: number | null = null;
  for (const request of requests) {
    if (request.product !== 'follow') continue;
    const query = request.params.query as { product?: string; panel?: string; params?: { family?: { kind?: string; measure?: string } } };
    const family = query.params?.family;
    if (query.product === 'series-state' && family?.kind === 'occupier-totals') {
      if (family.measure === 'resistance') resistance = request.params.id as number; else compliance = request.params.id as number;
    } else if (query.product === 'series-state' && family?.kind === 'country-game-scores') score = request.params.id as number;
    else if (query.product === 'series-state') power = request.params.id as number;
    if (query.product === 'army-size') army = request.params.id as number;
    if (query.panel === 'air-missions') air = request.params.id as number;
    if (query.panel === 'war-contributions') contribution = request.params.id as number;
    if (query.panel === 'territory-breakdown') territory = request.params.id as number;
    // The region widget and the timeline follow the same moment (every region the settings show): one follow.
    if (query.panel === 'air-superiority' && !('regions' in query)) superiority = timeline = request.params.id as number;
    if (query.panel === 'air-superiority' && 'regions' in query) timeline = request.params.id as number;
  }
  return { power, army, air, resistance, compliance, contribution, superiority, timeline, score, territory };
}
let superiorityPanel: number | null = null;
let timelinePanel: number | null = null;

/** Air missions' panel pushed once, as the service pushes a structure. */
function pushAirPanel(hour: number) {
  const id = followed().air;
  if (id === null) return;
  receive?.(new TextEncoder().encode(`PNL1${JSON.stringify({ panels: [{ id, hour, data: AIR_PANEL, refused: null }] })}`));
  const territory = followed().territory;
  if (territory !== null) receive?.(new TextEncoder().encode(`PNL1${JSON.stringify({ panels: [{ id: territory, hour, data: TERRITORY_PANEL, refused: null }] })}`));
}

/** The war's contributions at `hour`, as the service pushes the panel: the same members, every number moving. */
const contributions = (hour: number) => ({ branchId: '1', hour, available: true, warnings: [], contributions: [{ viewpointSide: ['GER'], opposingSide: ['SOV', 'ITA'], reason: null,
  participants: TAGS.map((tag, at) => ({ tag, side: at ? 'opponent' : 'viewpoint', ...Object.fromEntries(['equipmentDamage', 'occupation', 'airDamage', 'strategicBombing',
    'shipDamage', 'convoyDamage', 'casualties', 'unknownCasualties', 'lendLeaseSent', 'lendLeaseReceived', 'score', 'participation']
    .map((key, column) => [key, String((hour % 89 + column + at + 1) * 100_000)])) })) }] });

/** One message at `hour`: political power for each tag, the race's top three. */
function deliver(hour: number) {
  const ids = followed(), answers: { id: number; answer: Spec }[] = [];
  if (ids.power !== null) answers.push({ id: ids.power, answer: { keys: TAGS.map((_, at) => BigInt(at + 1)), columns: 1,
    values: TAGS.map((_, at) => BigInt(100 + at * 50 + (hour % 97)) * 100_000n) } });
  for (const id of [ids.resistance, ids.compliance]) if (id !== null) answers.push({ id, answer: { keys: TAGS.map((_, at) => BigInt(at + 1)), columns: 1,
    values: TAGS.map((_, at) => BigInt(1_000_000 + at * 200_000 + (hour % 89) * 1_000)) } });
  if (ids.army !== null) {
    const rows = TAGS.map((tag, at) => [at + 1, 40 + ((at * 13 + hour) % 30), tag] as const).sort((a, b) => b[1] - a[1] || a[0] - b[0]);
    answers.push({ id: ids.army, answer: { keys: rows.map(([key]) => BigInt(key)), columns: 1, values: rows.map(([, value]) => BigInt(value)),
      states: rows.map(() => CELL.value), names: rows.map(([, , tag]) => tag) } });
  }
  if (ids.score !== null) {
    const rows = scoreRows(hour).slice(0, 3);
    answers.push({ id: ids.score, answer: { keys: rows.map(([key]) => BigInt(key)), columns: 1, values: rows.map(([, value]) => BigInt(value) * 100_000n),
      states: rows.map(() => CELL.value), names: rows.map(([, , tag]) => tag) } });
  }
  if (ids.air !== null) answers.push({ id: AIR_CELLS, answer: { keys: [10n, 11n], columns: 3,
    values: [3n, BigInt(24 + hour % 40), 0n, 5n, BigInt(60 + hour % 17), BigInt(hour % 11)] } });
  if (ids.superiority !== null && superiorityPanel !== ids.superiority) {
    superiorityPanel = ids.superiority;
    receive?.(new TextEncoder().encode(`PNL1${JSON.stringify({ panels: [{ id: ids.superiority, hour, data: { ...SUPERIORITY, live: SUPERIORITY_LIVE }, refused: null }] })}`));
  }
  if (ids.territory !== null) answers.push({ id: TERRITORY_CELLS, answer: { keys: TERRITORY_KEYS, columns: 2,
    values: [BigInt(20 + hour % 7), 3n, BigInt(5 + hour % 3), 3n] } });
  if (superiorityPanel !== null) answers.push({ id: SUPERIORITY_FOLLOW, answer: { keys: SUPERIORITY_KEYS, columns: 1, values: superiorityScores(hour) } });
  if (ids.timeline !== null && ids.timeline !== ids.superiority && timelinePanel !== ids.timeline) {
    timelinePanel = ids.timeline;
    receive?.(new TextEncoder().encode(`PNL1${JSON.stringify({ panels: [{ id: ids.timeline, hour, data: { ...SUPERIORITY, live: TIMELINE_LIVE }, refused: null }] })}`));
  }
  if (timelinePanel !== null) answers.push({ id: TIMELINE_FOLLOW, answer: { keys: SUPERIORITY_KEYS, columns: 1, values: superiorityScores(hour) } });
  receive?.(message(hour, answers));
  if (ids.contribution !== null)
    receive?.(new TextEncoder().encode(`PNL1${JSON.stringify({ panels: [{ id: ids.contribution, hour, data: contributions(hour), refused: null }] })}`));
}

/** What a widget threw, by widget: a widget that throws is kept from taking the others down with it. */
const errors: Record<string, string> = {};
class Boundary extends Component<{ id: string; children: ReactNode }, { failed: boolean }> {
  state = { failed: false };
  static getDerivedStateFromError() { return { failed: true }; }
  componentDidCatch(error: Error) { errors[this.props.id] = `${error.message} ${error.stack ?? ''}`.slice(0, 600); }
  render() { return this.state.failed ? null : this.props.children; }
}

/** One widget as the dashboard mounts it: measured. */
function Counted({ id, children }: { id: string; children: ReactNode }) {
  return <MeasuredWidget value={id}><Boundary id={id}>
    <Profiler id={id} onRender={() => { renders[id] = (renders[id] ?? 0) + 1; }}>{children}</Profiler>
  </Boundary></MeasuredWidget>;
}
const counted = (id: string, children: ReactNode) => <Counted id={id}>{children}</Counted>;

export const fixture = {
  /** Political power of three countries and the army race's top three, following the head, paused at 30,000. */
  async mount() {
    root?.unmount(); host?.remove();
    clock = new Clock(); requests.length = 0;
    // As the dashboard's grid does: the page's clock owns when anything but input commits while it plays.
    setPageClock(clock); shown.clear(); superiorityPanel = null; timelinePanel = null;
    for (const key of Object.keys(renders)) delete renders[key];
    for (const key of Object.keys(owned)) delete owned[key];
    for (const key of Object.keys(errors)) delete errors[key];
    host = document.body.appendChild(document.createElement('div'));
    host.style.cssText = 'width: 640px; display: flex; flex-direction: column;';
    root = createRoot(host);
    const runtime = { tracks: [track], clock, client, addWidgetBelow() {}, campaigns: [], recordings: [] };
    const power = { id: 'power', kind: 'political-power', trackIds: ['main'], countries: { kind: 'tags', tags: TAGS }, time: { mode: 'follow' }, options: {} };
    const wars = { id: 'wars', kind: 'wars', trackIds: ['main'], countries: { kind: 'selected' }, time: { mode: 'follow' }, options: { view: 'gantt' } };
    const contribution = { id: 'contribution', kind: 'war-contribution', trackIds: ['main'], countries: { kind: 'selected' }, time: { mode: 'follow' }, options: {} };
    const army = { id: 'army', kind: 'army-size', trackIds: ['main'], countries: { kind: 'group', group: { type: 'top', count: 3 } }, time: { mode: 'follow' }, options: { view: 'race' } };
    const air = { id: 'air', kind: 'air-missions', trackIds: ['main'], countries: { kind: 'tags', tags: ['GER'] }, time: { mode: 'follow' }, options: {} };
    const trend = { id: 'trend', kind: 'occupation-trend', trackIds: ['main'], countries: { kind: 'tags', tags: TAGS }, time: { mode: 'follow' },
      options: { view: 'lines', measure: 'both' } };
    const superiority = { id: 'superiority', kind: 'air-superiority-region', trackIds: ['main'], countries: { kind: 'tags', tags: [] }, time: { mode: 'follow' }, options: {} };
    // The same moment ordered by activity: one follow with the region widget's, its rows reordering every hour.
    const activeRegions = { id: 'activeRegions', kind: 'air-superiority-region', trackIds: ['main'], countries: { kind: 'tags', tags: [] }, time: { mode: 'follow' },
      options: { order: 'active' } };
    const timelineWidget = { id: 'timeline', kind: 'air-superiority-timeline', trackIds: ['main'], countries: { kind: 'tags', tags: [] }, time: { mode: 'follow' }, options: {} };
    const score = { id: 'score', kind: 'game-score', trackIds: ['main'], countries: { kind: 'group', group: { type: 'top', count: 3 } }, time: { mode: 'follow' },
      options: { view: 'race', topMeasure: 'army' } };
    const factions = { id: 'factions', kind: 'factions', trackIds: ['main'], countries: { kind: 'all' }, time: { mode: 'follow' }, options: {} };
    const territory = { id: 'territory', kind: 'territory-breakdown', trackIds: ['main'], countries: { kind: 'all' }, time: { mode: 'follow' }, options: {} };
    const props = (widget: unknown) => ({ widget, runtime, active: true, onDiagnostics() {} }) as unknown as WidgetProps;
    root.render(<>
      <div style={{ height: '196px', display: 'flex' }}>{counted('power', <MeasureWidget {...props(power)}
        config={{ measure: 'politicalPower', title: 'Political power', noun: 'political power', unit: 'PP' }} />)}</div>
      <div style={{ height: '196px', display: 'flex' }}>{counted('army', <ArmySizeWidget {...props(army)} />)}</div>
      <div style={{ height: '196px', display: 'flex', flexDirection: 'column' }}>{counted('air', <AirMissionsWidget {...props(air)} />)}</div>
      <div style={{ height: '240px', display: 'flex' }}>{counted('wars', <WarsWidget {...props(wars)} />)}</div>
      <div style={{ height: '240px', display: 'flex', flexDirection: 'column' }}>{counted('factions', <FactionsWidget {...props(factions)} />)}</div>
      {/* Each in the viewport, which follows only what is on screen (headless, 800 by 600). */}
      <div style={{ position: 'fixed', top: 400, left: 0, width: 640, height: 196, display: 'flex' }}>{counted('score', <GameScoreWidget {...props(score)} />)}</div>
      <div style={{ height: '320px', display: 'flex', position: 'fixed', top: 0, left: 660, width: 640 }}>{counted('trend', <OccupationTrendWidget {...props(trend)} />)}</div>
      <div style={{ position: 'fixed', top: 0, left: 0, width: '640px', height: '320px', zIndex: 1, display: 'flex' }}>{counted('contribution', <WarContributionWidget {...props(contribution)} />)}</div>
      {/* Wide enough that a tenth of a bar holds its share's label, as the widget and main show it only where it fits. */}
      <div data-case="superiority" style={{ position: 'fixed', top: 0, left: 80, width: 720, height: 300, display: 'flex' }}>{counted('superiority', <AirSuperiorityRegionWidget {...props(superiority)} />)}</div>
      <div data-case="activeRegions" style={{ position: 'fixed', top: 440, left: 660, width: 140, height: 160, zIndex: 2, display: 'flex' }}>{counted('activeRegions', <AirSuperiorityRegionWidget {...props(activeRegions)} />)}</div>
      <div style={{ position: 'fixed', top: 0, left: 1240, width: 400, height: 280, display: 'flex' }}>{counted('battles', <PlaybackBattleLog clock={clock} />)}</div>
      <div style={{ position: 'fixed', top: 320, left: 400, width: 400, height: 280, display: 'flex' }}>{counted('log', <PlaybackLog clock={clock} />)}</div>
      <div style={{ position: 'fixed', top: 320, left: 820, width: 400, height: 280, display: 'flex' }}>{counted('stateLog', <PlaybackStateLog clock={clock} />)}</div>
      <div style={{ position: 'fixed', top: 600, left: 400, width: 400, height: 280, display: 'flex' }}>{counted('leaving', <PlaybackLeavingLog clock={clock} />)}</div>
      <div style={{ position: 'fixed', top: 600, left: 820, width: 400, height: 280, display: 'flex' }}>{counted('sections', <PlaybackSectionLog clock={clock} />)}</div>
      <div style={{ position: 'fixed', top: 320, left: 0, width: 400, height: 280, display: 'flex' }}>{counted('timeline', <AirSuperiorityTimelineWidget {...props(timelineWidget)} />)}</div>
      <div style={{ position: 'fixed', top: 300, left: 420, width: 360, height: 300 }}>{counted('territory', <TerritoryBreakdownWidget {...props(territory)} />)}</div>
      {/* The app's stylesheet sizes every box by its border, which a row's height is measured against. */}
      <style>{'[data-case="table"], [data-case="table"] * { box-sizing: border-box; }'}</style>
      <div style={{ position: 'fixed', top: 0, left: 1700 }}>{counted('landings', <PlaybackLandings clock={clock} />)}</div>
      <div data-case="table" style={{ position: 'fixed', top: 600, left: 820, width: 400, height: 1200, display: 'flex', flexDirection: 'column' }}>{counted('table', <PlaybackTable clock={clock} />)}</div>
    </>);
    await settle(12);
    pushAirPanel(clock.getHour());
    await settle(12);
    deliver(clock.getHour());
    await settle(12);
  },
  /** Plays `frames` frames of `hours` each, a message each frame at the head's hour; what moved meanwhile. */
  async play(frames: number, hours: number) {
    clock.playing = true; clock.anchor();
    await settle(30);
    const before = { requests: requests.length, renders: { ...renders }, owned: { ...owned } };
    why.clear();
    const started = performance.now();
    for (let at = 0; at < frames; at++) {
      clock.step(hours);
      deliver(clock.getHour());
      await frame();
    }
    await settle();
    const elapsed = performance.now() - started;
    const asked = requests.slice(before.requests).filter(request => !['follow', 'unfollow', 'follow-clock'].includes(request.product));
    const timelines = asked.filter(request => request.product === 'air-superiority-timeline').map(request => request.params.lastHour as number);
    const rankings = asked.filter(request => request.product === 'air-superiority-ranking').map(request => request.params.lastHour as number);
    const rendered = Object.fromEntries(Object.entries(renders).map(([id, count]) => [id, count - (before.renders[id] ?? 0)]));
    const ownCommits = Object.fromEntries(Object.entries(owned).map(([id, count]) => [id, count - (before.owned[id] ?? 0)]));
    return { head: clock.getHour(), elapsed, owned: ownCommits, asked: asked.map(request => request.product === 'series-tiles'
      ? `series-tiles ${(request.params as { binHours: number }).binHours}:${(request.params as { index: number }).index}` : request.product), rendered, timelines, rankings,
      why: Object.fromEntries([...why].map(([id, causes]) => [id, Object.fromEntries(causes)])) };
  },
  pause() { clock.playing = false; clock.anchor(); },
  /** Pauses and lets what the pace held commit. */
  async pauseSettled() { clock.playing = false; clock.anchor(); await settle(); },
  errors: () => ({ ...errors }),
  /** Each air superiority region row shown, in order, with its segments' widths and labels as painted. */
  superiorityBars: () => [...(host?.querySelectorAll<HTMLElement>('[data-case="superiority"] .air-region-row') ?? [])].map(row => ({ region: row.dataset.row,
    segments: [...row.querySelectorAll<HTMLElement>('.air-region-segment')].map(segment => [segment.style.width, segment.querySelector('b')?.textContent ?? '']) })),
  followed,
  /** Every request but the tiles, for a failing assertion's message. */
  asked: () => requests.filter(request => request.product !== 'series-tiles').map(request => JSON.stringify(request).slice(0, 300)),
  /** The last hour each widget reported shown. */
  shown: () => Object.fromEntries([...shown].map(([id, hours]) => [id, hours.at(-1) ?? null])),
  /** Air missions' painted counts: each region row's texts, in row order. */
  airCounts: () => [...host!.querySelectorAll<HTMLElement>('.air-missions tbody tr:not([aria-hidden="true"])')]
    .map(row => [...row.querySelectorAll<HTMLElement>('.table-live')].map(cell => cell.textContent)),
};
Object.assign(fixture, {
  /** The log's rows a viewer sees, top first: neither held ahead, faded out nor leaving. */
  logShown: () => [...host!.querySelectorAll<HTMLElement>('[aria-label="Synthetic events"] .log-row')]
    .filter(row => (row.dataset.reveal === undefined || row.dataset.reveal === 'in') && !row.classList.contains('is-leaving'))
    .map(row => ({ y: Number.parseFloat(row.style.transform.replace('translateY(', '')), text: row.lastElementChild?.textContent ?? '' }))
    .sort((a, b) => a.y - b.y).map(row => row.text),
  /** The state log's rows a viewer sees, top first: each row's id, the state it shows and how many states it holds. */
  stateLogShown: () => [...host!.querySelectorAll<HTMLElement>('[aria-label="Synthetic event states"] .log-row')]
    .filter(row => (row.dataset.reveal === undefined || row.dataset.reveal === 'in') && !row.classList.contains('is-leaving'))
    .map(row => ({ y: Number.parseFloat(row.style.transform.replace('translateY(', '')), id: row.dataset.logId ?? '',
      text: row.querySelector(':scope > .log-state:not([hidden]) > span:last-child')?.textContent ?? '', states: row.querySelectorAll(':scope > .log-state').length }))
    .sort((a, b) => a.y - b.y).map(({ id, text, states }) => ({ id, text, states })),
  /** What the list shows at `hour`, newest first. */
  logAt: (hour: number) => logRows(hour).map(row => row.item),
  /** The leaving log's rows a viewer sees, top first. */
  leavingShown: () => [...host!.querySelectorAll<HTMLElement>('[aria-label="Synthetic leaving events"] .log-row')]
    .filter(row => (row.dataset.reveal === undefined || row.dataset.reveal === 'in') && !row.classList.contains('is-leaving'))
    .map(row => ({ y: Number.parseFloat(row.style.transform.replace('translateY(', '')), text: row.lastElementChild?.textContent ?? '' }))
    .sort((a, b) => a.y - b.y).map(row => row.text),
  /** The leaving events not gone at `hour`, newest first. */
  leavingAt: (hour: number) => leavingRows(hour).map(row => row.item),
  /** The battle log's rows a viewer sees, top first: the state each shows. */
  battlesShown: () => [...host!.querySelectorAll<HTMLElement>('[aria-label="Synthetic battles"] .log-row')]
    .filter(row => (row.dataset.reveal === undefined || row.dataset.reveal === 'in') && !row.classList.contains('is-leaving'))
    .map(row => ({ y: Number.parseFloat(row.style.transform.replace('translateY(', '')), text: row.querySelector(':scope > .log-state:not([hidden]) > span:last-child')?.textContent ?? '' }))
    .sort((a, b) => a.y - b.y).map(row => row.text),
  battlesAt,
  /** Each shown battle row's sides as painted: the flags shown and the count of those that did not fit (`+N`), per side. */
  battleFlags: () => [...host!.querySelectorAll<HTMLElement>('[aria-label="Synthetic battles"] .log-row')]
    .filter(row => row.dataset.reveal === undefined || row.dataset.reveal === 'in')
    .map(row => [...row.querySelectorAll<HTMLElement>(':scope > .log-state:not([hidden]) .battle-flags')].map(side => {
      const more = side.querySelector<HTMLElement>('.battle-flags-more');
      return [...side.querySelectorAll<HTMLElement>('.battle-country')].filter(flag => flag.style.display !== 'none').length + (more && !more.hidden ? Number(more.textContent!.slice(1)) : 0);
    })),
  /** How many rows the ranked table holds at `hour`. */
  tableRowsAt: (hour: number) => tableRowsAt(hour).length,
  /** The ranked table's row heights as drawn, and its row height. */
  tableHeights: () => ({ rows: [...host!.querySelectorAll<HTMLElement>('[data-case="table"] tbody tr')].map(row => row.getBoundingClientRect().height),
    rowHeight: host!.querySelector<HTMLElement>('[data-case="table"] .chart-table')?.style.getPropertyValue('--table-row-height') }),
  /** The regions the widget ordered by activity shows, top first. */
  activeOrder: () => [...host!.querySelectorAll<HTMLElement>('[data-case="activeRegions"] .air-region-row')].map(row => row.dataset.row ?? ''),
  superiorityShares,
});
(window as unknown as { playbackBars: typeof fixture & { logShown(): string[]; stateLogShown(): { id: string; text: string; states: number }[]; logAt(hour: number): string[];
  leavingShown(): string[]; leavingAt(hour: number): string[]; battlesShown(): string[]; battlesAt(hour: number): string[]; battleFlags(): number[][]; activeOrder(): string[]; tableRowsAt(hour: number): number;
  superiorityShares(hour: number): { eng: number; ger: number } } }).playbackBars = fixture as never;

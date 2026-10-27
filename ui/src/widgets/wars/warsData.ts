import { runAt } from '../../charts/series.ts';
import type { TimelineLane, TimelineSpan } from '../../charts/timeline/layout';
import type { TimelineLabel } from '../../charts/timeline/labelCatalogue';
import { bridgeGaps } from '../../charts/timeline/runState.ts';
import { entryAt } from '../../dashboard/reads.ts';
import { Lineage, joinTagChanges } from '../../dashboard/identity.ts';
import type { CountryIdentityIndex } from '../../dashboard/countryIdentity';
import { compareTags } from '../../dashboard/tagOrder.ts';
import type { HourSpan, WarCasualtyRun, WarContributionGroup, WarObservation, WarParticipantRun, WarSide, WarTimeline, WarsAtHour, WarsReply } from '../../generated/dashboard';
import type { CountryLabel } from '../../dashboard/countryFlagsAt';
import { dayLabel } from '../../charts/draw.ts';

export type WarsView = 'gantt' | 'countries';
export type WarsColour = 'war' | 'side';
export type NativeSide = 'viewpoint' | 'opponent';

/** The words a country name takes while it loads or where it was not recorded. */
export const NAME_WORDS = { pending: 'Loading…', missing: 'Not recorded' };

const object = (value: unknown): value is Record<string, unknown> => typeof value === 'object' && value != null;
const text = (value: unknown): value is string => typeof value === 'string';
const hour = (value: unknown) => Number.isSafeInteger(value);
const list = (value: unknown, check: (item: unknown) => boolean) => Array.isArray(value) && value.every(check);
const range = (value: unknown): value is HourSpan & Record<string, unknown> => object(value) && hour(value.firstHour) && hour(value.lastHour) && (value.firstHour as number) <= (value.lastHour as number);
const ordered = (ranges: readonly HourSpan[]) => ranges.every((item, index) => !index || ranges[index - 1].lastHour < item.firstHour);
const side = (value: unknown): value is WarSide => value === 'attacker' || value === 'defender' || value === 'viewpoint' || value === 'opponent';
const raw = (value: unknown) => text(value) && /^-?\d+$/.test(value);

/** Refuse a different or malformed contract before its dated values reach a chart. */
export function isWarsReply(value: unknown): value is WarsReply {
  if (!object(value) || !text(value.branchId) || typeof value.available !== 'boolean'
    || !list(value.viewpoints, item => range(item) && object(item) && text(item.tag)) || !ordered(value.viewpoints as HourSpan[])
    || !list(value.warnings, text) || !list(value.unobserved, range) || !list(value.capitulationsUnobserved, range)
    || !ordered(value.unobserved as HourSpan[]) || !ordered(value.capitulationsUnobserved as HourSpan[])
    || !list(value.countryNames, text) || !Array.isArray(value.wars)) return false;
  if (!(value.viewpoints as HourSpan[]).every(item => {
    const gap = entryAt(value.unobserved as HourSpan[], item.lastHour);
    return !gap || gap.lastHour < item.firstHour;
  })) return false;
  const ids = new Set<string>();
  return value.wars.every(war => {
    if (!range(war) || !object(war) || !text(war.id) || ids.has(war.id)
      || typeof war.startedBy !== 'boolean' || typeof war.endedBy !== 'boolean') return false;
    ids.add(war.id);
    const contained = (item: HourSpan) => war.firstHour <= item.firstHour && item.lastHour <= war.lastHour;
    const observed = (item: HourSpan) => {
      const gap = entryAt(value.unobserved as HourSpan[], item.lastHour);
      return contained(item) && (!gap || gap.lastHour < item.firstHour);
    };
    const shape = list(war.observations, observation => range(observation) && object(observation) && observed(observation)
      && text(observation.viewpointTag) && list(observation.viewpointSide, text) && list(observation.opposingSide, text)
      && (observation.attackersAreViewpoint === null || typeof observation.attackersAreViewpoint === 'boolean')
      && (observation.representative === null || (Array.isArray(observation.representative) && observation.representative.length === 2 && observation.representative.every(text)))
      && object(observation.name) && text(observation.name.text) && typeof observation.name.known === 'boolean'
      && (observation.name.reason === null || text(observation.name.reason)) && (observation.name.key === null || text(observation.name.key)))
      && (war.observations as WarObservation[]).length > 0 && ordered(war.observations as WarObservation[])
      && list(war.casualties, run => Array.isArray(run) && run.length === 6 && hour(run[0]) && hour(run[1]) && run[0] <= run[1]
        && observed({ firstHour: run[0], lastHour: run[1] }) && run.slice(2).every(raw))
      && (war.casualties as WarCasualtyRun[]).every((run, index, runs) => !index || runs[index - 1][1] < run[0])
      && list(war.participants, run => range(run) && object(run) && observed(run) && text(run.tag) && side(run.side)
        && typeof run.joinedBy === 'boolean' && typeof run.leftBy === 'boolean')
      && list(war.capitulations, event => object(event) && text(event.tag) && hour(event.hour));
    if (!shape) return false;
    const runsByTag = new Map<string, WarParticipantRun[]>();
    for (const run of war.participants as WarParticipantRun[]) {
      const runs = runsByTag.get(run.tag);
      if (runs) runs.push(run); else runsByTag.set(run.tag, [run]);
    }
    for (const runs of runsByTag.values()) runs.sort((a, b) => a.firstHour - b.firstHour);
    return (war.capitulations as WarTimeline['capitulations']).every(event =>
      runAt(runsByTag.get(event.tag) ?? [], event.hour) != null);
  });
}

export function validateWarsReply(value: unknown): WarsReply {
  if (!isWarsReply(value)) throw new Error('The wars reply is malformed.');
  const current = (value as { current?: unknown }).current;
  // Requested-hour groups and contributions are independent of preparation;
  // each snapshot keeps a reply's war contract.
  if (current !== undefined && !list(current, entry => object(entry) && hour(entry.hour) && typeof entry.available === 'boolean'
    && list(entry.contributions, group => object(group) && list(group.viewpointSide, text) && list(group.opposingSide, text)
      && (group.reason === null || text(group.reason)) && list(group.participants, row => object(row) && text(row.tag) && side(row.side)
        && ['equipmentDamage', 'occupation', 'airDamage', 'strategicBombing', 'shipDamage', 'convoyDamage', 'casualties',
          'unknownCasualties', 'lendLeaseSent', 'lendLeaseReceived', 'score', 'participation'].every(key => raw(row[key]))))
    && list(entry.warnings, text) && isWarsReply({ branchId: value.branchId, available: entry.available, viewpoints: [], countryNames: [],
      unobserved: [], capitulationsUnobserved: [], warnings: entry.warnings, wars: entry.wars }))) throw new Error('The wars reply is malformed.');
  return value;
}

/** Requested native groups and contributions at `at`, from that hour's frame
 * alone, independently of preparation; null when the reply lists no read. */
export function currentWarsAt(reply: WarsReply | undefined, at: number): WarsAtHour | null {
  const current = (reply as { current?: readonly WarsAtHour[] } | undefined)?.current;
  return current?.find(entry => entry.hour === at) ?? null;
}

/** The game's contributions for the war at an hour: its native group, or, while the
 * war holds several (a declaration the game lists beside the war it joins), the
 * largest group within it. */
export function warContributions(groups: readonly WarContributionGroup[] | undefined, observation: WarObservation | null): WarContributionGroup | null {
  if (!groups || !observation) return null;
  const same = (a: readonly string[], b: readonly string[]) => a.length === b.length && a.every((tag, index) => tag === b[index]);
  const exact = groups.find(group => same(group.viewpointSide, observation.viewpointSide) && same(group.opposingSide, observation.opposingSide));
  if (exact) return exact;
  const viewpoint = new Set(observation.viewpointSide), opposing = new Set(observation.opposingSide);
  let best: WarContributionGroup | null = null;
  for (const group of groups) {
    if (!group.viewpointSide.every(tag => viewpoint.has(tag)) || !group.opposingSide.every(tag => opposing.has(tag))) continue;
    const size = group.viewpointSide.length + group.opposingSide.length;
    if (!best || size > best.viewpointSide.length + best.opposingSide.length) best = group;
  }
  return best;
}

/** Covering observations only: an old name or roster never fills an unobserved hour. */
export function observationAt(war: WarTimeline, at: number): WarObservation | null {
  return runAt(war.observations, Math.floor(at));
}

/** Finished wars retain their final label and orientation; internal gaps stay unknown. */
export function labelObservationAt(war: WarTimeline, at: number): WarObservation | null {
  return at > war.lastHour ? war.observations.at(-1) ?? null : observationAt(war, at);
}

/** The observed overview viewpoint, including peacetime; nothing across a gap. */
export function viewpointAt(reply: WarsReply, at: number): string | null {
  return runAt(reply.viewpoints, Math.floor(at))?.tag ?? null;
}

export function viewpointSummary(country: ReturnType<CountryLabel> | null): string {
  return country == null || country.known === false ? 'Viewpoint not recorded at this hour'
    : country.known !== true ? 'Loading viewpoint…' : `Wars as ${country.name} sees them`;
}

export function warsReplyState(reply: Pick<WarsReply, 'available'> | null | undefined, error: string | null | undefined): 'loading' | 'unavailable' | 'error' | 'ready' {
  return error ? 'error' : !reply ? 'loading' : reply.available ? 'ready' : 'unavailable';
}

const casualtyIndexes = new WeakMap<WarTimeline, { firstHour: number; lastHour: number; run: WarCasualtyRun }[]>();

/** Exact, separate dated casualty series; there is no interpolation or cross-war sum. */
/** A war's start, telling apart wars the game names alike. */
const startNote = (war: WarTimeline) => ` (${war.startedBy ? 'by' : 'from'} ${dayLabel(war.firstHour)})`;

/** Each war's name at `at`; started wars sharing a recorded name are told apart by their start. */
export function warNamesAt(wars: readonly WarTimeline[], at: number): Map<WarTimeline, string> {
  const names = new Map(wars.map(war => [war, labelObservationAt(war, at)?.name.text ?? NAME_WORDS.missing]));
  const counts = new Map<string, number>();
  for (const [war, name] of names) if (war.firstHour <= at && name !== NAME_WORDS.missing) counts.set(name, (counts.get(name) ?? 0) + 1);
  for (const [war, name] of names) if ((counts.get(name) ?? 0) > 1) names.set(war, name + startNote(war));
  return names;
}

export function casualtiesAt(war: WarTimeline, at: number): WarCasualtyRun | null {
  let indexed = casualtyIndexes.get(war);
  if (!indexed) {
    indexed = war.casualties.map(run => ({ firstHour: run[0], lastHour: run[1], run }));
    casualtyIndexes.set(war, indexed);
  }
  return runAt(indexed, Math.floor(at))?.run ?? null;
}

export interface PreparedParticipant {
  key: string;
  lineage: Lineage;
  identities?: CountryIdentityIndex;
  /** First observed tag; labels resolve the country through its whole tag history. */
  tag: string;
  runs: WarParticipantRun[];
  holdings: WarParticipantRun[];
  pieces: WarBarInfo[];
  capitulations: number[];
}

export interface WarBarInfo {
  id: string;
  war: WarTimeline;
  participant?: PreparedParticipant;
  run?: WarParticipantRun;
  /** One country continuously on the same recorded side, across changes of tag. */
  holding?: WarParticipantRun;
  firstHour: number;
  lastHour: number;
  subrow: number;
  capitulations?: readonly number[];
}

export interface PreparedWar {
  war: WarTimeline;
  order: number;
  participants: PreparedParticipant[];
  byCountry: ReadonlyMap<string, PreparedParticipant>;
  existence: WarBarInfo[];
  nativeSides: ReadonlyMap<WarParticipantRun, NativeSide>;
}

export interface PreparedCountry {
  key: string;
  lineage: Lineage;
  identities?: CountryIdentityIndex;
  tag: string;
  spans: WarBarInfo[];
  subrows: number;
  firstHour: number;
}

export interface PreparedWars {
  reply: WarsReply;
  labelCatalogue: readonly TimelineLabel[];
  lineage: Lineage;
  identities?: CountryIdentityIndex;
  identityBounds: ReadonlyMap<string, readonly HourSpan[]>;
  unidentifiedHours: number;
  wars: PreparedWar[];
  byId: ReadonlyMap<string, PreparedWar>;
  countries: PreparedCountry[];
}

/** A refetch keeps expanded wars that still belong to this branch and viewpoint. */
export function retainExpandedWars(expanded: ReadonlySet<string>, byId: ReadonlyMap<string, PreparedWar>): ReadonlySet<string> {
  if ([...expanded].every(id => byId.has(id))) return expanded;
  return new Set([...expanded].filter(id => byId.has(id)));
}

export function participantHoldingAt(participant: PreparedParticipant, at: number): WarParticipantRun | null {
  return runAt(participant.holdings, Math.floor(at));
}

/** The tag this country had at the hour named, including after its last war. */
export function countryTagAt(view: PreparedParticipant | PreparedCountry, at: number): string | null {
  return view.identities ? view.identities.aliasesAt(new Set([view.key]), Math.floor(at))[0] ?? null
    : view.lineage.tagOf(view.key, Math.floor(at));
}

function nativeSide(run: WarParticipantRun, war: WarTimeline): NativeSide | null {
  if (run.side === 'viewpoint' || run.side === 'opponent') return run.side;
  const orientation = observationAt(war, run.firstHour)?.attackersAreViewpoint;
  if (orientation == null) return null;
  return (run.side === 'attacker') === orientation ? 'viewpoint' : 'opponent';
}

function prepareWar(war: WarTimeline, order: number, lineage: Lineage, identities?: CountryIdentityIndex): PreparedWar {
  const byCountry = new Map<string, PreparedParticipant>();
  const nativeSides = new Map<WarParticipantRun, NativeSide>();
  const grouped = new Map<string, WarParticipantRun[]>();
  if (identities) for (const run of war.participants) {
    const known = identities.membershipRuns([run]);
    const starts = [...new Set([run.firstHour, ...known.flatMap(span => [span.firstHour, span.lastHour + 1])])]
      .filter(hour => hour <= run.lastHour).sort((a, b) => a - b);
    for (let index = 0; index < starts.length; index++) {
      const firstHour = starts[index], lastHour = (starts[index + 1] ?? run.lastHour + 1) - 1;
      const key = identities.keyAt(run.tag, firstHour) ?? JSON.stringify(['unidentified', run.tag, firstHour]);
      const runs = grouped.get(key) ?? [];
      runs.push({ ...run, firstHour, lastHour,
        joinedBy: run.joinedBy || firstHour !== run.firstHour, leftBy: run.leftBy || lastHour !== run.lastHour });
      grouped.set(key, runs);
    }
  } else for (const runs of joinTagChanges(war.participants, lineage)) {
    grouped.set(lineage.countryAt(runs[0].tag, runs[0].firstHour), runs);
  }
  for (const [key, runs] of grouped) {
    runs.sort((a, b) => a.firstHour - b.firstHour);
    const holdings = bridgeGaps(runs, [], (earlier, later) => earlier.side === later.side).map(stretch => ({
      ...stretch.runs[0], firstHour: stretch.firstHour, lastHour: stretch.lastHour, leftBy: stretch.runs.at(-1)!.leftBy,
    }));
    const participant: PreparedParticipant = { key, lineage, identities, tag: runs[0].tag, runs, holdings, pieces: [], capitulations: [] };
    byCountry.set(key, participant);
    for (const run of runs) {
      const native = nativeSide(run, war);
      if (native) nativeSides.set(run, native);
    }
  }
  for (const event of war.capitulations) {
    const key = identities ? identities.keyAt(event.tag, event.hour) : lineage.countryAt(event.tag, event.hour);
    if (key !== null) byCountry.get(key)?.capitulations.push(event.hour);
  }
  for (const participant of byCountry.values()) {
    participant.capitulations.sort((a, b) => a - b);
    let eventIndex = 0;
    for (const run of participant.runs) {
      while (eventIndex < participant.capitulations.length && participant.capitulations[eventIndex] < run.firstHour) eventIndex++;
      const firstEvent = eventIndex;
      while (eventIndex < participant.capitulations.length && participant.capitulations[eventIndex] <= run.lastHour) eventIndex++;
      participant.pieces.push({
        id: JSON.stringify([war.id, participant.key, run.tag, run.side, run.firstHour, run.lastHour]),
        war, participant, run, holding: participantHoldingAt(participant, run.firstHour) ?? undefined,
        firstHour: run.firstHour, lastHour: run.lastHour, subrow: 0,
        capitulations: participant.capitulations.slice(firstEvent, eventIndex),
      });
    }
  }
  const existence: WarBarInfo[] = [];
  for (const observation of war.observations) {
    const last = existence.at(-1);
    if (last && observation.firstHour === last.lastHour + 1) last.lastHour = observation.lastHour;
    else existence.push({ id: JSON.stringify([war.id, observation.firstHour]), war, firstHour: observation.firstHour, lastHour: observation.lastHour, subrow: 0 });
  }
  // Index the compact casualty series once, away from a moving tooltip's frame.
  casualtiesAt(war, war.firstHour);
  return { war, order, byCountry, nativeSides, existence,
    participants: [...byCountry.values()].sort((a, b) => a.runs[0].firstHour - b.runs[0].firstHour || compareTags(a.tag, b.tag) || compareTags(a.key, b.key)) };
}

/** Country overlap slots are reserved per war within each connected overlap
 * stretch. Playback never changes their order, and one war cannot hide another. */
function countryView(key: string, pieces: WarBarInfo[], order: ReadonlyMap<WarTimeline, number>, lineage: Lineage, identities?: CountryIdentityIndex): PreparedCountry {
  const runPieces = new Map<WarParticipantRun, WarBarInfo[]>();
  for (const piece of pieces) {
    const entries = runPieces.get(piece.run!);
    if (entries) entries.push(piece); else runPieces.set(piece.run!, [piece]);
  }
  const runs = [...runPieces.keys()].sort((a, b) => a.firstHour - b.firstHour || order.get(runPieces.get(a)![0].war)! - order.get(runPieces.get(b)![0].war)!);
  let subrows = 1;
  let from = 0;
  while (from < runs.length) {
    let end = from + 1;
    let through = runs[from].lastHour;
    const wars = new Set<WarTimeline>([runPieces.get(runs[from])![0].war]);
    while (end < runs.length && runs[end].firstHour <= through) {
      through = Math.max(through, runs[end].lastHour);
      wars.add(runPieces.get(runs[end++])![0].war);
    }
    const slots = new Map([...wars].sort((a, b) => order.get(a)! - order.get(b)!).map((war, index) => [war, index]));
    subrows = Math.max(subrows, slots.size);
    for (let index = from; index < end; index++) for (const piece of runPieces.get(runs[index])!) piece.subrow = slots.get(piece.war)!;
    from = end;
  }
  pieces.sort((a, b) => a.firstHour - b.firstHour || a.subrow - b.subrow || order.get(a.war)! - order.get(b.war)!);
  return { key, lineage, identities, tag: runs[0].tag, spans: pieces, subrows, firstHour: runs[0].firstHour };
}

/** Index membership by tag and build overlap slots once per branch reply. Work is
 * linear in output runs apart from deterministic sorts, never countries × wars. */
export function prepareWars(reply: WarsReply, lineage = new Lineage([]), identities?: CountryIdentityIndex): PreparedWars {
  const countryNames = reply.countryNames;
  const wars = [...reply.wars].sort((a, b) => a.firstHour - b.firstHour || compareTags(a.id, b.id)).map((war, order) => prepareWar(war, order, lineage, identities));
  const order = new Map(wars.map(view => [view.war, view.order]));
  const byCountry = new Map<string, WarBarInfo[]>();
  for (const view of wars) for (const participant of view.participants) {
    let pieces = byCountry.get(participant.key);
    if (!pieces) { pieces = []; byCountry.set(participant.key, pieces); }
    // The Gantt and country lanes own different slot layouts.
    for (const piece of participant.pieces) pieces.push({ ...piece });
  }
  const countries = [...byCountry].map(([key, pieces]) => countryView(key, pieces, order, lineage, identities))
    .sort((a, b) => a.firstHour - b.firstHour || compareTags(a.tag, b.tag) || compareTags(a.key, b.key));
  const labelCatalogue = [
    ...[...new Set([NAME_WORDS.missing, NAME_WORDS.pending, ...wars.flatMap(view => view.war.observations.flatMap(observation =>
      [observation.name.text, observation.name.text + startNote(view.war)]))])]
      .map(text => ({ text, space: 38, weight: 600 })),
    ...[...new Set([NAME_WORDS.missing, NAME_WORDS.pending, ...countryNames])]
      .map(text => ({ text, space: 64, weight: 500 })),
    ...[...new Set(['ATTACKERS', 'DEFENDERS', 'OPPOSING SIDE', 'VIEWPOINT’S SIDE', ...countryNames.map(name => `${name}’s side`.toUpperCase())])]
      .map(text => ({ text, space: 38, weight: 600, size: 10 })),
  ];
  const identityBounds = new Map<string, HourSpan[]>();
  if (identities) {
    const tags = new Set([...reply.viewpoints.map(view => view.tag), ...reply.wars.flatMap(war =>
      [...war.participants.map(run => run.tag), ...war.observations.map(observation => observation.viewpointTag)])]);
    for (const run of identities.aliasRuns(new Set(countries.map(country => country.key)), -0x8000_0000, 0x7fff_ffff)) tags.add(run.tag);
    for (const run of identities.membershipRuns([...tags].map(tag => ({ tag, firstHour: -0x8000_0000, lastHour: 0x7fff_ffff })))) {
      const runs = identityBounds.get(run.tag) ?? [];
      runs.push(run); identityBounds.set(run.tag, runs);
    }
  }
  let unidentifiedHours = 0;
  if (identities) for (const view of wars) for (const participant of view.participants) for (const run of participant.runs) {
    if (identities.keyAt(run.tag, run.firstHour) === null) unidentifiedHours += run.lastHour - run.firstHour + 1;
  }
  return { reply, lineage, identities, identityBounds, unidentifiedHours, wars, byId: new Map(wars.map(view => [view.war.id, view])), countries, labelCatalogue };
}

export type CountryResolver = (tag: string) => ReturnType<CountryLabel> & { color?: string };

/** Oriented labels only when recorded; otherwise keep the native two arrays. */
export function sideLabel(value: WarSide, observation: WarObservation | null, country: CountryResolver): string {
  if (value === 'attacker') return 'Attackers';
  if (value === 'defender') return 'Defenders';
  const orientation = observation?.attackersAreViewpoint;
  if (orientation != null) return (value === 'viewpoint') === orientation ? 'Attackers' : 'Defenders';
  if (value === 'opponent') return 'Opposing side';
  const label = observation ? country(observation.viewpointTag) : null;
  return label?.known === true ? `${label.name}’s side` : 'Viewpoint’s side';
}

interface LaneBars { spansById: ReadonlyMap<string, WarBarInfo> }
export type WarsLaneInfo = LaneBars & (
  | { kind: 'war'; war: WarTimeline }
  | { kind: 'side'; war: WarTimeline; nativeSide: NativeSide; viewpointTag: string | null }
  | { kind: 'participant'; war: WarTimeline; participant: PreparedParticipant; tag: string | null }
  | { kind: 'country'; country: PreparedCountry; tag: string | null }
);

export interface WarsLaneOptions {
  view: WarsView;
  expanded: ReadonlySet<string>;
  colour: WarsColour;
  palette: readonly string[];
  country: CountryResolver;
  /** Only countries in view are resolved; lane order never depends on their names. */
  named: ReadonlySet<string>;
  sideColours?: ReadonlyMap<string, ReadonlyMap<NativeSide, string>>;
  neutral?: string;
}

const barMap = (pieces: readonly WarBarInfo[]) => new Map(pieces.map(piece => [piece.id, piece]));
const laneId = (...parts: string[]) => JSON.stringify(parts);

/** Only the started prefix is shaped; playback never scans every country's future runs. */
export function piecesThrough<P extends { firstHour: number }>(pieces: readonly P[], at: number): readonly P[] {
  let low = 0;
  let high = pieces.length;
  while (low < high) {
    const middle = Math.floor((low + high) / 2);
    if (pieces[middle].firstHour <= at) low = middle + 1; else high = middle;
  }
  return low === pieces.length ? pieces : pieces.slice(0, low);
}

function spanOf(piece: WarBarInfo, label: string, color: string | undefined, height: number): TimelineSpan {
  return { id: piece.id, firstHour: piece.firstHour, lastHour: piece.lastHour, label, color,
    barHeight: height, openEnd: true, subrow: piece.subrow,
    ...(piece.capitulations?.length ? { glyphShape: 'cross' as const, glyphHours: piece.capitulations } : {}) };
}

/** Shape the two views using the same prepared runs. Names, flags and colours are
 * requested only for the visible countries supplied by the timeline viewport. */
export function warsLanes(data: PreparedWars, at: number, options: WarsLaneOptions): { lanes: TimelineLane[]; info: Map<string, WarsLaneInfo> } {
  const lanes: TimelineLane[] = [];
  const info = new Map<string, WarsLaneInfo>();
  const looks = new Map<string, ReturnType<CountryResolver>>();
  const country = (tag: string | null): ReturnType<CountryResolver> => {
    if (tag === null) return { name: NAME_WORDS.missing, known: false, flagReason: 'Unknown country at this hour.' };
    let look = looks.get(tag);
    if (!look) { look = options.named.has(tag) ? options.country(tag) : { name: NAME_WORDS.pending, flagPending: true }; looks.set(tag, look); }
    return look;
  };
  const names = warNamesAt(data.wars.map(view => view.war), at);
  if (options.view === 'countries') {
    for (const view of data.countries) {
      if (view.firstHour > at) continue;
      const tag = countryTagAt(view, at);
      const id = laneId('country', view.key);
      const look = country(tag);
      const pieces = piecesThrough(view.spans, at);
      info.set(id, { kind: 'country', country: view, tag, spansById: barMap(pieces) });
      lanes.push({ id, label: '', singleLine: true, identity: { label: look.name, countryFlag: look }, icons: false, names: false,
        subrows: view.subrows, spans: pieces.map(piece => {
          const native = piece.run && data.byId.get(piece.war.id)!.nativeSides.get(piece.run);
          const fallback = piece.run?.side === 'attacker' ? options.palette[0] : piece.run?.side === 'defender' ? options.palette[1] : options.neutral;
          const color = options.colour === 'war' ? options.palette[data.byId.get(piece.war.id)!.order % options.palette.length]
            : (native && options.sideColours?.get(piece.war.id)?.get(native)) ?? fallback;
          return spanOf(piece, names.get(piece.war)!, color, 10);
        }) });
    }
    return { lanes, info };
  }
  for (const view of data.wars) {
    const war = view.war;
    if (war.firstHour > at) continue;
    const id = laneId('war', war.id);
    const observation = labelObservationAt(war, at);
    const pieces = piecesThrough(view.existence, at);
    info.set(id, { kind: 'war', war, spansById: barMap(pieces) });
    lanes.push({ id, heading: true, expanded: options.expanded.has(war.id), rowHeight: 30, singleLine: true,
      separator: false, separatorAfter: true, label: '', identity: { label: names.get(war)! }, muted: !observation?.name.known, icons: false, names: false,
      spans: pieces.map(piece => spanOf(piece, names.get(war)!, options.neutral, 10)) });
    if (!options.expanded.has(war.id)) continue;
    const groups: Record<NativeSide, PreparedParticipant[]> = { viewpoint: [], opponent: [] };
    for (const participant of view.participants) {
      const run = entryAt(participant.runs, at);
      if (!run) continue;
      const native = view.nativeSides.get(run);
      if (native) groups[native].push(participant);
    }
    const sides: NativeSide[] = observation?.attackersAreViewpoint === false ? ['opponent', 'viewpoint'] : ['viewpoint', 'opponent'];
    for (const native of sides) {
      const sideId = laneId('side', war.id, native);
      info.set(sideId, { kind: 'side', war, nativeSide: native, viewpointTag: observation?.viewpointTag ?? null, spansById: new Map() });
      lanes.push({ id: sideId, label: '', identity: { label: sideLabel(native, observation, country) }, heading: true,
        depth: 1, muted: true, labelStyle: 'side', separator: false, rowHeight: 18, singleLine: true, icons: false, names: false, spans: [] });
      for (const participant of groups[native]) {
        const tag = countryTagAt(participant, at);
        const participantId = laneId('participant', war.id, participant.key);
        const look = country(tag);
        const participantPieces = piecesThrough(participant.pieces, at);
        info.set(participantId, { kind: 'participant', war, participant, tag, spansById: barMap(participantPieces) });
        lanes.push({ id: participantId, label: '', identity: { label: look.name, countryFlag: look }, depth: 1, rowHeight: 22,
          singleLine: true, separator: false, icons: false, names: false, spans: participantPieces.map(piece => spanOf(piece, look.name, look.color ?? options.neutral, 8)) });
      }
    }
  }
  return { lanes, info };
}

/** The visible lanes' current country labels, including an unoriented side's viewpoint. */
export function laneCountries(lanes: readonly TimelineLane[], info: ReadonlyMap<string, WarsLaneInfo>, at: number): string[] {
  const tags = new Set<string>();
  for (const lane of lanes) {
    const about = info.get(lane.id);
    if (!about) continue;
    if (about.kind === 'participant' || about.kind === 'country') {
      const tag = countryTagAt(about.kind === 'participant' ? about.participant : about.country, at);
      if (tag !== null) tags.add(tag);
    }
    else if (about.kind === 'side' && about.nativeSide === 'viewpoint' && about.viewpointTag
      && labelObservationAt(about.war, at)?.attackersAreViewpoint == null) tags.add(about.viewpointTag);
  }
  return [...tags];
}

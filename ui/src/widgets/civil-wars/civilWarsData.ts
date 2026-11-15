import { countryNameDetails } from '../../charts/tooltip.ts';
import { usNumber } from '../../charts/numberFormat.ts';
import type { TimelineLane } from '../../charts/timeline/layout';
import type { TooltipContent, TooltipRow } from '../../charts/tooltip';
import { runState, startUnseen } from '../../charts/timeline/runState.ts';
import type { RunState } from '../../charts/timeline/runState';
import type { CountryLabel } from '../../dashboard/countryFlagsAt';
import type { CountryIdentityIndex } from '../../dashboard/countryIdentity';
import type { NameAt } from '../../dashboard/countryFacts';
import { isOrderedSpans } from '../../charts/series.ts';
import { isHolderTags, tagAt } from '../../dashboard/politicsHistory.ts';
import { object, opaqueTag } from '../../dashboard/validation.ts';
import type { CivilWar, CivilWarOutcome, CivilWarSide, CivilWarsReply, HourSpan, MembershipRun, SideExit } from '../../generated/dashboard';

/** The Civil wars widget's data: each civil war of the `civil-wars` product as a
 * heading with its sides beneath it, a breakaway indented under the side it broke
 * from. A war is ongoing at an hour while the game's civil-war predicate holds for
 * a side, as the reply dates it. A winner is shown only where the reply proves one
 * (every other side annexed by a side), and only once playback reaches the
 * read that proves it. */

/** A side whose tag at the hour is not recorded. */
export const NAME_NOT_KNOWN = 'Name not known';
export const NAME_LOADING = 'Loading…';

const hour = (value: unknown): value is number => Number.isSafeInteger(value);
const EXITS: ReadonlySet<unknown> = new Set(['annexedBySide', 'annexedBySideOtherwise', 'annexedByOther']);

function validSide(side: unknown, index: number, count: number, war: Record<string, unknown>): boolean {
  return object(side) && isHolderTags(side.tags) && hour(side.firstHour) && hour(side.lastHour)
    && (war.firstHour as number) <= side.firstHour && side.firstHour <= side.lastHour && side.lastHour <= (war.lastHour as number)
    && (side.parent === null || hour(side.parent) && side.parent >= 0 && side.parent < count && side.parent !== index)
    && typeof side.existedBefore === 'boolean'
    && (side.exit === null || object(side.exit) && EXITS.has(side.exit.kind) && hour(side.exit.hour) && hour(side.exit.readHour)
      && (side.exit.by === null || opaqueTag(side.exit.by)));
}

/** Whether following parents from every side reaches a root: a loop is malformed. */
function acyclic(sides: readonly CivilWarSide[]): boolean {
  const state = new Uint8Array(sides.length);
  for (let start = 0; start < sides.length; start++) {
    const path: number[] = [];
    let at: number | null = start;
    while (at !== null && state[at] === 0) { state[at] = 1; path.push(at); at = sides[at].parent; }
    if (at !== null && state[at] === 1) return false;
    for (const index of path) state[index] = 2;
  }
  return true;
}

function validWar(war: unknown, ids: Set<string>): boolean {
  if (!object(war) || typeof war.id !== 'string' || ids.has(war.id) || !hour(war.firstHour) || !hour(war.lastHour)
    || war.firstHour > war.lastHour || !isOrderedSpans(war.atWar)
    || war.atWar.some(span => span.firstHour < (war.firstHour as number) || span.lastHour > (war.lastHour as number))
    || !Array.isArray(war.sides) || !war.sides.length) return false;
  ids.add(war.id);
  const count = war.sides.length;
  const outcome = war.outcome;
  return war.sides.every((side, index) => validSide(side, index, count, war)) && acyclic(war.sides as CivilWarSide[])
    && (outcome === null || object(outcome) && hour(outcome.winner) && outcome.winner >= 0 && outcome.winner < count
      && hour(outcome.hour) && hour(outcome.readHour));
}

/** Checks a `civil-wars` reply's shape, so a reply from another version is refused
 * rather than read wrongly. */
export function validateCivilWarsReply(value: unknown): CivilWarsReply {
  const ids = new Set<string>();
  if (!object(value) || typeof value.branchId !== 'string' || !(value.lastHour === null || hour(value.lastHour)) || !isOrderedSpans(value.known)
    || !Array.isArray(value.warnings) || value.warnings.some(warning => typeof warning !== 'string')
    || !Array.isArray(value.wars) || !value.wars.every(war => validWar(war, ids))) throw new Error('The civil wars reply is malformed.');
  return value as unknown as CivilWarsReply;
}

/** The sides in tree order, each after the side it broke from, with its depth
 * beneath the heading (1 for a side that broke from none). */
export function sideTree(sides: readonly CivilWarSide[]): { index: number; depth: number }[] {
  const children = new Map<number, number[]>();
  const roots: number[] = [];
  sides.forEach((side, index) => {
    if (side.parent === null) { roots.push(index); return; }
    const list = children.get(side.parent);
    if (list) list.push(index); else children.set(side.parent, [index]);
  });
  const order: { index: number; depth: number }[] = [];
  const stack = roots.reverse().map(index => ({ index, depth: 1 }));
  while (stack.length) {
    const next = stack.pop()!;
    order.push(next);
    const below = children.get(next.index) ?? [];
    for (let at = below.length - 1; at >= 0; at--) stack.push({ index: below[at], depth: next.depth + 1 });
  }
  return order;
}

/** The side a heading names: one that existed before the outbreak, else the first. */
export function namedSide(war: CivilWar): number {
  const index = war.sides.findIndex(side => side.existedBefore);
  return index < 0 ? 0 : index;
}

/** The hours a side is listed with each tag: a tag holds from its hour until the next. */
export function sideTagRuns(side: CivilWarSide, through: number): MembershipRun[] {
  const last = Math.min(side.lastHour, through);
  const runs: MembershipRun[] = [];
  side.tags.forEach((entry, index) => {
    const firstHour = Math.max(side.firstHour, entry.hour);
    const lastHour = Math.min(last, (side.tags[index + 1]?.hour ?? Infinity) - 1);
    if (firstHour <= lastHour) runs.push({ tag: entry.tag, firstHour, lastHour });
  });
  return runs;
}

export interface SideAt {
  index: number;
  side: CivilWarSide;
  depth: number;
  /** Still listed at the shown hour, or at the last prepared hour: named at the shown hour. */
  live: boolean;
  /** The hour its lane names it at: the shown hour while live, else its last listed hour. */
  labelHour: number;
  /** Its tag at `labelHour`; null where none was recorded. */
  tag: string | null;
  state: RunState;
  /** How it left, once playback reaches the read recording it. */
  exit: SideExit | null;
  winner: boolean;
  /** Faded: another side's win is proven. */
  muted: boolean;
}

export interface CivilWarAt {
  war: CivilWar;
  /** Not known to have ended by the shown hour: the game's civil-war predicate holds
   * for a side then, or last held just before hours not read. */
  ongoing: boolean;
  /** The predicate's state at the shown hour. */
  state: RunState;
  /** The sides listed by the shown hour, in tree order. */
  sides: SideAt[];
  /** The heading's country, as tagged at the outbreak; null where none was recorded. */
  titleTag: string | null;
  /** The proven outcome, once playback reaches the read proving it. */
  outcome: CivilWarOutcome | null;
  /** The winner as tagged at the outcome. */
  winnerTag: string | null;
}

export interface CivilWarsOptions {
  /** Ended civil wars are listed. */
  ended: boolean;
  /** The chosen countries' native holders; null for every country. */
  countryIds: ReadonlySet<string> | null;
  identities: Pick<CountryIdentityIndex, 'keysDuring'> | null;
  /** Hours not recorded, not read or still being prepared, merged: a listing ending
   * before them has no known end. */
  unread: readonly HourSpan[];
  /** The last prepared hour; a side listed through it is still listed. */
  lastHour: number | null;
  /** The first hour shown (a range's start): civil wars over before it are left out. */
  from?: number;
}

export interface CivilWarsView {
  wars: CivilWarAt[];
  /** Civil wars reached by the shown hour, before narrowing to the chosen countries. */
  reached: number;
  /** Civil wars reached and involving the chosen countries, whether shown or hidden as ended. */
  total: number;
  ongoing: number;
  /** Shown sides and headings whose tag at their hour is not recorded. */
  unnamed: number;
}

/** Whether a chosen country was one of the war's sides by `at`, by native holder:
 * a tag reused by another country later never matches, and aliases do. */
function involves(sides: readonly SideAt[], at: number, chosen: ReadonlySet<string>, identities: CivilWarsOptions['identities']): boolean {
  if (!identities) return false;
  for (const side of sides) for (const key of identities.keysDuring(sideTagRuns(side.side, at))) if (chosen.has(key)) return true;
  return false;
}

/** The civil wars at `at` (the playback date with Live, else the end shown): each
 * war that broke out by then and the sides listed by then, ongoing ones first,
 * then the newest outbreak first. */
export function civilWarsAt(reply: Pick<CivilWarsReply, 'wars'>, at: number, options: CivilWarsOptions): CivilWarsView {
  const shown: CivilWarAt[] = [];
  let reached = 0, total = 0, ongoingCount = 0, unnamed = 0;
  for (const war of reply.wars) {
    if (war.firstHour > at || options.from !== undefined && war.lastHour < options.from) continue;
    reached++;
    const state = runState(war.atWar, at, options.unread);
    const ongoing = state.kind === 'held' || state.kind === 'unknown';
    const revealed = war.outcome && war.outcome.readHour <= at ? war.outcome : null;
    const sides: SideAt[] = [];
    for (const { index, depth } of sideTree(war.sides)) {
      const side = war.sides[index];
      if (side.firstHour > at) continue;
      const live = at <= side.lastHour || options.lastHour !== null && side.lastHour >= options.lastHour;
      const labelHour = live ? at : side.lastHour;
      const winner = revealed?.winner === index;
      sides.push({ index, side, depth, live, labelHour, tag: tagAt(side.tags, labelHour), state: runState([side], at, options.unread),
        exit: side.exit && side.exit.readHour <= at ? side.exit : null, winner, muted: !!revealed && !winner });
    }
    if (options.countryIds && !involves(sides, at, options.countryIds, options.identities)) continue;
    total++;
    if (ongoing) ongoingCount++;
    if (!ongoing && !options.ended) continue;
    const titleTag = tagAt(war.sides[namedSide(war)].tags, war.firstHour);
    unnamed += sides.filter(side => side.tag === null).length + (titleTag === null ? 1 : 0);
    shown.push({ war, ongoing, state, sides, titleTag, outcome: revealed,
      winnerTag: revealed ? tagAt(war.sides[revealed.winner].tags, revealed.hour) : null });
  }
  shown.sort((a, b) => Number(b.ongoing) - Number(a.ongoing) || b.war.firstHour - a.war.firstHour || (a.war.id < b.war.id ? -1 : a.war.id > b.war.id ? 1 : 0));
  return { wars: shown, reached, total, ongoing: ongoingCount, unnamed };
}

export type Look = ReturnType<CountryLabel> & { color?: string };

export const unknownLook = (): Look => ({ name: NAME_NOT_KNOWN, known: false, flagReason: 'Unknown country at this hour.' });

export interface LaneText {
  /** A side as its lane names it: at the shown hour while `live`, else at `hour`. */
  side: (tag: string, hour: number, live: boolean) => Look;
  /** A country as recorded at an event's hour, such as the heading's at the outbreak. */
  dated: (tag: string, hour: number) => Look;
  day: (hour: number) => string;
  /** The heading bar's colour, and a side's without a recorded colour. */
  neutral: string;
  /** The theme's positive colour, for the winner's pill. */
  positive: string;
}

export type CivilWarLaneInfo = { kind: 'war'; war: CivilWarAt } | { kind: 'side'; war: CivilWarAt; side: SideAt };

const laneId = (...parts: string[]) => JSON.stringify(parts);
const plural = (count: number, one: string, many: string) => `${usNumber.format(count)} ${count === 1 ? one : many}`;

export const headingTitle = (name: string) => `Civil war in ${name}`;

/** The heading's name: the country that existed before the outbreak, as named then. */
export function titleLook(war: CivilWarAt, dated: LaneText['dated']): Look {
  return war.titleTag === null ? unknownLook() : dated(war.titleTag, war.war.firstHour);
}

/** The heading's second line: the proven winner, else whether it is still going. */
export function headingLine(war: CivilWarAt, dated: LaneText['dated']): string {
  if (war.outcome) return `Winner: ${war.winnerTag === null ? NAME_NOT_KNOWN : dated(war.winnerTag, war.outcome.hour).name}`;
  return war.ongoing ? `Ongoing · ${plural(war.sides.length, 'side', 'sides')}` : 'Outcome unknown';
}

/** A side's line under its name: when it joined, left or was annexed. */
export function sideLine(side: SideAt, day: (hour: number) => string, branchFirst: number, unread: readonly HourSpan[]): string {
  if (side.exit) return `Annexed ${day(side.exit.hour)}`;
  const state = side.state;
  if (side.live || state.kind === 'held') return `Since ${startUnseen(side.side.firstHour, branchFirst, unread) ? 'by ' : ''}${day(side.side.firstHour)}`;
  // The winner's listing ends with the war it won.
  if (state.kind === 'ended') return `${side.winner ? 'Won' : 'Left'} ${state.lastSeen == null ? '' : 'by '}${day(state.at)}`;
  return state.kind === 'unknown' ? `Last recorded ${day(state.lastSeen)}` : '';
}

/** The side's look in its lane. */
export function sideLook(side: SideAt, text: Pick<LaneText, 'side'>): Look {
  return side.tag === null ? unknownLook() : text.side(side.tag, side.labelHour, side.live);
}

/** The timeline's lanes: a heading per civil war, expanded unless collapsed, with
 * a lane per side beneath it, each breakaway indented under the side it broke from. */
export function civilWarLanes(view: readonly CivilWarAt[], collapsed: ReadonlySet<string>, text: LaneText, branchFirst: number,
  unread: readonly HourSpan[]): { lanes: TimelineLane[]; info: Map<string, CivilWarLaneInfo> } {
  const lanes: TimelineLane[] = [];
  const info = new Map<string, CivilWarLaneInfo>();
  for (const war of view) {
    const id = laneId('war', war.war.id);
    const title = headingTitle(titleLook(war, text.dated).name);
    const expanded = !collapsed.has(war.war.id);
    info.set(id, { kind: 'war', war });
    lanes.push({ id, heading: true, expanded, separator: false, separatorAfter: true, icons: false, names: false,
      identity: { label: title }, label: headingLine(war, text.dated),
      spans: [{ id: war.war.id, firstHour: war.war.firstHour, lastHour: war.war.lastHour, label: title, color: text.neutral, openEnd: true, barHeight: 10 }] });
    if (!expanded) continue;
    for (const side of war.sides) {
      const sideId = laneId('side', war.war.id, String(side.index));
      const look = sideLook(side, text);
      info.set(sideId, { kind: 'side', war, side });
      lanes.push({ id: sideId, depth: side.depth, separator: false, icons: false, names: false, muted: side.muted,
        identity: { label: look.name, countryFlag: look }, label: sideLine(side, text.day, branchFirst, unread),
        ...(side.winner ? { pill: { text: 'Winner', color: text.positive } } : {}),
        spans: [{ id: sideId, firstHour: side.side.firstHour, lastHour: side.side.lastHour, label: look.name,
          color: look.color ?? text.neutral, openEnd: true, barHeight: 8 }] });
    }
  }
  return { lanes, info };
}

// Live sides use the shown hour; headings and former sides use their supplying hours.
export function laneCountries(lanes: readonly TimelineLane[], info: ReadonlyMap<string, CivilWarLaneInfo>): { live: string[]; dated: NameAt[] } {
  const live = new Set<string>();
  const dated = new Map<string, NameAt>();
  const add = (tag: string | null, hour: number) => { if (tag !== null) dated.set(JSON.stringify([hour, tag]), { hour, tag }); };
  for (const lane of lanes) {
    const about = info.get(lane.id);
    if (!about) continue;
    if (about.kind === 'war') {
      add(about.war.titleTag, about.war.war.firstHour);
      if (about.war.outcome) add(about.war.winnerTag, about.war.outcome.hour);
    } else if (about.side.tag !== null) {
      if (about.side.live) live.add(about.side.tag); else add(about.side.tag, about.side.labelHour);
    }
  }
  return { live: [...live], dated: [...dated.values()] };
}

/** A side's tag at the hour its tooltip names it: the hovered hour within the hours it is listed. */
export function sideNameHour(side: SideAt, hovered: number): number {
  return Math.min(Math.max(Math.floor(hovered), side.side.firstHour), side.labelHour);
}

/** The countries a tooltip names at fixed hours: for a heading, its title, every side
 * and the winner; for a side, the side it broke from and who annexed it. */
export function tooltipCountries(about: CivilWarLaneInfo): NameAt[] {
  const wanted: NameAt[] = [];
  const add = (tag: string | null | undefined, hour: number) => { if (tag != null) wanted.push({ hour, tag }); };
  const { war } = about;
  if (about.kind === 'war') {
    add(war.titleTag, war.war.firstHour);
    for (const side of war.sides) add(side.tag, side.labelHour);
    if (war.outcome) add(war.winnerTag, war.outcome.hour);
  } else {
    const parent = about.side.side.parent;
    if (parent !== null) add(tagAt(war.war.sides[parent].tags, about.side.side.firstHour), about.side.side.firstHour);
    if (about.side.exit) add(about.side.exit.by, about.side.exit.hour);
  }
  return wanted;
}

export interface TooltipText {
  /** Countries as recorded at an hour. */
  dated: (tag: string, hour: number) => Look;
  day: (hour: number) => string;
  branchFirst: number;
  unread: readonly HourSpan[];
}

function endText(state: RunState, held: string, text: TooltipText): string {
  switch (state.kind) {
    case 'held': case 'none': return held;
    case 'ended': return state.lastSeen == null ? text.day(state.at) : `by ${text.day(state.at)}`;
    case 'unknown': return `Not recorded after ${text.day(state.lastSeen)}`;
  }
}

const startText = (firstHour: number, text: TooltipText) => `${startUnseen(firstHour, text.branchFirst, text.unread) ? 'by ' : ''}${text.day(firstHour)}`;

const listNames = (names: readonly string[]) => names.length <= 1 ? names.join('')
  : `${names.slice(0, -1).join(', ')} and ${names.at(-1)}`;

/** A heading's tooltip: dates, sides and the outcome with its evidence. */
export function warTooltip(war: CivilWarAt, hovered: number, text: TooltipText): TooltipContent {
  const name = (tag: string | null, hour: number) => tag === null ? unknownLook() : text.dated(tag, hour);
  // A listing the predicate never held for has no end to name.
  const rows: TooltipRow[] = [
    { label: 'Started', value: startText(war.war.firstHour, text) },
    ...war.state.kind === 'none' ? [] : [{ label: 'Ended', value: endText(war.state, 'Ongoing', text) }],
  ];
  let note: string | undefined;
  if (war.outcome) {
    const loserLooks = war.sides.filter(side => !side.winner).map(side => name(side.tag, side.labelHour));
    const losers = loserLooks.map(look => look.name);
    const winner = name(war.winnerTag, war.outcome.hour);
    rows.push({ ...countryNameDetails(winner, ...loserLooks), label: 'Outcome', value: `Winner: ${winner.name} — ${listNames(losers)} annexed by a side by ${text.day(war.outcome.hour)}`, tone: 'positive' });
  } else if (!war.ongoing) {
    rows.push({ label: 'Outcome', value: 'Unknown', tone: 'muted' });
    note = 'No side was left after the others were annexed by a side.';
  }
  rows.push({ label: 'Sides', value: String(war.sides.length), divider: true });
  for (const side of war.sides) {
    const look = name(side.tag, side.labelHour);
    rows.push({ label: look.name, countryFlag: look, indent: true, short: true, ...(side.winner ? { value: 'Winner', tone: 'positive' as const } : {}) });
  }
  return { ...countryNameDetails(name(war.titleTag, war.war.firstHour)), title: headingTitle(name(war.titleTag, war.war.firstHour).name), subtitle: `Civil war · ${text.day(hovered)}`, rows, ...(note ? { note } : {}) };
}

/** How a side left, as the recorded annexation says. */
export function exitText(exit: SideExit, by: string): string {
  switch (exit.kind) {
    case 'annexedBySide': return by;
    case 'annexedBySideOtherwise': return `${by}, not by the civil war’s own annexation`;
    case 'annexedByOther': return `${by}, which was not a side`;
  }
}

/** A side's tooltip: its name and tag at the hovered hour, the side it broke from, and how it left. */
export function sideTooltip(war: CivilWarAt, side: SideAt, hovered: number, look: Look, tag: string | null, text: TooltipText): TooltipContent {
  const rows: TooltipRow[] = [{ label: 'Tag', value: tag ?? 'Not recorded', tone: tag === null ? 'muted' : undefined }];
  const parent = side.side.parent;
  if (parent !== null) {
    const parentTag = tagAt(war.war.sides[parent].tags, side.side.firstHour);
    const label = parentTag === null ? unknownLook() : text.dated(parentTag, side.side.firstHour);
    rows.push({ label: 'Broke from', value: `${label.name} on ${text.day(side.side.firstHour)}`, countryFlag: label });
  } else rows.push({ label: 'Since', value: startText(side.side.firstHour, text) });
  if (side.exit) {
    const by = side.exit.by === null ? { name: 'Not recorded', known: false } : text.dated(side.exit.by, side.exit.hour);
    rows.push({ label: 'Annexed by', value: exitText(side.exit, by.name), ...(side.exit.by === null ? { tone: 'muted' as const } : { countryFlag: by }) },
      { label: 'Annexed', value: text.day(side.exit.hour) });
  } else rows.push({ label: 'Left', value: side.live ? 'Still a side' : endText(side.state, 'Still a side', text) });
  if (side.winner) rows.push({ label: 'Outcome', value: 'Winner', tone: 'positive' });
  return { title: look.name, countryFlag: look, subtitle: `Side · ${text.day(hovered)}`, rows };
}

/** The dated Jump destinations a lane offers: the outbreak, a split and the end
 * (the last hour the war was recorded ongoing). */
export function jumpHours(about: CivilWarLaneInfo): { label: string; hour: number }[] {
  const { war } = about;
  const hours = [{ label: 'the outbreak', hour: war.war.firstHour }];
  if (about.kind === 'side' && about.side.side.parent !== null) hours.push({ label: 'the split', hour: about.side.side.firstHour });
  if (war.state.kind === 'ended') hours.push({ label: 'the end', hour: war.state.lastSeen ?? war.state.at - 1 });
  return hours;
}

export const summaryText = (view: Pick<CivilWarsView, 'total' | 'ongoing'>) =>
  `${plural(view.total, 'civil war', 'civil wars')} · ${usNumber.format(view.ongoing)} ongoing`;

/** Every hour from which what the civil-wars timeline shows may differ: a war, a fight, a side or a tag starting, the
 * hour after one ends, an exit or an outcome dated or read, a stretch not read starting or ending, a country changing
 * identity. Ascending, distinct. */
export function changeHours(reply: Pick<CivilWarsReply, 'wars'>, unread: readonly HourSpan[], identityChanges: readonly { hour: number }[] = []): number[] {
  const hours = new Set<number>();
  const span = (first: number, last: number) => { hours.add(first); hours.add(last + 1); };
  for (const war of reply.wars) {
    span(war.firstHour, war.lastHour);
    for (const fight of war.atWar) span(fight.firstHour, fight.lastHour);
    for (const side of war.sides) {
      span(side.firstHour, side.lastHour);
      for (const tag of side.tags) hours.add(tag.hour);
      if (side.exit) { hours.add(side.exit.hour); hours.add(side.exit.readHour); }
    }
    if (war.outcome) { hours.add(war.outcome.hour); hours.add(war.outcome.readHour); }
  }
  for (const gap of unread) span(gap.firstHour, gap.lastHour);
  for (const change of identityChanges) hours.add(change.hour);
  return [...hours].sort((a, b) => a - b);
}

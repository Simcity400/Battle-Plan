import { colour, readableColour } from '../../charts/palette.ts';
import { isOrderedSpans } from '../../charts/series.ts';
import { runState, startUnseen, type RunState } from '../../charts/timeline/runState.ts';
import { countryFromKey, isSlotKey } from '../../dashboard/countryKey.ts';
import { isHolderTags, tagAt, unknownHours } from '../../dashboard/politicsHistory.ts';
import { object, opaqueTag } from '../../dashboard/validation.ts';
import { unavailableArt } from '../../families/art/types.ts';
import type { BoundedReadView } from '../../families/contract.ts';
import type { HolderRun, HourRanges } from '../../families/holders.ts';
import { registerFamilyProjection } from '../../families/registry.ts';
import type { TimelineWidgetDeclaration } from '../../families/timeline/declaration.ts';
import { countryLook, inHourSpans, timelineModel } from '../../families/timeline/helpers.ts';
import { laneByKey, type TimelineLane, type TimelineProjection, type TimelineSideLabel, type TimelineSpan } from '../../families/timeline/model.ts';
import { formatDateOnly, formatScalar } from '../../families/valueFormat.ts';
import type { CivilWar, CivilWarSide, CivilWarsReply, SideExit } from '../../generated/dashboard';

export interface CivilWarsProjectionOptions {
  readonly showEnded: boolean;
  readonly firstHour: number;
  readonly branchFirstHour: number;
  readonly surface: string;
  readonly selected: readonly string[] | null;
  readonly holders: Readonly<Record<string, readonly HolderRun[]>> | null;
  readonly membership: Readonly<Record<string, HourRanges>> | null;
  readonly countryReason: string | null;
}

export interface CivilWarLaneMeta {
  readonly kind: 'war' | 'side';
  readonly warId: string;
  readonly sideIndex?: number;
}

const WAR_LANE_HEIGHT = 44;
const SIDE_LANE_HEIGHT = 24;
const EXITS: ReadonlySet<unknown> = new Set(['annexedBySide', 'annexedBySideOtherwise', 'annexedByOther']);
const int = (value: unknown): value is number => Number.isSafeInteger(value);
const plural = (count: number, one: string, many: string) => `${formatScalar(count, { decimals: 0 })} ${count === 1 ? one : many}`;

function validSide(side: unknown, index: number, count: number, war: Record<string, unknown>, replyHour: number): boolean {
  return object(side)
    && int(side.slot) && side.slot >= 0
    && isHolderTags(side.tags)
    && (side.tags as readonly { hour: number }[]).every(entry => entry.hour <= replyHour)
    && int(side.firstHour) && int(side.lastHour)
    && (war.firstHour as number) <= side.firstHour && side.firstHour <= side.lastHour && side.lastHour <= (war.lastHour as number)
    && (side.parent === null || (int(side.parent) && side.parent >= 0 && side.parent < count && side.parent !== index))
    && typeof side.existedBefore === 'boolean'
    && (side.exit === null || (object(side.exit) && EXITS.has(side.exit.kind)
      && int(side.exit.hour) && int(side.exit.readHour)
      && side.exit.hour <= replyHour && side.exit.readHour <= replyHour
      && (side.exit.by === null || opaqueTag(side.exit.by))));
}

function acyclic(sides: readonly CivilWarSide[]): boolean {
  const state = new Uint8Array(sides.length);
  for (let start = 0; start < sides.length; start++) {
    const path: number[] = [];
    let at: number | null = start;
    while (at !== null && state[at] === 0) {
      state[at] = 1;
      path.push(at);
      at = sides[at]!.parent;
    }
    if (at !== null && state[at] === 1) return false;
    for (const index of path) state[index] = 2;
  }
  return true;
}

function validWar(war: unknown, ids: Set<string>, replyHour: number): boolean {
  if (!object(war) || typeof war.id !== 'string' || ids.has(war.id)
    || !int(war.firstHour) || !int(war.lastHour)
    || war.firstHour > war.lastHour || war.lastHour > replyHour
    || !isOrderedSpans(war.atWar)
    || war.atWar.some(span => span.firstHour < (war.firstHour as number) || span.lastHour > (war.lastHour as number))
    || !Array.isArray(war.sides) || !war.sides.length) return false;
  ids.add(war.id);
  const count = war.sides.length;
  const outcome = war.outcome;
  return war.sides.every((side, index) => validSide(side, index, count, war, replyHour))
    && acyclic(war.sides as CivilWarSide[])
    && (outcome === null || (object(outcome)
      && int(outcome.winner) && outcome.winner >= 0 && outcome.winner < count
      && int(outcome.hour) && int(outcome.readHour)
      && outcome.hour <= replyHour && outcome.readHour <= replyHour));
}

export function validateCivilWarsReply(value: unknown): CivilWarsReply {
  const ids = new Set<string>();
  if (!object(value) || typeof value.branchId !== 'string'
    || !int(value.hour)
    || !(value.nextChange === null || (int(value.nextChange) && value.nextChange > value.hour))
    || !isOrderedSpans(value.known)
    || !Array.isArray(value.warnings) || value.warnings.some(warning => typeof warning !== 'string')
    || !Array.isArray(value.wars) || !value.wars.every(war => validWar(war, ids, value.hour as number))) {
    throw new Error('The civil wars reply is malformed.');
  }
  return value as unknown as CivilWarsReply;
}

export function civilWarsProof(data: unknown, _requested: readonly [number, number], hour: number): readonly [number, number] | null {
  const reply = data as CivilWarsReply;
  const last = reply.nextChange === null ? Number.MAX_SAFE_INTEGER : reply.nextChange - 1;
  return reply.hour <= hour && hour <= last ? [reply.hour, last] : null;
}

export function sideTree(sides: readonly CivilWarSide[]): { index: number; depth: number }[] {
  const children = new Map<number, number[]>();
  const roots: number[] = [];
  sides.forEach((side, index) => {
    if (side.parent === null) {
      roots.push(index);
      return;
    }
    const list = children.get(side.parent);
    if (list) list.push(index);
    else children.set(side.parent, [index]);
  });
  const order: { index: number; depth: number }[] = [];
  const stack = roots.reverse().map(index => ({ index, depth: 1 }));
  while (stack.length) {
    const next = stack.pop()!;
    order.push(next);
    const below = children.get(next.index) ?? [];
    for (let at = below.length - 1; at >= 0; at--) stack.push({ index: below[at]!, depth: next.depth + 1 });
  }
  return order;
}

function fightingEnd(war: CivilWar): number {
  return war.atWar.at(-1)?.lastHour ?? war.lastHour;
}

function namedSide(war: CivilWar): number {
  const index = war.sides.findIndex(side => side.existedBefore);
  return index < 0 ? 0 : index;
}

function sideMatchesSelected(
  side: CivilWarSide,
  hour: number,
  selected: readonly string[],
  holders: Readonly<Record<string, readonly HolderRun[]>> | null,
  membership: Readonly<Record<string, HourRanges>> | null,
): boolean {
  const sideEnd = Math.min(side.lastHour, hour);
  const overlaps = (ranges: HourRanges | undefined): boolean =>
    !ranges || ranges.some(([lo, hi]) => lo <= sideEnd && hi >= side.firstHour);
  for (const key of selected) {
    if (!overlaps(membership?.[key])) continue;
    if (isSlotKey(key)) {
      if (countryFromKey(key) === side.slot) return true;
      continue;
    }
    const holderRuns = holders?.[key];
    if (holderRuns?.length) {
      for (let i = 0; i < side.tags.length; i++) {
        const entry = side.tags[i]!;
        const runFirst = Math.max(side.firstHour, entry.hour);
        const runLast = Math.min(sideEnd, (side.tags[i + 1]?.hour ?? Infinity) - 1);
        if (runFirst > runLast) continue;
        if (holderRuns.some(run => run.tag === entry.tag && run.firstHour <= runLast && run.lastHour >= runFirst)) {
          return true;
        }
      }
      continue;
    }
    for (let i = 0; i < side.tags.length; i++) {
      const entry = side.tags[i]!;
      const runFirst = Math.max(side.firstHour, entry.hour);
      const runLast = Math.min(sideEnd, (side.tags[i + 1]?.hour ?? Infinity) - 1);
      if (runFirst <= runLast && entry.tag === key) return true;
    }
  }
  return false;
}

interface ResolvedSide {
  readonly index: number;
  readonly side: CivilWarSide;
  readonly depth: number;
  readonly live: boolean;
  readonly labelHour: number;
  readonly tag: string | null;
  readonly state: RunState;
  readonly winner: boolean;
  readonly muted: boolean;
  readonly name: string;
  readonly flag: ReturnType<typeof unavailableArt> | ReturnType<typeof countryLook>['flag'];
  readonly colour: string;
}

function resolveSideLook(
  readView: BoundedReadView,
  tag: string | null,
  atHour: number,
  surface: string,
  fallbackOrdinal: number,
): { readonly name: string; readonly flag: ReturnType<typeof countryLook>['flag']; readonly colour: string } {
  if (tag === null) {
    return {
      name: 'Country not recorded',
      flag: unavailableArt('The country tag was not recorded at this hour'),
      colour: colour(surface, fallbackOrdinal),
    };
  }
  const look = countryLook(readView, tag, atHour);
  return {
    name: look.name,
    flag: look.flag,
    colour: look.colour ? readableColour(look.colour, surface) : colour(surface, fallbackOrdinal),
  };
}

function sideAside(side: ResolvedSide): string {
  if (side.winner) return 'Winner';
  if (side.side.exit) return `Annexed ${formatDateOnly(side.side.exit.hour)}`;
  if (side.live || side.state.kind === 'held') {
    return `Since ${formatDateOnly(side.side.firstHour)}`;
  }
  if (side.state.kind === 'ended') {
    return `Left ${formatDateOnly(side.state.at)}`;
  }
  return side.state.kind === 'unknown' ? `Last recorded ${formatDateOnly(side.state.lastSeen)}` : '';
}

function exitText(exit: SideExit, by: string): string {
  switch (exit.kind) {
    case 'annexedBySide': return by;
    case 'annexedBySideOtherwise': return `${by}, not by the civil war\u2019s own annexation`;
    case 'annexedByOther': return `${by}, which was not a side`;
  }
}

function headlinePair(sides: readonly ResolvedSide[], outcome: CivilWar['outcome']): readonly [ResolvedSide, ResolvedSide] | null {
  if (sides.length < 2) return null;
  if (outcome) {
    const winner = sides.find(s => s.index === outcome.winner) ?? sides[0]!;
    const rival = sides.find(s => s.index !== winner.index && s.side.existedBefore)
      ?? sides.find(s => s.index !== winner.index)!;
    return [winner, rival];
  }
  return [sides[0]!, sides[1]!];
}

function toSideLabel(side: ResolvedSide): TimelineSideLabel {
  return {
    name: side.name,
    flag: side.flag,
    flagTag: side.tag ?? '?',
    strip: [],
  };
}

export const civilWarsProjection: TimelineProjection<CivilWarsProjectionOptions> = registerFamilyProjection<TimelineProjection<CivilWarsProjectionOptions>>({
  family: 'timeline',
  kind: 'civil-wars',

  nextChange(readView) {
    return readView.panel<CivilWarsReply>('main')?.nextChange ?? Infinity;
  },

  project(readView, opt) {
    const hour = readView.hour;
    const reply = readView.panel<CivilWarsReply>('main');
    const base = { hour, firstHour: opt.firstHour, labelWidth: 340, outcomeWidth: 230 };
    if (opt.countryReason) {
      return timelineModel({ ...base, empty: opt.countryReason, emptyKind: 'unavailable' });
    }
    if (!reply) return timelineModel(base);
    const nextChangeAt = reply.nextChange ?? Infinity;
    const unread = unknownHours([], reply.known, opt.branchFirstHour, reply.hour);
    const notes: string[] = [];

    if (!reply.known.length && !reply.wars.length) {
      return timelineModel({ ...base, empty: 'Civil wars were not recorded in this campaign', emptyKind: 'unavailable', warnings: reply.warnings, nextChangeAt });
    }

    const entries: {
      readonly war: CivilWar;
      readonly ongoing: boolean;
      readonly state: RunState;
      readonly sides: readonly ResolvedSide[];
      readonly titleTag: string | null;
      readonly titleName: string;
      readonly winnerName: string | null;
    }[] = [];

    let matchedTotal = 0;
    let ongoingCount = 0;
    let unnamedCount = 0;

    for (const war of reply.wars) {
      if (war.firstHour > reply.hour) continue;
      if (opt.selected && !war.sides.some(side => sideMatchesSelected(side, reply.hour, opt.selected!, opt.holders, opt.membership))) {
        continue;
      }
      const state = runState(war.atWar, reply.hour, unread);
      const ongoing = state.kind === 'held' || state.kind === 'unknown';
      if ((ongoing ? hour : fightingEnd(war)) < opt.firstHour) continue;
      matchedTotal++;
      if (ongoing) ongoingCount++;
      if (!ongoing && !opt.showEnded) continue;

      const resolvedSides: ResolvedSide[] = sideTree(war.sides).map(({ index, depth }, ordinal) => {
        const side = war.sides[index]!;
        const live = side.lastHour >= reply.hour && ongoing;
        const labelHour = live ? hour : Math.min(side.lastHour, hour);
        const tag = tagAt(side.tags, labelHour);
        if (tag === null) unnamedCount++;
        const look = resolveSideLook(readView, tag, labelHour, opt.surface, ordinal);
        const winner = war.outcome?.winner === index;
        return {
          index,
          side,
          depth,
          live,
          labelHour,
          tag,
          state: runState([side], reply.hour, unread),
          winner,
          muted: !!war.outcome && !winner,
          ...look,
        };
      });

      const titleTag = tagAt(war.sides[namedSide(war)]!.tags, war.firstHour);
      if (titleTag === null) unnamedCount++;
      const titleName = titleTag === null ? 'Country not recorded' : countryLook(readView, titleTag, war.firstHour).name;
      const winnerSide = war.outcome ? war.sides[war.outcome.winner] : undefined;
      const winnerTag = war.outcome && winnerSide ? tagAt(winnerSide.tags, war.outcome.hour) : null;
      const winnerName = war.outcome
        ? (winnerTag === null ? 'Country not recorded' : countryLook(readView, winnerTag, war.outcome.hour).name)
        : null;

      entries.push({ war, ongoing, state, sides: resolvedSides, titleTag, titleName, winnerName });
    }

    entries.sort((a, b) =>
      Number(b.ongoing) - Number(a.ongoing)
      || b.war.firstHour - a.war.firstHour
      || (a.war.id < b.war.id ? -1 : a.war.id > b.war.id ? 1 : 0));

    if (unnamedCount > 0) {
      notes.push(`${plural(unnamedCount, 'side has', 'sides have')} no recorded country tag at the displayed hour.`);
    }

    const summaryText = `${plural(matchedTotal, 'civil war', 'civil wars')} · ${formatScalar(ongoingCount, { decimals: 0 })} ongoing`;

    if (!entries.length) {
      const empty = opt.selected && !matchedTotal
        ? 'No civil wars for these countries'
        : matchedTotal > 0
          ? 'No ongoing civil wars'
          : inHourSpans(unread, reply.hour)
            ? 'Civil wars not recorded at this date'
            : 'No civil wars yet';
      return timelineModel({
        ...base,
        summaryText,
        empty,
        ...(inHourSpans(unread, reply.hour) && !matchedTotal ? { emptyKind: 'unavailable' as const } : {}),
        unobserved: unread,
        warnings: reply.warnings,
        notes,
        nextChangeAt,
      });
    }

    const lanes: TimelineLane[] = [];
    for (const entry of entries) {
      const { war, ongoing, state, sides, titleName, winnerName } = entry;
      const warKey = `cw:${war.id}`;
      const pair = headlinePair(sides, war.outcome);
      const topColor = pair ? pair[0].colour : (sides[0]?.colour ?? colour(opt.surface, 0));
      let bottomColor = pair ? pair[1].colour : colour(opt.surface, 1);
      if (bottomColor === topColor) bottomColor = colour(opt.surface, 1);
      const endHour = ongoing ? hour : Math.min(fightingEnd(war), hour);
      const span: TimelineSpan = {
        id: 'war',
        firstHour: war.firstHour,
        lastHour: endHour,
        color: topColor,
        halves: [topColor, bottomColor],
        endCap: !ongoing && war.atWar.length > 0,
        open: ongoing,
        startBy: startUnseen(war.firstHour, opt.branchFirstHour, unread),
        endUnseen: !ongoing && state.kind === 'ended' && state.lastSeen !== null,
      };
      const outcome = war.outcome && winnerName
        ? { mark: 'bar' as const, text: `${winnerName} won`, detail: formatDateOnly(war.outcome.hour) }
        : ongoing
          ? { mark: 'ongoing' as const, text: 'Ongoing', detail: plural(sides.length, 'side', 'sides') }
          : {
              mark: 'ended' as const,
              text: 'Outcome unknown',
              detail: formatDateOnly(state.kind === 'ended' ? state.at : war.lastHour),
            };
      const title = `Civil war in ${titleName}`;
      const extraSides = Math.max(0, sides.length - 2);
      lanes.push({
        key: warKey,
        kind: 'item',
        label: title,
        height: WAR_LANE_HEIGHT,
        ...(pair ? {
          sides: [toSideLabel(pair[0]), toSideLabel(pair[1])],
          sidesBelowLabel: true,
          sidesSeparator: 'v',
          ...(extraSides > 0 ? { extraSides } : {}),
        } : {
          subtitle: outcome.text,
        }),
        outcome,
        expandable: true,
        defaultOpen: ongoing,
        spans: [span],
        auditValue: `${war.id}|${title}|${outcome.text} ${outcome.detail ?? ''}`.trim(),
        meta: { kind: 'war', warId: war.id } satisfies CivilWarLaneMeta,
      });

      for (const side of sides) {
        const aside = sideAside(side);
        const sideOpen = side.live;
        lanes.push({
          key: `${warKey}:s:${side.index}`,
          kind: 'item',
          depth: side.depth,
          parentKey: warKey,
          height: SIDE_LANE_HEIGHT,
          barHeight: 10,
          label: side.name,
          ...(aside ? { aside } : {}),
          flag: side.flag,
          flagTag: side.tag ?? '?',
          spans: [{
            id: `s:${side.index}`,
            firstHour: side.side.firstHour,
            lastHour: sideOpen ? hour : Math.min(side.side.lastHour, hour),
            color: side.colour,
            open: sideOpen,
            startBy: startUnseen(side.side.firstHour, opt.branchFirstHour, unread),
            endUnseen: !sideOpen && side.state.kind === 'ended' && side.state.lastSeen !== null,
          }],
          auditValue: `${side.tag ?? `slot:${side.side.slot}`}|${aside}`,
          meta: { kind: 'side', warId: war.id, sideIndex: side.index } satisfies CivilWarLaneMeta,
        });
      }
    }

    return timelineModel({
      ...base,
      lanes,
      summaryText,
      unobserved: unread,
      warnings: reply.warnings,
      notes,
      nextChangeAt,
    });
  },

  tooltip(model, hit, readView) {
    const reply = readView.panel<CivilWarsReply>('main');
    const lane = laneByKey(model, hit.laneKey);
    const meta = lane?.meta as CivilWarLaneMeta | undefined;
    const war = meta && reply?.wars.find(item => item.id === meta.warId);
    if (!lane || !war || !reply) return lane ? { title: lane.label, hour: hit.hour ?? model.hour, items: [] } : null;

    const at = hit.hour ?? model.hour;
    const nameAt = (tag: string | null, when: number) => tag === null ? 'Country not recorded' : countryLook(readView, tag, when).name;

    if (meta.kind === 'war') {
      const items: { label: string; value?: string; missingReason?: string }[] = [
        { label: 'Started', value: formatDateOnly(war.firstHour) },
      ];
      if (war.outcome) {
        const winnerSide = war.sides[war.outcome.winner];
        const winnerTag = winnerSide ? tagAt(winnerSide.tags, war.outcome.hour) : null;
        items.push({ label: 'Outcome', value: `${nameAt(winnerTag, war.outcome.hour)} won (${formatDateOnly(war.outcome.hour)})` });
      } else if (lane.outcome) {
        items.push({ label: 'Outcome', value: `${lane.outcome.text}${lane.outcome.detail ? ` · ${lane.outcome.detail}` : ''}` });
      }
      for (const { index } of sideTree(war.sides)) {
        const side = war.sides[index]!;
        const labelHour = Math.min(side.lastHour, model.hour);
        const tag = tagAt(side.tags, labelHour);
        const status = war.outcome?.winner === index
          ? 'Winner'
          : side.exit
            ? `Annexed ${formatDateOnly(side.exit.hour)}`
            : side.lastHour >= reply.hour
              ? 'Active'
              : `Left ${formatDateOnly(side.lastHour)}`;
        items.push({ label: nameAt(tag, labelHour), value: status });
      }
      return {
        title: lane.label,
        subtitle: `${formatDateOnly(war.firstHour)} – ${lane.outcome?.mark === 'ongoing' ? 'ongoing' : formatDateOnly(fightingEnd(war))}`,
        hour: at,
        items,
      };
    }

    const side = meta.sideIndex !== undefined ? war.sides[meta.sideIndex] : undefined;
    if (!side) return { title: lane.label, hour: at, items: [] };
    const hoveredHour = Math.min(Math.max(at, side.firstHour), Math.min(side.lastHour, model.hour));
    const tag = tagAt(side.tags, hoveredHour);
    const items: { label: string; value?: string; missingReason?: string }[] = [
      tag === null
        ? { label: 'Tag', missingReason: 'Country tag not recorded at this hour' }
        : { label: 'Tag', value: tag },
    ];
    if (side.parent !== null) {
      const parentSide = war.sides[side.parent];
      const parentTag = parentSide ? tagAt(parentSide.tags, side.firstHour) : null;
      items.push({ label: 'Broke from', value: `${nameAt(parentTag, side.firstHour)} on ${formatDateOnly(side.firstHour)}` });
    } else {
      items.push({ label: 'Since', value: formatDateOnly(side.firstHour) });
    }
    if (side.exit) {
      const byName = side.exit.by === null ? 'Country not recorded' : nameAt(side.exit.by, side.exit.hour);
      items.push({ label: 'Annexed by', value: exitText(side.exit, byName) });
      items.push({ label: 'Annexed', value: formatDateOnly(side.exit.hour) });
    } else {
      items.push({ label: 'Status', value: side.lastHour >= reply.hour ? 'Still a side' : `Left ${formatDateOnly(side.lastHour)}` });
    }
    if (war.outcome?.winner === meta.sideIndex) {
      items.push({ label: 'Outcome', value: 'Winner' });
    }
    return {
      title: lane.label,
      subtitle: 'Civil war side',
      hour: at,
      items,
    };
  },
});

export const civilWarsDeclaration: TimelineWidgetDeclaration<CivilWarsProjectionOptions> = {
  family: 'timeline',
  kind: 'civil-wars',
  label: 'Civil wars',
  subject: 'civil wars',
  emptyCampaign: 'Load a campaign to see its civil wars.',
  icon: 'flag-torn',
  countries: { mode: 'multi', content: 'history', allowSlots: true },
  jumpToDate: true,
  keyByBranch: true,
  sources: ctx => ({
    owner: `civil-wars:${ctx.track.source.id}:${ctx.track.source.revision}:${ctx.branch.id}`,
    entries: [{
      name: 'main',
      source: ctx.track.source,
      branch: ctx.branch.id,
      offset: ctx.track.axisOffset,
      read: {
        shape: 'branch-to-hour',
        coverage: { mode: 'population', form: 'all' },
        product: 'civil-wars',
        followQuery: null,
        params: { branchId: ctx.branch.id },
        buildSpanParams: (params, span) => ({ ...params, hour: span.lastHour }),
        provenInterval: civilWarsProof,
        dynamicKeys: ['hour'],
        validate: validateCivilWarsReply,
      },
    }],
  }),
  projection: civilWarsProjection,
  projectionOptions: (ctx, extra) => ({
    showEnded: ctx.options.ended !== 'hidden',
    firstHour: extra.extent[0],
    branchFirstHour: ctx.branch.firstHour,
    surface: extra.surface,
    selected: ctx.countries.all ? null : ctx.countries.tags,
    holders: ctx.countries.holders,
    membership: ctx.countries.membership,
    countryReason: ctx.countries.all ? null : ctx.countries.reason,
  }),
};

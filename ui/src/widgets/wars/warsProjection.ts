import { colour, readableColour } from '../../charts/palette.ts';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import { flagChoiceArt } from '../../families/art/provider.ts';
import { unavailableArt } from '../../families/art/types.ts';
import type { ArtHandle, ArtResult, BoundedReadView } from '../../families/contract.ts';
import { registerFamilyProjection } from '../../families/registry.ts';
import type { TimelineWidgetDeclaration } from '../../families/timeline/declaration.ts';
import { inHourSpans, timelineModel } from '../../families/timeline/helpers.ts';
import { laneByKey, type TimelineLane, type TimelineProjection, type TimelineSideLabel, type TimelineSpan } from '../../families/timeline/model.ts';
import { formatDateOnly } from '../../families/valueFormat.ts';
import type { WarObservation, WarParticipantRun, WarTimeline, WarTimelineReply } from '../../generated/dashboard';

export type WarsView = 'gantt' | 'countries';

export interface WarsOptions {
  readonly view: WarsView;
  readonly colourBy: 'war' | 'side';
  readonly firstHour: number;
  readonly surface: string;
}

export const SURRENDER_FLAG: ArtHandle = { kind: 'sprite', key: 'GFX_surrender_tiny_flag', frame: 0 };
const DAY = 24;
const WAR_LANE = 36;
const ROW = 21;
const AGAINST = 18;

const object = (value: unknown): value is Record<string, unknown> => value != null && typeof value === 'object';

export function validateWarTimeline(data: unknown): WarTimelineReply {
  const ok = object(data) && Number.isInteger(data.hour) && (data.nextChange === null || (Number.isInteger(data.nextChange) && (data.nextChange as number) > (data.hour as number)))
    && Array.isArray(data.wars) && Array.isArray(data.unobserved) && Array.isArray(data.warnings) && typeof data.available === 'boolean'
    && data.wars.every(war => object(war) && typeof war.id === 'string' && Number.isInteger(war.firstHour) && Number.isInteger(war.lastHour)
      && (war.lastHour as number) <= (data.hour as number) && Array.isArray(war.participants) && Array.isArray(war.observations) && Array.isArray(war.capitulations)
      && war.participants.every(run => object(run) && typeof run.tag === 'string' && Number.isInteger(run.firstHour) && (run.lastHour as number) <= (data.hour as number))
      && war.capitulations.every(item => object(item) && typeof item.tag === 'string' && (item.hour as number) <= (data.hour as number)));
  if (!ok) throw new Error('The war timeline reply is malformed.');
  return data as WarTimelineReply;
}

export function warTimelineProof(data: unknown, _requested: readonly [number, number], hour: number): readonly [number, number] | null {
  const reply = data as WarTimelineReply;
  const last = reply.nextChange === null ? Number.MAX_SAFE_INTEGER : reply.nextChange - 1;
  return reply.hour <= hour && hour <= last ? [reply.hour, last] : null;
}

type Side = 'attackers' | 'defenders';

function latestObservation(war: WarTimeline): WarObservation | undefined {
  return war.observations.at(-1);
}

export function sideOf(run: WarParticipantRun, observation: WarObservation | undefined): Side {
  if (run.side === 'attacker') return 'attackers';
  if (run.side === 'defender') return 'defenders';
  const viewpointAttacks = observation?.attackersAreViewpoint ?? true;
  return (run.side === 'viewpoint') === viewpointAttacks ? 'attackers' : 'defenders';
}

interface WarSides {
  readonly attackers: WarParticipantRun[];
  readonly defenders: WarParticipantRun[];
  readonly leads: readonly [string | null, string | null];
}

export function warSides(war: WarTimeline): WarSides {
  const observation = latestObservation(war);
  const attackers: WarParticipantRun[] = [];
  const defenders: WarParticipantRun[] = [];
  for (const run of war.participants) (sideOf(run, observation) === 'attackers' ? attackers : defenders).push(run);
  const named: readonly string[] = observation?.representative ?? [];
  const lead = (runs: readonly WarParticipantRun[]) => {
    if (observation && runs.some(run => run.tag === observation.viewpointTag)) return observation.viewpointTag;
    const earliest = runs.reduce((least, run) => Math.min(least, run.firstHour), Infinity);
    const first = runs.filter(run => run.firstHour === earliest);
    return first.find(run => named.includes(run.tag))?.tag ?? first[0]?.tag ?? null;
  };
  const first = (runs: WarParticipantRun[], tag: string | null) => runs.sort((a, b) => Number(b.tag === tag) - Number(a.tag === tag) || a.firstHour - b.firstHour);
  first(attackers, lead(attackers));
  first(defenders, lead(defenders));
  return { attackers, defenders, leads: [lead(attackers), lead(defenders)] };
}

function countryAt(readView: BoundedReadView, tag: string, hour: number): { readonly name: string; readonly flag: ArtResult; readonly colour: string | null } {
  const who = readView.who('country', tag, Math.min(hour, readView.hour));
  const flag = flagChoiceArt(readView, who.flagChoice, 'panel');
  return {
    name: who.name.status === 'ready' ? who.name.value : tag,
    flag,
    colour: who.colour?.status === 'ready' ? who.colour.value : null,
  };
}

function duration(first: number, last: number): string {
  const days = Math.max(1, Math.round((last - first + 1) / DAY));
  if (days < 60) return `${days} ${days === 1 ? 'day' : 'days'}`;
  const months = Math.round(days / 30.4);
  if (months < 24) return `${months} months`;
  return `${Math.round(months / 12)} years`;
}

interface Ending {
  readonly ongoing: boolean;
  readonly loser: Side | null;
  readonly text: string;
  readonly detail: string;
  readonly mark: 'ongoing' | 'ended' | 'flag';
}

function endingOf(war: WarTimeline, sides: WarSides, hour: number, name: (tag: string) => string): Ending {
  if (war.lastHour >= hour) return { ongoing: true, loser: null, text: 'Ongoing', detail: duration(war.firstHour, hour), mark: 'ongoing' };
  if (war.endedBy) return { ongoing: false, loser: null, text: 'Last recorded', detail: formatDateOnly(war.lastHour), mark: 'ended' };
  const capitulated = new Set(war.capitulations.map(item => item.tag));
  const fallen = (runs: readonly WarParticipantRun[], lead: string | null) => lead !== null && capitulated.has(lead) && runs.length > 0;
  const lastCap = war.capitulations.at(-1);
  if (fallen(sides.defenders, sides.leads[1]) && lastCap) {
    return { ongoing: false, loser: 'defenders', text: `${name(sides.leads[1]!)} capitulated`, detail: formatDateOnly(lastCap.hour), mark: 'flag' };
  }
  if (fallen(sides.attackers, sides.leads[0]) && lastCap) {
    return { ongoing: false, loser: 'attackers', text: `${name(sides.leads[0]!)} capitulated`, detail: formatDateOnly(lastCap.hour), mark: 'flag' };
  }
  return { ongoing: false, loser: null, text: 'Ended', detail: formatDateOnly(war.lastHour + 1), mark: 'ended' };
}

function sideLabel(readView: BoundedReadView, runs: readonly WarParticipantRun[], lead: string | null, at: number, dim: boolean): TimelineSideLabel {
  const leadLook = lead ? countryAt(readView, lead, at) : null;
  const others = [...new Set(runs.filter(run => run.tag !== lead && run.lastHour >= at).map(run => run.tag))];
  return {
    name: leadLook?.name ?? 'Country not recorded',
    flag: leadLook?.flag ?? unavailableArt('The side has no lead country recorded'),
    flagTag: lead ?? '?',
    strip: others.map(tag => ({ flag: countryAt(readView, tag, at).flag, tag })),
    ...(dim ? { dim: true } : {}),
  };
}

function sideColour(readView: BoundedReadView, tag: string | null, at: number, surface: string, fallback: number): string {
  const look = tag ? countryAt(readView, tag, at) : null;
  return look?.colour ? readableColour(look.colour, surface) : colour(surface, fallback);
}

export interface WarLaneMeta {
  readonly warId: string;
  readonly tag?: string;
}

function warLanes(readView: BoundedReadView, reply: WarTimelineReply, opt: WarsOptions): TimelineLane[] {
  const hour = readView.hour;
  const lanes: TimelineLane[] = [];
  for (const war of reply.wars) {
    const sides = warSides(war);
    const at = Math.min(war.lastHour, hour);
    const name = (tag: string) => countryAt(readView, tag, at).name;
    const ending = endingOf(war, sides, reply.hour, name);
    const open = ending.ongoing;
    const lastHour = open ? hour : war.lastHour;
    const halves: readonly [string, string] = [sideColour(readView, sides.leads[0], at, opt.surface, 0), sideColour(readView, sides.leads[1], at, opt.surface, 1)];
    const key = `war:${war.id}`;
    const dates = `${war.startedBy ? 'By ' : ''}${formatDateOnly(war.firstHour)} – ${open ? 'ongoing' : formatDateOnly(war.lastHour)}`;
    const span: TimelineSpan = {
      id: 'war', firstHour: war.firstHour, lastHour, color: halves[0], halves, open, startBy: war.startedBy, endUnseen: war.endedBy && !open,
      marks: war.capitulations.map(item => ({ hour: item.hour, art: readView.art(SURRENDER_FLAG), label: `${name(item.tag)} capitulated` })),
    };
    lanes.push({
      key, kind: 'item', label: `${name(sides.leads[0] ?? '?')} – ${name(sides.leads[1] ?? '?')}`, subtitle: dates, height: WAR_LANE,
      sides: [sideLabel(readView, sides.attackers, sides.leads[0], at, ending.loser === 'attackers'), sideLabel(readView, sides.defenders, sides.leads[1], at, ending.loser === 'defenders')],
      outcome: { mark: ending.mark, text: ending.text, detail: ending.detail, ...(ending.mark === 'flag' ? { flag: readView.art(SURRENDER_FLAG) } : {}) },
      expandable: true, defaultOpen: open, spans: [span],
      auditValue: `${war.id}|${dates}|${ending.text} ${ending.detail}`,
      meta: { warId: war.id } satisfies WarLaneMeta,
    });
    const capitulated = new Map(war.capitulations.map(item => [item.tag, item.hour]));
    const row = (runs: readonly WarParticipantRun[], index: number): TimelineLane => {
      const tag = runs[0]!.tag;
      const look = countryAt(readView, tag, Math.min(runs.at(-1)!.lastHour, hour));
      const late = runs[0]!.firstHour > war.firstHour;
      const fell = capitulated.get(tag);
      const fill = look.colour ? readableColour(look.colour, opt.surface) : colour(opt.surface, index + 2);
      return {
        key: `${key}:${index}`, kind: 'item', depth: 1, parentKey: key, height: ROW, barHeight: 10,
        label: look.name, ...(late ? { aside: `Joined ${formatDateOnly(runs[0]!.firstHour)}` } : {}),
        flag: look.flag, flagTag: tag,
        spans: runs.map((run, at) => {
          const held = run.lastHour >= reply.hour && open;
          const last = at === runs.length - 1;
          return {
            id: `${index}:${at}`, firstHour: run.firstHour, lastHour: held ? hour : run.lastHour, open: held, color: fill,
            startBy: run.joinedBy, endUnseen: run.leftBy && !held,
            ...(last && fell !== undefined && fell <= hour ? { marks: [{ hour: fell, art: readView.art(SURRENDER_FLAG), label: `${look.name} capitulated` }] } : {}),
          };
        }),
        auditValue: `${tag}|${runs.map(run => `${run.firstHour}-${run.lastHour >= reply.hour && open ? 'open' : run.lastHour}`).join(',')}`,
        meta: { warId: war.id, tag } satisfies WarLaneMeta,
      };
    };
    const byCountry = (runs: readonly WarParticipantRun[]) => {
      const grouped = new Map<string, WarParticipantRun[]>();
      for (const run of runs) (grouped.get(run.tag) ?? grouped.set(run.tag, []).get(run.tag)!).push(run);
      return [...grouped.values()];
    };
    const attackers = byCountry(sides.attackers);
    const defenders = byCountry(sides.defenders);
    attackers.forEach((runs, index) => lanes.push(row(runs, index)));
    if (defenders.length) {
      lanes.push({ key: `${key}:against`, kind: 'heading', depth: 1, parentKey: key, height: AGAINST, label: 'Against', spans: [], auditValue: 'Against' });
    }
    defenders.forEach((runs, index) => lanes.push(row(runs, attackers.length + index)));
  }
  return lanes;
}

function countryLanes(readView: BoundedReadView, reply: WarTimelineReply, opt: WarsOptions): TimelineLane[] {
  const hour = readView.hour;
  const byTag = new Map<string, { runs: { war: WarTimeline; run: WarParticipantRun; side: Side; ordinal: number }[] }>();
  reply.wars.forEach((war, ordinal) => {
    const observation = latestObservation(war);
    for (const run of war.participants) {
      const entry = byTag.get(run.tag) ?? byTag.set(run.tag, { runs: [] }).get(run.tag)!;
      entry.runs.push({ war, run, side: sideOf(run, observation), ordinal });
    }
  });
  const lanes: TimelineLane[] = [];
  for (const [tag, entry] of byTag) {
    const look = countryAt(readView, tag, hour);
    const atWar = entry.runs.filter(item => item.run.lastHour >= reply.hour && item.war.lastHour >= reply.hour).length;
    const subtitle = atWar ? `${atWar} ${atWar === 1 ? 'war' : 'wars'} ongoing` : `${entry.runs.length} ${entry.runs.length === 1 ? 'war' : 'wars'}`;
    lanes.push({
      key: `country:${tag}`, kind: 'item', label: look.name, subtitle, flag: look.flag, flagTag: tag,
      spans: entry.runs.map(({ war, run, side, ordinal }, index) => {
        const held = run.lastHour >= reply.hour && war.lastHour >= reply.hour;
        return {
          id: `${war.id}:${index}`, firstHour: run.firstHour, lastHour: held ? hour : run.lastHour, open: held,
          color: opt.colourBy === 'side' ? colour(opt.surface, side === 'attackers' ? 0 : 1) : colour(opt.surface, ordinal),
          startBy: run.joinedBy, endUnseen: run.leftBy && !held, meta: { warId: war.id, tag } satisfies WarLaneMeta,
        };
      }),
      auditValue: `${tag}|${subtitle}`,
    });
  }
  return lanes;
}

export const warsProjection: TimelineProjection<WarsOptions> = registerFamilyProjection<TimelineProjection<WarsOptions>>({
  family: 'timeline',
  kind: 'wars',

  nextChange(readView) {
    return readView.panel<WarTimelineReply>('main')?.nextChange ?? Infinity;
  },

  project(readView, opt) {
    const hour = readView.hour;
    const reply = readView.panel<WarTimelineReply>('main');
    const firstShown = reply?.wars.reduce((least, war) => Math.min(least, war.firstHour), Infinity) ?? Infinity;
    const base = { hour, firstHour: Number.isFinite(firstShown) ? Math.max(opt.firstHour, firstShown) : opt.firstHour, labelWidth: 300 };
    if (!reply) return timelineModel(base);
    const nextChangeAt = reply.nextChange ?? Infinity;
    if (!reply.available) return timelineModel({ ...base, empty: 'Wars were not recorded in this campaign', emptyKind: 'unavailable', warnings: reply.warnings, nextChangeAt });
    if (!reply.wars.length) {
      const unread = inHourSpans(reply.unobserved, reply.hour);
      return timelineModel({ ...base, empty: unread ? 'Wars not recorded at this date' : 'No wars yet', ...(unread ? { emptyKind: 'unavailable' as const } : {}), warnings: reply.warnings, nextChangeAt });
    }
    const ongoing = reply.wars.filter(war => war.lastHour >= reply.hour).length;
    const summaryText = `${reply.wars.length} ${reply.wars.length === 1 ? 'war' : 'wars'} · ${ongoing} ongoing`;
    const lanes = opt.view === 'countries' ? countryLanes(readView, reply, opt) : warLanes(readView, reply, opt);
    return timelineModel({ ...base, lanes, summaryText, unobserved: reply.unobserved, warnings: reply.warnings, nextChangeAt, ...(opt.view === 'gantt' ? { outcomeWidth: 200 } : {}) });
  },

  tooltip(model, hit, readView) {
    const reply = readView.panel<WarTimelineReply>('main');
    const lane = laneByKey(model, hit.laneKey);
    const meta = lane?.meta as WarLaneMeta | undefined;
    const war = meta && reply?.wars.find(item => item.id === meta.warId);
    if (!lane || !war) return lane ? { title: lane.label, hour: hit.hour ?? model.hour, items: [] } : null;
    const sides = warSides(war);
    const at = Math.min(war.lastHour, model.hour);
    const names = (runs: readonly WarParticipantRun[]) => [...new Set(runs.map(run => run.tag))].map(tag => {
      const first = runs.find(run => run.tag === tag)!;
      const name = countryAt(readView, tag, at).name;
      return first.firstHour > war.firstHour ? `${name} (joined ${formatDateOnly(first.firstHour)})` : name;
    }).join(', ');
    const name = war.observations.at(-1)?.name;
    return {
      title: lane.label,
      subtitle: `${formatDateOnly(war.firstHour)} – ${war.lastHour >= model.hour ? 'ongoing' : formatDateOnly(war.lastHour)}${name && !name.known ? ' · name not recorded' : name?.known ? ` · ${name.text}` : ''}`,
      hour: hit.hour ?? model.hour,
      items: [
        { label: `Attackers (${new Set(sides.attackers.map(run => run.tag)).size})`, value: names(sides.attackers) || 'Not recorded' },
        { label: `Defenders (${new Set(sides.defenders.map(run => run.tag)).size})`, value: names(sides.defenders) || 'Not recorded' },
        ...(lane.outcome ? [{ label: 'Outcome', value: `${lane.outcome.text} ${lane.outcome.detail ?? ''}`.trim() }] : []),
      ],
    };
  },
}, 'gantt');

registerFamilyProjection(warsProjection, 'countries');

export const warsDeclaration: TimelineWidgetDeclaration<WarsOptions> = {
  family: 'timeline',
  kind: 'wars',
  label: 'Wars timeline',
  subject: 'wars',
  emptyCampaign: 'Load a campaign to see its wars.',
  icon: 'shield',
  countries: { mode: 'single' },
  jumpToDate: true,
  keyByBranch: true,
  sources: ctx => ({
    owner: `wars:${ctx.track.source.id}:${ctx.track.source.revision}:${ctx.branch.id}:${ctx.countryTag ?? ''}`,
    entries: [{
      name: 'main',
      source: ctx.track.source,
      branch: ctx.branch.id,
      offset: ctx.track.axisOffset,
      read: {
        shape: 'branch-to-hour',
        coverage: { mode: 'keyed', keys: ctx.countryTag ? [ctx.countryTag] : [] },
        product: 'war-timeline',
        followQuery: null,
        params: { branchId: ctx.branch.id, viewpoint: ctx.countryTag ? countryFromKey(ctx.countryTag) : null },
        buildSpanParams: (params, span) => ({ ...params, hour: span.lastHour }),
        provenInterval: warTimelineProof,
        dynamicKeys: ['hour'],
        validate: validateWarTimeline,
      },
    }],
  }),
  projection: warsProjection,
  projectionOptions: (ctx, extra) => ({
    view: ctx.options.view === 'countries' ? 'countries' : 'gantt',
    colourBy: ctx.options.colour === 'side' ? 'side' : 'war',
    firstHour: extra.extent[0],
    surface: extra.surface,
  }),
};

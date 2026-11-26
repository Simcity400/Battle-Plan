import { liveKey } from '../../charts/live.ts';
import { CHART_MEASURE_COLOURS } from '../../charts/palette.ts';
import { firstHourOfMonth, gameDate, monthIndex } from '../../dashboard/calendar.ts';
import { subjectHandle } from '../../dashboard/chosen.ts';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import type { BoundedReadView, FamilyPlanContext } from '../../families/contract.ts';
import { registerFamilyProjection } from '../../families/registry.ts';
import type { TimelineWidgetDeclaration } from '../../families/timeline/declaration.ts';
import { countryLook, timelineModel } from '../../families/timeline/helpers.ts';
import {
  laneByKey,
  spanAt,
  type TimelineHeader,
  type TimelineLane,
  type TimelineMeterItem,
  type TimelineProjection,
  type TimelineSpan,
} from '../../families/timeline/model.ts';
import { formatDateOnly, formatScalar, formatShare } from '../../families/valueFormat.ts';
import type {
  CareerBattle,
  LeaderCareerReply,
  LeaderHourReply,
  LeaderSelection,
  LeaderTrait,
} from '../../generated/dashboard';
import { validateLeaderHour } from './dossierProjection.ts';
import { experienceWhole } from './experience.ts';

export const TIMELINE_VIEW = 'timeline';

export interface CareerTimelineOptions {
  readonly reference: string | null;
  readonly firstHour: number;
  readonly surface: string;
}

const PARTIAL = 'Battle counts are incomplete: some battles were not recorded.';
const PARTIAL_MONTH = 'Part of this month was not recorded: battles then are not counted.';
const FLEET_NOTE = 'Fleet commands cover fleets the recording names this leader for.';
const COMMAND_COLOURS = ['#394d72', '#3b64a8', '#44587e', '#355c96'] as const;
const HELD_DIAMOND = '#5c8df6';
const GAINED_DIAMOND = '#e5a43b';
const TRAIT_CLASSES = ['Held before', 'Gained', 'Gained'] as const;

export function validateLeaderCareer(input: unknown): LeaderCareerReply {
  const reply = input as LeaderCareerReply | null;
  if (
    !reply
    || typeof reply !== 'object'
    || !Number.isSafeInteger(reply.hour)
    || !(reply.nextChange === null || (Number.isSafeInteger(reply.nextChange) && reply.nextChange > reply.hour))
    || !Array.isArray(reply.runs)
    || !Array.isArray(reply.traitGains)
    || !Array.isArray(reply.traits)
    || !Array.isArray(reply.commands)
    || !Array.isArray(reply.battles)
    || !Array.isArray(reply.observed)
    || !Array.isArray(reply.warnings)
  ) {
    throw new Error('The leader career reply is malformed.');
  }
  return reply;
}

export function leaderCareerProof(data: unknown, _requested: readonly [number, number], hour: number): readonly [number, number] | null {
  const reply = data as LeaderCareerReply;
  const last = reply.nextChange === null ? Number.MAX_SAFE_INTEGER : reply.nextChange - 1;
  return reply.hour <= hour && hour <= last ? [reply.hour, last] : null;
}

function leaderParams(ctx: FamilyPlanContext): { readonly reference: string } | { readonly selection: LeaderSelection } | null {
  const chosen = String(ctx.options.leader ?? '');
  if (chosen) {
    const reference = subjectHandle(ctx.track.source.metadata.campaignKey, chosen);
    return reference === null ? null : { reference };
  }
  if (ctx.countries.all) return { selection: { kind: 'all' } };
  return ctx.countries.tags.length ? { selection: { kind: 'countries', countries: ctx.countries.tags.map(countryFromKey) } } : null;
}

const traitName = (trait: LeaderTrait | undefined) => trait?.name ?? trait?.token ?? 'Trait name not recorded';

interface BattleMonth {
  readonly first: number;
  readonly last: number;
  count: number;
  won: number;
  lost: number;
  unresolved: number;
}

type Span = readonly [first: number, last: number];

function intersect(a: readonly Span[], b: readonly Span[]): Span[] {
  const out: Span[] = [];
  let i = 0;
  let j = 0;
  while (i < a.length && j < b.length) {
    const first = Math.max(a[i]![0], b[j]![0]);
    const last = Math.min(a[i]![1], b[j]![1]);
    if (first <= last) out.push([first, last]);
    if (a[i]![1] < b[j]![1]) i++;
    else j++;
  }
  return out;
}

function coveredHours(spans: readonly Span[], first: number, last: number): number {
  let hours = 0;
  for (const [a, b] of intersect(spans, [[first, last]])) hours += b - a + 1;
  return hours;
}

function gapsWithin(observed: readonly Span[], first: number, last: number): { firstHour: number; lastHour: number }[] {
  const gaps: { firstHour: number; lastHour: number }[] = [];
  let cursor = first;
  for (const [a, b] of observed) {
    if (b < cursor) continue;
    if (a > last) break;
    if (a > cursor) gaps.push({ firstHour: cursor, lastHour: a - 1 });
    cursor = Math.max(cursor, b + 1);
  }
  if (cursor <= last) gaps.push({ firstHour: cursor, lastHour: last });
  return gaps;
}

function battleMonths(battles: readonly CareerBattle[], firstHour: number, hour: number): BattleMonth[] {
  const byMonth = new Map<number, BattleMonth>();
  for (const battle of battles) {
    if (battle.start > hour || battle.read > hour) continue;
    const date = gameDate(battle.start);
    const key = date ? monthIndex(date) : battle.start;
    const start = date ? firstHourOfMonth(key) : battle.start;
    const end = date ? firstHourOfMonth(key + 1) - 1 : battle.start;
    if (end < firstHour) continue;
    let entry = byMonth.get(key);
    if (!entry) {
      entry = { first: Math.max(firstHour, start), last: Math.min(hour, end), count: 0, won: 0, lost: 0, unresolved: 0 };
      byMonth.set(key, entry);
    }
    entry.count++;
    if (battle.won === true) entry.won++;
    else if (battle.won === false) entry.lost++;
    else entry.unresolved++;
  }
  return [...byMonth.entries()].sort((a, b) => a[0] - b[0]).map(([, entry]) => entry);
}

function buildHeader(
  readView: BoundedReadView,
  hourReply: LeaderHourReply | undefined,
  career: LeaderCareerReply,
  reference: string,
): TimelineHeader {
  const hour = readView.hour;
  const role = hourReply?.role ?? career.runs.at(-1)?.role ?? null;
  const name = hourReply?.name ?? career.name ?? 'Name not recorded';
  const ownerTag = role?.owner ?? null;
  const ownerLook = ownerTag ? countryLook(readView, ownerTag, hour) : null;
  const rank = role ? (role.rank ?? `Rank ${role.category}`) : 'Leader';
  const skillVal = hourReply ? hourReply.skill : (career.runs.at(-1)?.skill ?? null);
  const maxSkill = role?.maximumSkill ?? null;
  const skillText = skillVal === null
    ? 'skill not recorded'
    : `★ ${skillVal}${maxSkill === null ? '' : ` / ${maxSkill}`}`;
  const fought = hourReply?.fought ?? career.battles.length;
  const won = hourReply?.won ?? career.battles.filter(b => b.won === true).length;
  const lost = hourReply?.lost ?? career.battles.filter(b => b.won === false).length;
  const decided = won + lost;
  const unresolved = Math.max(0, fought - decided);
  const winRate = decided ? formatShare(won / decided) : '—';
  const portraitHour = hourReply?.firstSeenHour ?? career.portraitHour ?? hour;

  return {
    portrait: readView.art({ kind: 'portrait', leader: reference, hour: portraitHour }),
    portraitName: name,
    title: name,
    subtitle: `${rank} · ${skillText}`,
    ...(ownerLook ? { flag: ownerLook.flag, flagTag: ownerLook.tag } : {}),
    kpis: [
      { value: formatScalar(won, { decimals: 0 }), label: 'won' },
      { value: formatScalar(lost, { decimals: 0 }), label: 'lost' },
      { value: formatScalar(unresolved, { decimals: 0 }), label: 'unresolved' },
      { value: formatScalar(fought, { decimals: 0 }), label: 'fought' },
      { value: winRate, label: 'won' },
    ],
    compactKpi: `${formatScalar(won, { decimals: 0 })} won · ${formatScalar(lost, { decimals: 0 })} lost · ${winRate}`,
  };
}

function buildMeters(readView: BoundedReadView, hourReply: LeaderHourReply | undefined) {
  const training = hourReply?.progress?.training;
  if (training === undefined) return undefined;
  if (training === null) {
    return { title: 'Traits in training', items: [], empty: 'Trait training was not recorded at this date' };
  }
  const live = readView.live('hour');
  const items: TimelineMeterItem[] = training.map(trait => {
    const raw = live?.get(liveKey(trait.key, 'experience'));
    const rawText = raw === undefined ? null : String(raw);
    const required = trait.requiredRaw;
    const figure = rawText === null ? 'Not recorded' : experienceWhole(rawText, required);
    let fraction: number | null = null;
    if (rawText !== null && required !== null) {
      const denom = Number(required);
      fraction = denom > 0 ? Math.max(0, Math.min(1, Number(rawText) / denom)) : 1;
    }
    return {
      key: trait.key,
      label: trait.name ?? trait.token ?? 'Trait name not recorded',
      figure,
      fraction,
    };
  });
  return { title: 'Traits in training', items, empty: 'No traits in training' };
}

export const careerTimelineProjection: TimelineProjection<CareerTimelineOptions> = registerFamilyProjection<TimelineProjection<CareerTimelineOptions>>({
  family: 'timeline',
  kind: 'general-career',

  nextChange(readView) {
    return readView.panel<LeaderCareerReply>('career')?.nextChange ?? Infinity;
  },

  project(readView, opt) {
    const hour = readView.hour;
    const base = { hour, firstHour: opt.firstHour, labelWidth: 72, axisPosition: 'top' as const };
    const refused = readView.refused('hour') ?? readView.refused('career');
    if (refused) return timelineModel({ ...base, empty: refused, emptyKind: 'unavailable' });

    const career = readView.panel<LeaderCareerReply>('career');
    const hourReply = readView.panel<LeaderHourReply>('hour');
    if (!career) return timelineModel(base);

    const nextChangeAt = career.nextChange ?? Infinity;
    const warnings = [...new Set([...career.warnings, ...(hourReply?.warnings ?? [])])];
    const reference = career.reference ?? hourReply?.reference ?? null;

    if (reference === null) {
      const missing = career.missing ?? hourReply?.missing;
      return timelineModel({
        ...base,
        empty: missing ?? (opt.reference === null ? 'No generals yet.' : 'This leader is not serving at this date.'),
        emptyKind: missing ? 'unavailable' : 'empty',
        warnings,
        nextChangeAt,
      });
    }

    if (career.missing && !career.runs.length) {
      return timelineModel({
        ...base,
        empty: career.missing,
        emptyKind: 'unavailable',
        warnings,
        nextChangeAt,
      });
    }

    const serving = hourReply?.missing === null ? hourReply : undefined;
    const header = buildHeader(readView, serving, career, reference);
    const meters = buildMeters(readView, serving);

    const commandSpans: TimelineSpan[] = career.commands
      .filter(cmd => cmd.firstHour <= hour && cmd.lastHour >= opt.firstHour)
      .map((cmd, index) => {
        const label = cmd.name ?? `${cmd.kind === 'fleet' ? 'Fleet' : 'Formation'} ${cmd.formation}`;
        const open = cmd.lastHour >= career.hour;
        return {
          id: `cmd:${cmd.formation}:${cmd.firstHour}`,
          firstHour: Math.max(opt.firstHour, cmd.firstHour),
          lastHour: open ? hour : Math.min(hour, cmd.lastHour),
          color: COMMAND_COLOURS[index % COMMAND_COLOURS.length]!,
          label,
          open,
          meta: { kind: 'command', name: label, firstHour: cmd.firstHour, lastHour: cmd.lastHour, open },
        };
      });

    const through = (last: number) => (last === career.hour ? hour : Math.min(last, hour));
    const observed: Span[] = career.observed.map(([first, last]) => [first, through(last)] as const);
    const service: Span[] = career.runs.map(run => [run.firstHour, through(run.lastHour)] as const);
    const inService = intersect(observed, service);
    const months = battleMonths(career.battles, opt.firstHour, hour);
    const maxMonthly = months.reduce((top, m) => Math.max(top, m.won, m.lost), 1);
    const battleSpans: TimelineSpan[] = months.flatMap((m, index) => {
      const pieces = intersect(inService, [[m.first, m.last]]);
      const partial = coveredHours(observed, m.first, m.last) < m.last - m.first + 1;
      const meta = { kind: 'battle', ...m, partial };
      return pieces.map(([first, last]) => ({
        id: `bat:${index}:${first}`,
        firstHour: first,
        lastHour: last,
        color: CHART_MEASURE_COLOURS.supplyOk,
        open: false,
        histogram: { positive: m.won, negative: m.lost, max: maxMonthly },
        meta,
      }));
    });

    const gainsAt = new Map<number, { readonly name: string; readonly gainClass: number }[]>();
    for (const [at, idx, gainClass] of career.traitGains) {
      if (at > hour || at < opt.firstHour) continue;
      const list = gainsAt.get(at);
      const entry = { name: traitName(career.traits[idx]), gainClass };
      if (list) list.push(entry);
      else gainsAt.set(at, [entry]);
    }
    const traitSpans: TimelineSpan[] = [...gainsAt].sort((a, b) => a[0] - b[0]).map(([at, entries]) => {
      const gained = entries.filter(entry => entry.gainClass !== 0);
      const colour = gained.length ? GAINED_DIAMOND : HELD_DIAMOND;
      return {
        id: `trait:${at}`,
        firstHour: at,
        lastHour: at,
        color: colour,
        open: false,
        diamond: { color: colour, ...(gained.length ? { text: gained.map(entry => entry.name).join(', ') } : {}) },
        meta: { kind: 'trait', hour: at, entries },
      };
    });

    const lanes: TimelineLane[] = [
      {
        key: 'command',
        kind: 'item',
        label: 'Command',
        height: 28,
        barHeight: 18,
        spans: commandSpans,
        auditValue: `Command|${commandSpans.length}`,
      },
      {
        key: 'battles',
        kind: 'item',
        label: 'Battles',
        height: 40,
        barHeight: 34,
        spans: battleSpans,
        auditValue: `Battles|${months.length}`,
      },
      {
        key: 'traits',
        kind: 'item',
        label: 'Traits',
        height: 28,
        barHeight: 14,
        spans: traitSpans,
        auditValue: `Traits|${traitSpans.length}`,
      },
    ];

    const notes: string[] = [];
    if (hourReply?.missing) notes.push(hourReply.missing);
    const partial = (serving?.battlesPartialFrom !== null && serving?.battlesPartialFrom !== undefined && serving.battlesPartialFrom <= hour)
      || (career.battlesPartialFrom !== null && career.battlesPartialFrom <= hour);
    if (partial) notes.push(PARTIAL);
    const roleKind = serving?.role?.kind ?? career.runs.at(-1)?.role.kind;
    if (roleKind === 2) notes.push(FLEET_NOTE);

    return timelineModel({
      ...base,
      header,
      ...(meters ? { meters } : {}),
      lanes,
      unobserved: gapsWithin(observed, opt.firstHour, hour),
      warnings,
      notes,
      nextChangeAt,
    });
  },

  tooltip(model, hit) {
    const lane = laneByKey(model, hit.laneKey);
    if (!lane) return null;
    const at = hit.hour ?? model.hour;
    const span = spanAt(lane, hit);
    const meta = span?.meta as Record<string, unknown> | undefined;
    if (meta?.kind === 'command') {
      return {
        title: String(meta.name),
        subtitle: 'Command',
        hour: at,
        items: [
          { label: 'Since', value: formatDateOnly(Number(meta.firstHour)) },
          { label: 'Until', value: meta.open ? 'Ongoing' : formatDateOnly(Number(meta.lastHour)) },
        ],
      };
    }
    if (meta?.kind === 'battle') {
      return {
        title: `${formatScalar(Number(meta.count), { decimals: 0 })} battles`,
        subtitle: `${formatDateOnly(Number(meta.first))} – ${formatDateOnly(Number(meta.last))}`,
        hour: at,
        items: [
          { label: 'Won', value: formatScalar(Number(meta.won), { decimals: 0 }) },
          { label: 'Lost', value: formatScalar(Number(meta.lost), { decimals: 0 }) },
          { label: 'Unresolved', value: formatScalar(Number(meta.unresolved), { decimals: 0 }) },
        ],
        ...(meta.partial ? { note: PARTIAL_MONTH } : {}),
      };
    }
    if (meta?.kind === 'trait') {
      const entries = meta.entries as readonly { readonly name: string; readonly gainClass: number }[];
      const date = formatDateOnly(Number(meta.hour));
      const classOf = (entry: { readonly gainClass: number }) => TRAIT_CLASSES[entry.gainClass] ?? 'Recorded';
      return entries.length === 1
        ? { title: entries[0]!.name, subtitle: 'Trait', hour: at, items: [{ label: classOf(entries[0]!), value: date }] }
        : { title: `${formatScalar(entries.length, { decimals: 0 })} traits`, subtitle: date, hour: at, items: entries.map(entry => ({ label: entry.name, value: classOf(entry) })) };
    }
    return {
      title: model.header?.title ?? lane.label,
      subtitle: lane.label,
      hour: at,
      items: [{ label: lane.label, value: 'No recorded entry at this date' }],
    };
  },
}, TIMELINE_VIEW);

export const careerTimelineDeclaration: TimelineWidgetDeclaration<CareerTimelineOptions> = {
  family: 'timeline',
  kind: 'general-career',
  label: 'General career',
  subject: 'general',
  emptyCampaign: 'Load a campaign to see a general’s career.',
  icon: 'history',
  countries: { mode: 'multi', allowSlots: true },
  jumpToDate: true,
  keyByBranch: true,
  sources: ctx => {
    const params = leaderParams(ctx);
    const owner = `general-career-timeline:${ctx.track.source.id}:${ctx.track.source.revision}:${ctx.branch.id}:${JSON.stringify(params)}`;
    if (!params) {
      const unavailable = String(ctx.options.leader ?? '')
        ? 'The saved general is from another campaign. Choose one in widget settings.'
        : null;
      return { owner, entries: [], ...(unavailable ? { unavailable } : {}) };
    }
    const key = 'reference' in params ? params.reference : JSON.stringify(params.selection);
    const common = { source: ctx.track.source, branch: ctx.branch.id, offset: ctx.track.axisOffset };
    return {
      owner,
      entries: [
        {
          name: 'hour',
          ...common,
          read: {
            shape: 'point' as const,
            channel: 'panel' as const,
            coverage: { mode: 'keyed' as const, keys: [key] },
            product: 'unit-leader-hour' as const,
            followQuery: { panel: 'unit-leader-hour', branchId: ctx.branch.id, ...params },
            params: { branchId: ctx.branch.id, ...params },
            validate: validateLeaderHour,
          },
        },
        {
          name: 'career',
          ...common,
          read: {
            shape: 'branch-to-hour' as const,
            coverage: { mode: 'keyed' as const, keys: [key] },
            product: 'unit-leader-career' as const,
            followQuery: null,
            params: { branchId: ctx.branch.id, ...params },
            buildSpanParams: (base, span) => ({ ...base, hour: span.lastHour }),
            provenInterval: leaderCareerProof,
            dynamicKeys: ['hour'],
            validate: validateLeaderCareer,
          },
        },
      ],
    };
  },
  projection: careerTimelineProjection,
  projectionOptions: (ctx, extra) => {
    const params = leaderParams(ctx);
    return {
      reference: params && 'reference' in params ? params.reference : null,
      firstHour: extra.extent[0],
      surface: extra.surface,
    };
  },
};

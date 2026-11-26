import { countryFromKey } from '../../dashboard/countryKey.ts';
import { contentKey } from '../../dashboard/options.ts';
import type { BoundedReadView } from '../../families/contract.ts';
import type { AuxProductSpec, CountryPolicy, FamilyPlanContext, SourceEntry, SourcePlan } from '../../families/plan.ts';
import type { WarContributionGroup, WarContributionsReply, WarObservation, WarsReply, WarTimeline } from '../../generated/dashboard';
import { observationAt, validateWarContributions, validateWarsReply, warContributions } from '../wars/warsData.ts';
import { WAR_NOT_STARTED, chosenWar, readHourFor, unobservedAt, warChipStateAt, warEligibilityAt, warNextChangeAt } from './warContributionData.ts';

export type WarSide = 'viewpoint' | 'opposing';

export interface WarChoiceOptions {
  readonly wars: WarsReply | null;
  readonly saved: string | null;
}

export type ChosenGroup =
  | { readonly kind: 'shown'; readonly war: WarTimeline; readonly reply: WarContributionsReply; readonly observation: WarObservation; readonly group: WarContributionGroup; readonly sides: readonly [WarSide, WarSide] }
  | { readonly kind: 'loading' }
  | { readonly kind: 'blank'; readonly empty: string; readonly emptyKind: 'empty' | 'unavailable' };

export const NO_WAR = 'Choose a war to see who fought it.';

export const warCountries: CountryPolicy = { mode: 'single', choose: 'Choose a country to see the wars it fought.' };

export const warsAux: AuxProductSpec<WarsReply> = {
  product: 'wars',
  params: (track, _options, country) => ({ branchId: track.branch.id, viewpoint: country === null ? null : countryFromKey(country) }),
  validate: validateWarsReply,
};

export function savedWar(options: Readonly<Record<string, unknown>>): string | null {
  return typeof options.war === 'string' && options.war ? contentKey(options.war) : null;
}

export function attackersFirst<T>(observation: WarObservation | null, viewpoint: T, opposing: T): readonly [T, T] {
  return observation?.attackersAreViewpoint === false ? [opposing, viewpoint] : [viewpoint, opposing];
}

export function leadOf(group: WarContributionGroup, side: WarSide): string {
  return (side === 'viewpoint' ? group.viewpointSide[0] : group.opposingSide[0]) ?? '';
}

export function sideCountries(group: WarContributionGroup, side: WarSide): string[] {
  return [...new Set(side === 'viewpoint' ? group.viewpointSide : group.opposingSide)];
}

function replyFor(readView: BoundedReadView, war: WarTimeline, hour: number): WarContributionsReply | undefined {
  const going = observationAt(war, hour) !== null;
  const raw = readView.panel(going ? 'live' : 'ended');
  if (raw === undefined) return undefined;
  const reply = validateWarContributions(raw);
  return reply.hour >= war.firstHour && reply.hour <= war.lastHour && reply.hour <= hour ? reply : undefined;
}

export function warContributionNextChange(hour: number, options: WarChoiceOptions): number {
  return warNextChangeAt(options.wars ?? undefined, options.saved, hour);
}

export function chosenGroup(readView: BoundedReadView, options: WarChoiceOptions): ChosenGroup {
  const hour = readView.hour;
  if (!options.wars) return { kind: 'loading' };
  if (!options.wars.available) return { kind: 'blank', empty: 'Wars are not in this recording.', emptyKind: 'unavailable' };
  if (unobservedAt(options.wars, hour)) {
    return { kind: 'blank', empty: 'This war’s contributions were not recorded at this time.', emptyKind: 'unavailable' };
  }
  const war = chosenWar(options.wars, options.saved, hour);
  if (!war) {
    const saved = options.wars.wars.find(item => item.id === options.saved);
    return { kind: 'blank', empty: saved ? WAR_NOT_STARTED : NO_WAR, emptyKind: 'empty' };
  }
  if (!observationAt(war, readHourFor(war, hour))) {
    return { kind: 'blank', empty: 'This war’s contributions were not recorded at this time.', emptyKind: 'unavailable' };
  }
  const reply = replyFor(readView, war, hour);
  if (!reply) return { kind: 'loading' };
  const observation = observationAt(war, reply.hour);
  const group = observation ? warContributions(reply.contributions, observation) : null;
  if (!observation || !group || (group.reason && !group.measures.length)) return { kind: 'blank', empty: group?.reason ?? 'This war’s contributions were not recorded at this time.', emptyKind: 'unavailable' };
  return { kind: 'shown', war, reply, observation, group, sides: attackersFirst<WarSide>(observation, 'viewpoint', 'opposing') };
}

function endedRead(wars: WarsReply | undefined, saved: string | null, hour: number): { readonly at: number | null; readonly valid: readonly [number, number] } {
  const state = warChipStateAt(wars, saved, hour);
  if (!state.war || observationAt(state.war, hour) !== null) return { at: null, valid: state.valid };
  const target = readHourFor(state.war, hour);
  return observationAt(state.war, target) !== null ? { at: target, valid: state.valid } : { at: null, valid: state.valid };
}

function staticHour(ctx: FamilyPlanContext<WarsReply>): number | null {
  const { time } = ctx.widget;
  if (time.mode === 'campaign') return ctx.lastRecordedHour ?? ctx.branch.lastHour;
  if (time.mode === 'pin') {
    const window = time.windows[ctx.track.id];
    return window ? ctx.branch.firstHour + window.lastOffset : null;
  }
  if (time.mode === 'range') return ctx.staticSpan ? ctx.staticSpan[1] : null;
  return null;
}

export function warContributionSources(ctx: FamilyPlanContext<WarsReply>): SourcePlan {
  const viewpoint = ctx.countryTag;
  const params = { branchId: ctx.branch.id, viewpoint: viewpoint === null ? null : countryFromKey(viewpoint) };
  const saved = savedWar(ctx.options);
  const owner = `war-contribution:${ctx.track.source.id}:${ctx.track.source.revision}:${ctx.branch.id}:${viewpoint ?? ''}:${saved ?? ''}`;
  if (!ctx.aux || !ctx.aux.available || !ctx.aux.wars.length) {
    return { owner, entries: [], allowEmptyFollow: true, valid: [-Infinity, Infinity] };
  }
  const entry = (name: string, read: SourceEntry['read']): SourceEntry => ({ name, source: ctx.track.source, branch: ctx.branch.id, offset: ctx.track.axisOffset, read });
  const live = entry('live', {
    shape: 'point', channel: 'panel', coverage: { mode: 'population', form: 'all' }, product: 'war-contributions',
    followQuery: { panel: 'war-contributions', ...params }, params, validate: validateWarContributions,
  });
  const ended = entry('ended', {
    shape: 'point', channel: 'followed', coverage: { mode: 'population', form: 'all' }, product: 'war-contributions', followQuery: null, params,
    buildExactParams: (base, hour) => {
      const read = endedRead(ctx.aux, saved, hour);
      return read.at === null ? null : { ...base, hour: read.at };
    },
    provenInterval: (_data, hour) => endedRead(ctx.aux, saved, hour).valid,
    idleInterval: hour => endedRead(ctx.aux, saved, hour).valid,
    preserveDisplayHour: true,
    validate: validateWarContributions,
  });
  const at = ctx.following ? (ctx.planHour ?? null) : staticHour(ctx);
  if (at !== null) {
    const eligibility = warEligibilityAt(ctx.aux, saved, at);
    if (eligibility.mode === 'none') return { owner, entries: [], allowEmptyFollow: true, valid: eligibility.valid };
    return { owner, entries: [eligibility.mode === 'live' ? live : ended], valid: eligibility.valid };
  }
  return { owner, entries: [live, ended] };
}

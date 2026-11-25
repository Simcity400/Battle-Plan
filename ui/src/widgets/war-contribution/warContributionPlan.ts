import { countryFromKey } from '../../dashboard/countryKey.ts';
import { contentKey } from '../../dashboard/options.ts';
import type { BoundedReadView } from '../../families/contract.ts';
import type { AuxProductSpec, CountryPolicy, FamilyPlanContext, SourcePlan } from '../../families/plan.ts';
import type { WarContributionGroup, WarContributionsReply, WarObservation, WarsReply, WarTimeline } from '../../generated/dashboard';
import { observationAt, validateWarContributions, validateWarsReply, warContributions } from '../wars/warsData.ts';
import { chosenWar, readHourFor } from './warContributionData.ts';

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

export function attackersFirst(group: WarContributionGroup): readonly [WarSide, WarSide] {
  const lead = group.participants.find(participant => participant.tag === group.viewpointSide[0]);
  return lead?.side === 'defender' ? ['opposing', 'viewpoint'] : ['viewpoint', 'opposing'];
}

export function leadOf(group: WarContributionGroup, side: WarSide): string {
  return (side === 'viewpoint' ? group.viewpointSide[0] : group.opposingSide[0]) ?? '';
}

function replyFor(readView: BoundedReadView, war: WarTimeline, hour: number): WarContributionsReply | undefined {
  const raw = readView.panel('live');
  if (raw === undefined || hour < war.firstHour) return undefined;
  const reply = validateWarContributions(raw);
  const at = observationAt(war, hour) || hour > war.lastHour ? readHourFor(war, hour) : war.firstHour;
  if (reply.hour > at) return undefined;
  return reply.hour === at ? reply : { ...reply, hour: at };
}

export function chosenGroup(readView: BoundedReadView, options: WarChoiceOptions): ChosenGroup {
  const hour = readView.hour;
  if (!options.wars) return { kind: 'loading' };
  if (!options.wars.available) return { kind: 'blank', empty: 'Wars are not in this recording.', emptyKind: 'unavailable' };
  const war = chosenWar(options.wars, options.saved, hour);
  if (!war) return { kind: 'blank', empty: NO_WAR, emptyKind: 'empty' };
  const reply = replyFor(readView, war, hour);
  if (!reply) return { kind: 'loading' };
  const observation = observationAt(war, reply.hour);
  const group = observation ? warContributions(reply.contributions, observation) : null;
  if (!observation || !group || group.reason) return { kind: 'blank', empty: group?.reason ?? 'This war’s contributions were not recorded at this time.', emptyKind: 'unavailable' };
  return { kind: 'shown', war, reply, observation, group, sides: attackersFirst(group) };
}

export function warContributionSources(ctx: FamilyPlanContext<WarsReply>): SourcePlan {
  const viewpoint = ctx.countryTag;
  const war = savedWar(ctx.options) ?? 'auto';
  const params = { branchId: ctx.branch.id, viewpoint: viewpoint === null ? null : countryFromKey(viewpoint), war };
  return {
    owner: `war-contribution:${ctx.track.source.id}:${ctx.track.source.revision}:${ctx.branch.id}:${viewpoint ?? ''}:${war}`,
    entries: [{
      name: 'live', source: ctx.track.source, branch: ctx.branch.id, offset: ctx.track.axisOffset,
      read: {
        shape: 'point', channel: 'panel', coverage: { mode: 'population', form: 'all' }, product: 'war-contributions',
        followQuery: { panel: 'war-contributions', ...params }, params, validate: validateWarContributions,
      },
    }],
  };
}

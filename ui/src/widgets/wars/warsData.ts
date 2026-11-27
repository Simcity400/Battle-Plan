import { runAt } from '../../charts/series.ts';
import { entryAt } from '../../dashboard/reads.ts';
import type { HourSpan, WarContributionGroup, WarContributionsReply, WarObservation, WarParticipantRun, WarSide, WarTimeline, WarsReply } from '../../generated/dashboard';

const object = (value: unknown): value is Record<string, unknown> => typeof value === 'object' && value != null;
const text = (value: unknown): value is string => typeof value === 'string';
const hour = (value: unknown) => Number.isSafeInteger(value);
const list = (value: unknown, check: (item: unknown) => boolean) => Array.isArray(value) && value.every(check);
const range = (value: unknown): value is HourSpan & Record<string, unknown> => object(value) && hour(value.firstHour) && hour(value.lastHour) && (value.firstHour as number) <= (value.lastHour as number);
const ordered = (ranges: readonly HourSpan[]) => ranges.every((item, index) => !index || ranges[index - 1].lastHour < item.firstHour);
const side = (value: unknown): value is WarSide => value === 'attacker' || value === 'defender' || value === 'viewpoint' || value === 'opponent';
const raw = (value: unknown) => text(value) && /^-?\d+$/.test(value);

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
      && (observation.contributionGroup === undefined || (Array.isArray(observation.contributionGroup)
        && observation.contributionGroup.length === 2 && observation.contributionGroup.every(tags => list(tags, text))))
      && object(observation.name) && text(observation.name.text) && typeof observation.name.known === 'boolean'
      && (observation.name.reason === null || text(observation.name.reason)) && (observation.name.key === null || text(observation.name.key)))
      && (war.observations as WarObservation[]).length > 0 && ordered(war.observations as WarObservation[])
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
  return value;
}

export function validateWarContributions(value: unknown): WarContributionsReply {
  if (!object(value) || !text(value.branchId) || !hour(value.hour) || typeof value.available !== 'boolean' || !list(value.warnings, text)
    || !(value.missing === null || text(value.missing))
    || !list(value.contributions, group => object(group) && list(group.viewpointSide, text) && list(group.opposingSide, text)
      && (group.reason === null || text(group.reason)) && list(group.participants, row => object(row) && text(row.tag) && side(row.side)
        && ['equipmentDamage', 'occupation', 'airDamage', 'strategicBombing', 'shipDamage', 'convoyDamage', 'casualties',
          'unknownCasualties', 'lendLeaseSent', 'lendLeaseReceived', 'score', 'participation'].every(key => raw(row[key])))))
    throw new Error('The war contributions reply is malformed.');
  return value as unknown as WarContributionsReply;
}

export function warContributions(groups: readonly WarContributionGroup[] | undefined, observation: WarObservation | null): WarContributionGroup | null {
  if (!groups || !observation) return null;
  const same = (a: readonly string[], b: readonly string[]) => a.length === b.length && a.every((tag, index) => tag === b[index]);
  const [viewpoint, opposing] = observation.contributionGroup ?? [observation.viewpointSide, observation.opposingSide];
  return groups.find(group => same(group.viewpointSide, viewpoint) && same(group.opposingSide, opposing)) ?? null;
}

export function observationAt(war: WarTimeline, at: number): WarObservation | null {
  return runAt(war.observations, Math.floor(at));
}

export function labelObservationAt(war: WarTimeline, at: number): WarObservation | null {
  return at > war.lastHour ? war.observations.at(-1) ?? null : observationAt(war, at);
}


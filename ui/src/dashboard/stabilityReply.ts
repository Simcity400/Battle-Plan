import type { CountryStabilityReply, ReadRun } from '../generated/dashboard';
import { readRunAt } from './reads.ts';
import { orderedRuns } from '../widgets/country-measure/measureData.ts';

const isCarried = (value: unknown): value is ReadRun => typeof value === 'object' && value != null
  && Number.isInteger((value as ReadRun).firstHour) && Number.isInteger((value as ReadRun).lastHour) && Number.isInteger((value as ReadRun).readHour)
  && (value as ReadRun).firstHour <= (value as ReadRun).lastHour && (value as ReadRun).readHour < (value as ReadRun).firstHour;

/** Checks a `country-stability` reply's shape: every run list, and the carried
 * stretches, in hour order and never overlapping; a carried read is earlier than
 * the hours it covers. The breakdown is checked by its user. */
export function validateStabilityReply(data: unknown): CountryStabilityReply {
  const reply = data as CountryStabilityReply;
  const ok = typeof reply === 'object' && reply != null && typeof reply.branchId === 'string'
    && Number.isInteger(reply.decimals) && reply.decimals >= 0 && reply.decimals <= 12
    && Array.isArray(reply.warnings) && reply.warnings.every(warning => typeof warning === 'string')
    && Array.isArray(reply.series) && reply.series.every(series => typeof series.tag === 'string'
      && orderedRuns(series.stability) && orderedRuns(series.warSupport) && orderedRuns(series.maxCommandPower)
      && typeof series.recorded === 'boolean' && Array.isArray(series.carried) && series.carried.every(isCarried)
      && series.carried.every((run, index) => index === 0 || series.carried[index - 1].lastHour < run.firstHour)
      && (series.breakdown === null || (typeof series.breakdown === 'object' && Number.isInteger(series.breakdown.hour))));
  if (!ok) throw new Error('The stability reply is malformed.');
  return reply;
}

/** The hour of the read behind a shown hour's values: an earlier read when the
 * hour is carried, otherwise the hour itself. */
export function readHourAt(carried: readonly ReadRun[], hour: number): number {
  return readRunAt(carried, hour, true);
}

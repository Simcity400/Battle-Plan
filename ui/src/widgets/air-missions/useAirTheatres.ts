import type { WidgetRuntime } from '../../dashboard/definitions';
import type { BoundTrack } from '../../dashboard/model';
import { useFollowedProduct } from '../../dashboard/products';
import { oneDay } from '../../dashboard/batch5Requests';
import type { AirMissionsParams, AirStatisticsParams } from '../../dashboard/batch5Params';
import type { AirTheatre } from './airMissionsData';

const validate = (value: unknown): readonly AirTheatre[] | null => {
  const theatres = (value as { theatres?: unknown } | null)?.theatres;
  if (theatres === undefined) return null;
  if (!Array.isArray(theatres) || !theatres.every(theatre => theatre && typeof theatre.id === 'string' && typeof theatre.name === 'string'
    && Array.isArray(theatre.regions) && theatre.regions.every((region: unknown) => typeof region === 'string')))
    throw new Error('The air theatres in this reply are malformed.');
  return theatres as AirTheatre[];
};

/** Theatre membership at a reached hour: undefined while loading, null when the recording
 * cannot establish it then. `hour` is the hour the held `theatres` answer. */
export interface AirTheatres { theatres?: readonly AirTheatre[] | null; hour?: number; error?: string; retry: () => void }

/** The air theatres and their regions at `hour`, from the product's one-hour view. Membership
 * follows the wings' mission regions, which change at any hour, so it is read at the hour
 * itself; while playback moves on, the last answer stays until the next one lands, and
 * values it filters are read at its own `hour`. */
export function useAirTheatres(runtime: WidgetRuntime, track: BoundTrack | undefined, product: 'air-missions' | 'air-statistics',
  params: AirMissionsParams | AirStatisticsParams | null, hour: number | null, active: boolean): AirTheatres {
  const at = hour === null || !track ? null : Math.max(track.branch.firstHour, Math.min(track.branch.lastHour, Math.floor(hour)));
  const request = track && params && at !== null ? { sourceId: track.source.id, revision: track.source.revision, product,
    params: oneDay(params, at) } : null;
  const read = useFollowedProduct<readonly AirTheatre[] | null>(runtime.client, request, ['firstHour', 'lastHour'], active, validate, track?.source);
  const answered = read.data === undefined || read.moment === undefined ? undefined : (JSON.parse(read.moment) as [number, number])[0];
  return { theatres: read.data, hour: answered, error: read.error, retry: read.retry };
}

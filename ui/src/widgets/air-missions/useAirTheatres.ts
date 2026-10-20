import type { WidgetRuntime } from '../../dashboard/definitions';
import type { BoundTrack } from '../../dashboard/model';
import { useProduct } from '../../dashboard/products';
import { oneDay } from '../../dashboard/batch5Requests';
import type { AirMissionsParams, AirStatisticsParams } from '../../dashboard/batch5Params';
import type { AirTheatre } from './airMissionsData';

/** Theatre membership dates by the hour asked: the reply names theatres and their regions
 * for a one-hour window only, so the widgets ask once per calendar day shown. */
const DAY_HOURS = 24;

const validate = (value: unknown): readonly AirTheatre[] | null => {
  const theatres = (value as { theatres?: unknown } | null)?.theatres;
  if (theatres === undefined) return null;
  if (!Array.isArray(theatres) || !theatres.every(theatre => theatre && typeof theatre.id === 'string' && typeof theatre.name === 'string'
    && Array.isArray(theatre.regions) && theatre.regions.every((region: unknown) => typeof region === 'string')))
    throw new Error('The air theatres in this reply are malformed.');
  return theatres as AirTheatre[];
};

/** The air theatres and their regions at the day of `hour`, from the product's one-hour view;
 * null while loading, or when the recording names none then. */
export function useAirTheatres(runtime: WidgetRuntime, track: BoundTrack | undefined, product: 'air-missions' | 'air-statistics',
  params: AirMissionsParams | AirStatisticsParams | null, hour: number | null, active: boolean): readonly AirTheatre[] | null {
  const day = hour === null || !track ? null : Math.max(track.branch.firstHour, Math.min(track.branch.lastHour, Math.floor(hour / DAY_HOURS) * DAY_HOURS));
  const request = track && params && day !== null ? { sourceId: track.source.id, revision: track.source.revision, product,
    params: oneDay(params, day) } : null;
  return useProduct<readonly AirTheatre[] | null>(runtime.client, request, active, validate, track?.source).data ?? null;
}

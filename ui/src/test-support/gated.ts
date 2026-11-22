import { boundCountryToken, type BoundCountry, type DatedCountrySelection } from '../dashboard/datedCountry.ts';
import type { FollowQuery, GatedHour, GatedParams, GatedTime, GatedWindow } from '../dashboard/regionTime';
import type { Gate } from '../dashboard/regionGate';
import type { WidgetTime } from '../dashboard/model';

export const gatedHour = (hour: number) => hour as GatedHour;
export const gatedWindow = (firstHour: number, lastHour: number): GatedWindow => ({ firstHour: gatedHour(firstHour), lastHour: gatedHour(lastHour) });
export const gatedParams = (params: unknown) => params as GatedParams;
export const followQuery = (query: Readonly<Record<string, unknown>>) => query as FollowQuery;
export const gatedTime = (time: Exclude<WidgetTime, { mode: 'campaign' }>) => time as GatedTime;
export const boundCountry = (selection: DatedCountrySelection) => selection as BoundCountry;
export const datedToken = (selection: DatedCountrySelection) => boundCountryToken(boundCountry(selection));

export function openGate(firstHour: number, lastHour: number): Pick<Gate, 'follow' | 'recorded' | 'within' | 'span' | 'hourAt'> {
  const inside = (hour: number | null | undefined) => hour != null && Number.isFinite(hour) && Math.floor(hour) >= firstHour && Math.floor(hour) <= lastHour;
  return {
    follow: query => query as FollowQuery | null,
    recorded: hour => inside(hour) ? gatedHour(Math.floor(hour!)) : null,
    within: hour => inside(hour) ? gatedHour(Math.floor(hour!)) : null,
    span: (first, last) => Math.max(first, firstHour) <= Math.min(last, lastHour) ? gatedWindow(Math.max(Math.floor(first), firstHour), Math.min(Math.floor(last), lastHour)) : null,
    hourAt: axis => inside(axis) ? gatedHour(Math.floor(axis)) : null,
  };
}

import { boundCountryToken, type BoundCountry, type DatedCountrySelection } from '../dashboard/datedCountry.ts';
import { checkedTime, type FollowQuery, type GatedHour, type GatedParams, type GatedTime, type GatedWindow, type PinTime } from '../dashboard/regionTime.ts';
import type { Gate, HeldCountry } from '../dashboard/regionGate';

export const gatedHour = (hour: number) => hour as GatedHour;
export const gatedWindow = (firstHour: number, lastHour: number): GatedWindow => ({ firstHour: gatedHour(firstHour), lastHour: gatedHour(lastHour) });
export const gatedParams = (params: unknown) => params as GatedParams;
export const followQuery = (query: Readonly<Record<string, unknown>>) => query as FollowQuery;
const anywhere = (id: string) => ({ id, branch: { firstHour: 0, lastHour: Number.MAX_SAFE_INTEGER, gaps: [] } });
export function gatedTime(time: PinTime): GatedTime {
  const made = checkedTime(time, (time.mode === 'pin' ? Object.keys(time.windows) : ['range']).map(anywhere), 'range').time;
  if (!made) throw new Error(`Not a time a gate makes: ${JSON.stringify(time)}`);
  return made;
}
export const heldCountry = (held: { tag: string; origin: number; slot: number; countryId: number; token?: string }) => ({ token: '', ...held }) as HeldCountry;
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

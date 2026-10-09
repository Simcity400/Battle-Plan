import { validatePlaceNamesReply } from './placeNamesData.ts';

export interface StateAt { hour: number; state: number }
export interface StateLabel { state: number; name: string | null; missing: string | null }
// Every wanted state and hour is retained; only the transport byte budget refuses a reply.
export const stateNameRequestOptions = { perRequest: Infinity, maxHours: Infinity } as const;
export const stateNameKey = (hour: number, state: number) => JSON.stringify([hour, String(state)]);
export const validateStateNamesReply = validatePlaceNamesReply;

import { contentKey, contentSet, setToken } from '../../dashboard/options.ts';
import type { OptionValue } from '../../dashboard/options.ts';

export interface RegionChoice {
  readonly picks: readonly number[];
  readonly hidden: readonly number[];
}

export const AUTOMATIC_REGIONS: RegionChoice = { picks: [], hidden: [] };

const PICK = 'r:';
const HIDE = 'h:';

function idOf(key: string, prefix: string): number | null {
  if (!key.startsWith(prefix)) return null;
  const id = Number(key.slice(prefix.length));
  return Number.isSafeInteger(id) && id >= 0 ? id : null;
}

export function readRegionChoice(value: OptionValue | undefined): RegionChoice {
  const picks: number[] = [];
  const hidden: number[] = [];
  for (const token of contentSet(value ?? '')) {
    const key = contentKey(token);
    const pick = idOf(key, PICK);
    if (pick !== null) {
      if (!picks.includes(pick)) picks.push(pick);
      continue;
    }
    const hide = idOf(key, HIDE);
    if (hide !== null && !hidden.includes(hide)) hidden.push(hide);
  }
  return { picks, hidden: hidden.filter(id => !picks.includes(id)) };
}

export function writeRegionChoice(choice: RegionChoice): string {
  return [...choice.picks.map(id => setToken(`${PICK}${id}`)), ...choice.hidden.map(id => setToken(`${HIDE}${id}`))].join(' ');
}

export function addRegion(choice: RegionChoice, id: number): RegionChoice {
  return { picks: [...choice.picks.filter(item => item !== id), id], hidden: choice.hidden.filter(item => item !== id) };
}

export function removeRegion(choice: RegionChoice, id: number): RegionChoice {
  if (choice.picks.includes(id)) return { picks: choice.picks.filter(item => item !== id), hidden: choice.hidden };
  return choice.hidden.includes(id) ? choice : { picks: choice.picks, hidden: [...choice.hidden, id] };
}

export function moveRegion(choice: RegionChoice, id: number, to: number): RegionChoice {
  const picks = choice.picks.filter(item => item !== id);
  const at = Math.max(0, Math.min(picks.length, Math.floor(to)));
  picks.splice(at, 0, id);
  return { picks, hidden: choice.hidden.filter(item => item !== id) };
}

export function regionParams(choice: RegionChoice): { readonly pinnedRegions?: readonly number[]; readonly hiddenRegions?: readonly number[] } {
  return {
    ...(choice.picks.length ? { pinnedRegions: choice.picks } : {}),
    ...(choice.hidden.length ? { hiddenRegions: choice.hidden } : {}),
  };
}

export function isAutomatic(choice: RegionChoice): boolean {
  return choice.picks.length === 0 && choice.hidden.length === 0;
}

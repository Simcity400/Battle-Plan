import type { FamilyNavTuple } from './keyboard.ts';

export type FamilyExpansion = Readonly<Record<string, boolean>>;

export const NO_EXPANSION: FamilyExpansion = Object.freeze({});

export interface ExpansionToggle {
  readonly next: FamilyExpansion;
  readonly open: boolean;
  readonly label: string;
}

export function toggleExpansion(
  expansion: FamilyExpansion,
  nav: readonly FamilyNavTuple[],
  key: string,
  exclusive = false,
): ExpansionToggle | null {
  const entry = nav.find(item => item[0] === key);
  if (!entry || entry[3] < 0) return null;
  const open = entry[3] === 0;
  const closed = open && exclusive ? Object.fromEntries(nav.filter(item => item[3] === 1).map(item => [item[0], false])) : {};
  return { next: { ...expansion, ...closed, [key]: open }, open, label: entry[1] };
}

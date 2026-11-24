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
): ExpansionToggle | null {
  const entry = nav.find(item => item[0] === key);
  if (!entry || entry[3] < 0) return null;
  const open = entry[3] === 0;
  return { next: { ...expansion, [key]: open }, open, label: entry[1] };
}

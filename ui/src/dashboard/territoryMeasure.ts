export type TerritoryMeasure = 'states' | 'provinces' | 'victoryPoints' | 'area';
export type TerritoryRule = 'controlled' | 'owned' | 'occupied';

export const MEASURES: readonly { value: TerritoryMeasure; label: string }[] = [
  { value: 'states', label: 'States' }, { value: 'provinces', label: 'Provinces' },
  { value: 'victoryPoints', label: 'Victory points' }, { value: 'area', label: 'Map area' },
];

export const RULES: readonly { value: TerritoryRule; label: string }[] = [
  { value: 'controlled', label: 'Controlled' }, { value: 'owned', label: 'Owned' }, { value: 'occupied', label: 'Occupied' },
];

export const AREA_UNIT = 'map cells';

export function measureOf(value: unknown): TerritoryMeasure {
  return MEASURES.find(measure => measure.value === value)?.value ?? 'states';
}

export function ruleOf(value: unknown): TerritoryRule {
  return value === 'owned' || value === 'occupied' ? value : 'controlled';
}

export function territorySeriesMeasure(measure: TerritoryMeasure, rule: TerritoryRule): string {
  return `${rule}${measure[0].toUpperCase()}${measure.slice(1)}`;
}

export function territoryNote(measure: TerritoryMeasure): string | undefined {
  return measure === 'area' ? 'Map area exaggerates territory near the poles.' : undefined;
}

export function territoryMeasureLabel(measure: TerritoryMeasure, rule: TerritoryRule): string {
  const name = MEASURES.find(item => item.value === measure)!.label;
  return `${RULES.find(item => item.value === rule)!.label} ${name.toLowerCase()}${measure === 'area' ? ` · ${AREA_UNIT}` : ''}`;
}

import { factoryCountOf, factoryMeasureLabel } from '../../dashboard/factoryMeasure.ts';
import { measureOf, ruleOf, territoryMeasureLabel, territoryNote } from '../../dashboard/territoryMeasure.ts';
import type { OptionValue } from '../../dashboard/options';
import type { FactionTotalsParams } from '../../generated/dashboard';

export const MEASURES = [{ value: 'factories', label: 'Factories' }, { value: 'territory', label: 'Territory' }] as const;

/** Both measures go straight to the shared product's existing adapters. */
export function factionStrengthSource(options: Readonly<Record<string, OptionValue>>): FactionTotalsParams['source'] {
  return options.measure === 'territory'
    ? { kind: 'territory', measure: measureOf(options.territoryMeasure), rule: ruleOf(options.rule) }
    : { kind: 'factories', mode: factoryCountOf(options.count), groups: [] };
}

export function factionStrengthLabel(source: FactionTotalsParams['source'], unit: string | null): string {
  return source.kind === 'territory' ? territoryMeasureLabel(source.measure, source.rule, unit) : factoryMeasureLabel(source.mode);
}

export function factionStrengthNote(source: FactionTotalsParams['source']): string | undefined {
  return source.kind === 'territory' ? territoryNote(source.measure) : undefined;
}

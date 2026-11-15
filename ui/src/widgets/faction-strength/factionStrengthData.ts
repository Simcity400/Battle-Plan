import { measureOf, ruleOf, territoryMeasureLabel, territoryNote, territorySeriesMeasure } from '../../dashboard/territoryMeasure.ts';
import type { OptionValue } from '../../dashboard/options';

export const MEASURES = [{ value: 'factories', label: 'Factories' }, { value: 'territory', label: 'Territory' }] as const;

/** Factory counts the game's way, each a declared measure of `country-factories` over all three groups. */
export const COUNT_MODES = [
  { value: 'free', label: 'Available', measure: 'free7', hint: 'Built less damaged, given away and withheld for consumer goods, contracts, special projects and imports' },
  { value: 'built', label: 'Built', measure: 'built7', hint: 'The game’s Total: every factory counted, damaged ones included' },
  { value: 'topBar', label: 'Top bar', measure: 'topBar7', hint: 'The top bar’s total: built less damaged and factories given away' },
] as const;

export interface FactionSource { kind: 'territory' | 'country-factories'; measure: string; label: string; note?: string }

/** The declared series and measure the chosen options sum over each faction. */
export function factionStrengthSource(options: Readonly<Record<string, OptionValue>>): FactionSource {
  if (options.measure === 'territory') {
    const measure = measureOf(options.territoryMeasure), rule = ruleOf(options.rule);
    return { kind: 'territory', measure: territorySeriesMeasure(measure, rule), label: territoryMeasureLabel(measure, rule), note: territoryNote(measure) };
  }
  const mode = COUNT_MODES.find(item => item.value === options.count) ?? COUNT_MODES[0];
  return { kind: 'country-factories', measure: mode.measure, label: `${mode.label} factories` };
}

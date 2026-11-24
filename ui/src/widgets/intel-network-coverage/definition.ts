import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';
import { breakdownOption, breakdownVariants, DRILL_OPTION } from '../../charts/drill/drillState.ts';

export const MEASURES = [{ value: 'covered', label: 'States covered' }, { value: 'strength', label: 'Strength' }, { value: 'strongest', label: 'Strongest state' }] as const;
export const PAIR = { id: 'pair', label: 'Pair history' };
export const OPTIONS: readonly OptionField[] = [
  breakdownOption([PAIR], 'Matrix'), DRILL_OPTION,
  { key: 'measure', section: 'options', label: 'Measure', kind: 'choice', default: 'covered', choices: MEASURES },
  { key: 'basis', section: 'options', label: 'Basis', kind: 'choice', default: 'controlled', visibleWhen: o => o.measure !== 'strongest',
    choices: [{ value: 'controlled', label: 'Controlled states' }, { value: 'core', label: 'Core states' }] },
  { key: 'mostCountries', section: 'options', label: 'Countries shown', kind: 'number', min: 2, max: Number.MAX_SAFE_INTEGER, step: 1, default: 12,
    visibleWhen: o => o.show !== 'pair' },
  { key: 'owner', section: 'options', label: 'Network owner', kind: 'content-choice', automatic: 'Choose a cell', default: '', visibleWhen: () => false },
  { key: 'target', section: 'options', label: 'Network target', kind: 'content-choice', automatic: 'Choose a cell', default: '', visibleWhen: () => false },
];
export const definition = {
  kind: 'intel-network-coverage', version: 1, title: 'Intel network coverage', icon: 'eye', category: 'Intelligence',
  description: 'Each country’s intel network in every other: states covered, strength and strongest state.',
  timeModes: ['campaign', 'follow', 'pin'], defaultTime: 'follow', scope: 'moment', data: true, history: true,
  countries: { max: Number.MAX_SAFE_INTEGER, ranked: true, all: true },
  minSize: { w: 4, h: 5 }, defaultSize: { w: 6, h: 9 }, options: OPTIONS, viewSwitch: { option: 'measure' },
  variants: breakdownVariants('Intel network coverage', OPTIONS), playsBackDated: true,
} as const satisfies WidgetMeta;

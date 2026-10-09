import { TOP_COUNTRIES } from '../../charts/race/options.ts';
import { partsRaceOptions } from '../../charts/race/partsRaceOptions.ts';
import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';
import { COUNT_MODES } from './factoriesData.ts';
import { breakdownOption } from '../../charts/drill/drillState.ts';

export const OPTIONS: readonly OptionField[] = [
  ...partsRaceOptions('type').map(field => field.key === 'show'
    ? breakdownOption([{ id: 'breakdown', label: 'Factory types' }], 'Factories') : field),
  { key: 'count', section: 'options', label: 'Count', hint: 'Free after withheld: what is left once damaged, consumer goods, trade, projects, licences, contracts and factories given away are taken out',
    kind: 'choice', default: 'free',
    choices: COUNT_MODES.map(mode => ({ value: mode.value, label: mode.label })) },
  { key: 'types', section: 'options', label: 'Factory types', hint: 'From the campaign’s own buildings', kind: 'content-set',
    automatic: 'All types', default: '' },
];

export const definition = {
  kind: 'factories', version: 1, title: 'Factories', icon: 'factory',
  description: 'Factory counts per country: a race re-ranked every hour, lines over time or one country by type.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'campaign', scope: 'history', data: true, history: true,
  minSize: { w: 4, h: 6 }, defaultSize: { w: 6, h: 12 },
  countries: { max: Number.MAX_SAFE_INTEGER, ranked: true },
  defaultCountries: TOP_COUNTRIES,
  options: OPTIONS,
} as const satisfies WidgetMeta;

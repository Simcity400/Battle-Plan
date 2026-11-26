import type { OptionField } from '../../dashboard/options';
import { breakdownOption } from '../drill/drillState.ts';
import { DATE_OVERLAY_OPTION, RACE_AXIS_OPTIONS, RACE_TIME_OPTIONS, raceViewOption } from './options.ts';

export function partsRaceOptions(part: string, breakdown = `By ${part}`): readonly OptionField[] {
  return [
    { key: 'drillOriginHour', section: 'options', label: 'Country selected at hour', kind: 'number',
      min: -0x8000_0000, max: 0x7fff_ffff, step: 1, default: null, visibleWhen: () => false },
    { key: 'holder', section: 'options', label: 'Selected country', kind: 'content-choice', automatic: '', default: '',
      visibleWhen: () => false },
    { ...raceViewOption(part), visibleWhen: options => options.show === 'main' },
    breakdownOption([{ id: 'breakdown', label: breakdown }], 'Country totals'),
    { key: 'split', section: 'options', label: `Split bars by ${part}`, kind: 'toggle', default: false,
      visibleWhen: options => options.show === 'main' && options.view === 'race' },
    { ...DATE_OVERLAY_OPTION, visibleWhen: options => options.show === 'main' && options.view === 'race' },
    { key: 'endLabels', section: 'options', label: 'Names at line ends', kind: 'toggle', default: true,
      visibleWhen: options => options.show === 'main' && options.view === 'lines' },
    ...[...RACE_TIME_OPTIONS, ...RACE_AXIS_OPTIONS].map(field => ({ ...field,
      visibleWhen: (options, countries) => field.visibleWhen?.({ ...options, view: options.show === 'breakdown' ? 'mix' : options.view }, countries) ?? true } satisfies OptionField)),
  ];
}

import type { OptionField } from '../../dashboard/options';
import { DATE_OVERLAY_OPTION, RACE_AXIS_OPTIONS, RACE_TIME_OPTIONS, raceViewOption } from './options.ts';

export function partsRaceOptions(part: string): readonly OptionField[] {
  return [
    { key: 'drillOriginHour', section: 'options', label: 'Country selected at hour', kind: 'number',
      min: -0x8000_0000, max: 0x7fff_ffff, step: 1, default: null, visibleWhen: () => false },
    { key: 'holder', section: 'options', label: 'Selected country', kind: 'content-choice', automatic: '', default: '',
      visibleWhen: () => false },
    raceViewOption(part),
    { key: 'split', section: 'options', label: `Split bars by ${part}`, kind: 'toggle', default: false,
      visibleWhen: options => options.view === 'race' },
    DATE_OVERLAY_OPTION,
    { key: 'endLabels', section: 'options', label: 'Names at line ends', kind: 'toggle', default: true,
      visibleWhen: options => options.view === 'lines' },
    ...RACE_TIME_OPTIONS,
    ...RACE_AXIS_OPTIONS,
  ];
}

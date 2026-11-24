import type { WidgetMeta } from '../../dashboard/definitions';
import { END_LABELS_OPTION, LINE_AXIS_OPTIONS, LINE_TIME_OPTIONS } from '../../dashboard/lineChartOptions.ts';
import type { OptionField } from '../../dashboard/options';

export const OPTIONS: readonly OptionField[] = [
  ...LINE_TIME_OPTIONS,
  { key: 'measure', section: 'options', label: 'Measure', kind: 'choice', default: 'banked',
    choices: [{ value: 'banked', label: 'Banked' }, { value: 'gain', label: 'Daily gain' }, { value: 'change', label: 'Change per day' }] },
  { key: 'army', section: 'options', label: 'Army', kind: 'toggle', default: true },
  { key: 'navy', section: 'options', label: 'Navy', kind: 'toggle', default: true },
  { key: 'air', section: 'options', label: 'Air', kind: 'toggle', default: true },
  END_LABELS_OPTION,
  ...LINE_AXIS_OPTIONS,
];

export const definition = {
  kind: 'experience', version: 1, title: 'Experience', icon: 'chart-line',
  description: 'Army, navy and air experience over time: banked, daily gain or net change per day.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'follow', scope: 'history', data: true, history: true,
  minSize: { w: 4, h: 6 }, defaultSize: { w: 8, h: 10 },
  countries: { max: Number.MAX_SAFE_INTEGER },
  options: OPTIONS,
} as const satisfies WidgetMeta;

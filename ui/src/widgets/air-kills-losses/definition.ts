import type { WidgetMeta } from '../../dashboard/definitions';
import type { WidgetOptions, OptionField } from '../../dashboard/options';
import { END_LABELS_OPTION } from '../../dashboard/lineChartOptions.ts';
import { DATE_OVERLAY_OPTION, RACE_AXIS_OPTIONS, RACE_TIME_OPTIONS, TOP_COUNTRIES, raceViewVariants } from '../../charts/race/options.ts';
import { DRILL_OPTION } from '../../charts/drill/drillState.ts';

export const OPTIONS: readonly OptionField[] = [
  { key: 'view', section: 'options', label: 'View', kind: 'choice', default: 'race',
    choices: [{ value: 'race', label: 'Race' }, { value: 'lines', label: 'Lines' }, { value: 'breakdown', label: 'Loss causes' }] },
  { key: 'measure', section: 'options', label: 'Rank by', kind: 'choice', default: 'taken',
    choices: [{ value: 'taken', label: 'Own losses' }, { value: 'dealt', label: 'Enemy losses' }] },
  { key: 'regions', section: 'options', label: 'Air regions', kind: 'content-set', default: '', automatic: 'Every recorded region' },
  { key: 'causes', section: 'options', label: 'Loss causes', kind: 'content-set', default: '', automatic: 'Every recorded cause' },
  { key: 'ratio', section: 'options', label: 'Ratio beside race values', kind: 'toggle', default: true, visibleWhen: options => options.view === 'race' },
  DATE_OVERLAY_OPTION, { ...END_LABELS_OPTION, visibleWhen: options => options.view === 'lines' },
  ...RACE_TIME_OPTIONS.map(field => ({ ...field, visibleWhen: options => options.view === 'breakdown'
    ? field.key === 'afterPlayhead' : field.visibleWhen?.(options) ?? true } satisfies OptionField)),
  ...RACE_AXIS_OPTIONS.map(field => ({ ...field, visibleWhen: options => options.view !== 'breakdown'
    && (field.visibleWhen?.(options) ?? true) } satisfies OptionField)),
  { ...DRILL_OPTION, visibleWhen: options => options.view !== 'breakdown' },
];

export const definition = {
  kind: 'air-kills-losses', version: 2, title: 'Air kills & losses', icon: 'target',
  description: 'Aircraft losses and enemy aircraft destroyed, including operational losses.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'follow', scope: 'history', data: true, history: true,
  minSize: { w: 4, h: 6 }, defaultSize: { w: 8, h: 10 }, countries: { max: Number.MAX_SAFE_INTEGER, ranked: true }, defaultCountries: TOP_COUNTRIES,
  migrate: (_from, widget) => ({ ...widget, configVersion: 2, options: normaliseAirOptions(widget.options) }),
  options: OPTIONS,
  variants: raceViewVariants('Air kills & losses', OPTIONS, { breakdown: 'Aircraft loss causes' }),
} as const satisfies WidgetMeta;

export function normaliseAirOptions(options: WidgetOptions = {}): WidgetOptions {
  const { show, ...rest } = options;
  return { ...rest, view: show === 'breakdown' ? 'breakdown' : rest.view ?? 'race' };
}

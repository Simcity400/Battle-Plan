import type { WidgetMeta } from '../../dashboard/definitions';
import { AIR_OPTIONS } from '../../dashboard/airOptions.ts';
import type { OptionField } from '../../dashboard/options';
import { regionPickerControl } from '../../families/settingsControls.ts';
export const OPTIONS: readonly OptionField[] = [...AIR_OPTIONS,
  regionPickerControl({ automatic: 'All active regions' }),
  { key: 'order', section: 'options', label: 'Sort regions by', kind: 'choice', default: 'contested', choices: [
    { value: 'contested', label: 'Most contested first' }, { value: 'active', label: 'Most active' }, { value: 'name', label: 'Name' }] },
];
export const definition = {
  kind: 'air-superiority-region', version: 1, title: 'Air superiority by region', icon: 'plane',
  description: 'Each region’s exact air scores at playback, compared per side or from one viewpoint.',
  timeModes: ['follow', 'pin'], defaultTime: 'follow', scope: 'moment', data: true, countries: { max: 1 }, history: false,
  minSize: { w: 4, h: 5 }, defaultSize: { w: 8, h: 8 }, options: OPTIONS,
} as const satisfies WidgetMeta;

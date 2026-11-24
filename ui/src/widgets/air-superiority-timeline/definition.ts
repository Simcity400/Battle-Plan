import type { WidgetMeta } from '../../dashboard/definitions';
import { AIR_OPTIONS } from '../../dashboard/airOptions.ts';
import type { OptionField } from '../../dashboard/options';
import { regionPickerControl } from '../../families/settingsControls.ts';

export const OPTIONS: readonly OptionField[] = [...AIR_OPTIONS,
  regionPickerControl({ automatic: 'Most active regions' }),
  { key: 'regionCount', section: 'options', label: 'Most active regions shown', kind: 'number', min: 1, max: Number.MAX_SAFE_INTEGER, step: 1, default: 7 },
];
export const definition = {
  kind: 'air-superiority-timeline', version: 1, title: 'Air superiority timeline', icon: 'plane',
  description: 'Air superiority per region from the start of the recording to the playhead, in the leading side’s colour.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'follow', scope: 'history', data: true, countries: { max: 1 }, history: true,
  minSize: { w: 4, h: 5 }, defaultSize: { w: 10, h: 8 }, options: OPTIONS,
} as const satisfies WidgetMeta;

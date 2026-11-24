import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';
import { regionPickerControl } from '../../families/settingsControls.ts';

export const OPTIONS: readonly OptionField[] = [
  { key: 'view', section: 'options', label: 'View', kind: 'choice', default: 'shares',
    choices: [{ value: 'shares', label: 'Shares' }, { value: 'timeline', label: 'Timeline' }] },
  regionPickerControl({ automatic: 'Most active regions', sea: true }),
  { key: 'rowCount', section: 'options', label: 'Most active regions shown', kind: 'number', min: 1, max: Number.MAX_SAFE_INTEGER, step: 1, default: 7 },
];
export const definition = {
  kind: 'naval-supremacy', version: 1, title: 'Naval supremacy', icon: 'anchor', category: 'Navy',
  description: 'Who controls each sea region at the playhead, and day by day from the start of the recording, from one country’s point of view.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'follow', scope: 'history', data: true, countries: { max: 1 }, history: true,
  minSize: { w: 4, h: 5 }, defaultSize: { w: 10, h: 8 }, options: OPTIONS, viewSwitch: { option: 'view' },
} as const satisfies WidgetMeta;

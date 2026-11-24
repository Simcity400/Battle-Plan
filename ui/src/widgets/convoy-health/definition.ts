import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';
import { choiceVariants } from '../../dashboard/options.ts';
import { regionPickerControl } from '../../families/settingsControls.ts';

export const OPTIONS: readonly OptionField[] = [
  { key: 'view', section: 'options', label: 'View', kind: 'choice', default: 'regions',
    choices: [{ value: 'regions', label: 'Regions' }, { value: 'trade', label: 'Trade' }] },
  regionPickerControl({ automatic: 'Most active regions', visibleWhen: o => o.view !== 'trade' }),
  { key: 'rowCount', section: 'options', label: 'Most active regions shown', kind: 'number', min: 1, max: Number.MAX_SAFE_INTEGER, step: 1, default: 10,
    visibleWhen: o => o.view !== 'trade' },
];

export const definition = {
  kind: 'convoy-health', version: 1, title: 'Convoy health', icon: 'anchor', category: 'Economy',
  description: 'Convoy efficiency by sea region, and the trade efficiency of each convoyed import, from the start of the recording to the playhead.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'follow', scope: 'history', data: true, countries: { max: 1 }, history: true,
  minSize: { w: 4, h: 5 }, defaultSize: { w: 10, h: 8 }, options: OPTIONS, viewSwitch: { option: 'view' },
  variants: choiceVariants(OPTIONS.find(field => field.key === 'view'), choice => `Convoy health: ${choice.label}`),
} as const satisfies WidgetMeta;

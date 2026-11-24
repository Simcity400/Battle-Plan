import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';
import { DRILL_OPTION } from '../../charts/drill/drillState.ts';
import { regionPickerControl } from '../../families/settingsControls.ts';

export const OPTIONS: readonly OptionField[] = [
  regionPickerControl({ automatic: 'All mined regions', sea: true }),
  { key: 'order', section: 'options', label: 'Sort regions by', kind: 'choice', default: 'mines', choices: [
    { value: 'mines', label: 'Most mines' }, { value: 'name', label: 'Name' }] },
  DRILL_OPTION,
];
export const definition = {
  kind: 'minefields', version: 1, title: 'Minefields', icon: 'anchor', category: 'Navy',
  description: 'Mines each country has in each sea region, with what the fields have hit.',
  timeModes: ['campaign', 'follow', 'pin'], defaultTime: 'follow', scope: 'moment', data: true, history: true,
  countries: { max: Number.MAX_SAFE_INTEGER, all: true },
  minSize: { w: 4, h: 5 }, defaultSize: { w: 6, h: 8 }, options: OPTIONS,
} as const satisfies WidgetMeta;

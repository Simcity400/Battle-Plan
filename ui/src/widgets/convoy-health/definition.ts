import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';
import { choiceVariants } from '../../dashboard/options.ts';
import { DRILL_OPTION } from '../../charts/drill/drillState.ts';

export const DETAILS = { id: 'row', label: 'Convoy details' };

export const OPTIONS: readonly OptionField[] = [
  { key: 'view', section: 'options', label: 'View', kind: 'choice', default: 'regions',
    choices: [{ value: 'regions', label: 'Convoy efficiency by region' }, { value: 'trade', label: 'Trade efficiency of imports' }] },
  { key: 'rowCount', section: 'options', label: 'Number of rows', hint: 'The most active rows are shown.', kind: 'number', min: 1, max: Number.MAX_SAFE_INTEGER, step: 1, default: 10 },
  DRILL_OPTION,
];

export const definition = {
  kind: 'convoy-health', version: 1, title: 'Convoy health', icon: 'anchor', category: 'Economy',
  description: 'Daily convoy efficiency by sea region, and the trade efficiency of each import deal.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'follow', scope: 'history', data: true, countries: { max: 1 }, history: true,
  minSize: { w: 4, h: 5 }, defaultSize: { w: 10, h: 8 }, options: OPTIONS,
  variants: choiceVariants(OPTIONS.find(field => field.key === 'view'), choice => choice.label),
} as const satisfies WidgetMeta;

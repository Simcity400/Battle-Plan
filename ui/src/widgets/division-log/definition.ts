import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';
import { LOG_OPTIONS } from '../../charts/log/logOptions.ts';

export const OPTIONS: readonly OptionField[] = [
  { key: 'events', section: 'options', label: 'Divisions', kind: 'choice', default: 'both',
    choices: [{ value: 'both', label: 'Created and lost' }, { value: 'created', label: 'Created' }, { value: 'lost', label: 'Lost and transferred' }] },
  { key: 'templates', section: 'options', label: 'Template types', kind: 'content-set', catalogue: true, default: '', automatic: 'Every template' },
  { key: 'totals', section: 'options', label: 'Totals', kind: 'toggle', default: false },
  ...LOG_OPTIONS,
];
export const definition = {
  kind: 'division-log', version: 1, title: 'Division log', icon: 'shield',
  description: 'Division creations, losses and transfers, with templates, locations and attackers.',
  timeModes: ['campaign', 'follow', 'range'], defaultTime: 'follow', scope: 'history', data: true, history: true,
  minSize: { w: 4, h: 6 }, defaultSize: { w: 7, h: 14 }, countries: { max: Number.MAX_SAFE_INTEGER, all: true },
  availability: { product: 'division-log-manifest', params: {} }, options: OPTIONS,
} as const satisfies WidgetMeta;

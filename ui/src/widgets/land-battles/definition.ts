import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';
import { LOG_OPTIONS } from '../../charts/log/logOptions.ts';
import { TABLE_OPTIONS, tableSortOptions } from '../../charts/table/options.ts';

const logView = (options: Readonly<Record<string, unknown>>) => options.view !== 'biggest';
const biggestView = (options: Readonly<Record<string, unknown>>) => options.view === 'biggest';
export const DEFAULT_SORT = { column: 'measure', direction: 'descending' } as const;
export const DEFAULT_TOP_N = 30;
export const OPTIONS: readonly OptionField[] = [
  { key: 'view', section: 'options', label: 'View', kind: 'choice', default: 'log',
    choices: [{ value: 'log', label: 'Battle log' }, { value: 'biggest', label: 'Biggest battles' }] },
  { key: 'measure', section: 'options', label: 'Biggest by', kind: 'choice', default: 'Casualties', visibleWhen: biggestView,
    choices: [{ value: 'Casualties', label: 'Casualties' }, { value: 'Peak size', label: 'Peak size' }, { value: 'Length', label: 'Length' }] },
  { key: 'topN', section: 'options', label: 'Top battles', kind: 'number', min: 1, max: Infinity, step: 1, default: DEFAULT_TOP_N, visibleWhen: biggestView },
  ...LOG_OPTIONS.filter(field => field.key !== 'chips' && field.key !== 'density').map(field => ({ ...field, visibleWhen: logView })),
  ...TABLE_OPTIONS.map(field => field.key === 'density' ? field : { ...field, visibleWhen: biggestView }),
  ...tableSortOptions([{ key: 'measure', label: 'Leaderboard measure' }, { key: 'Casualties', label: 'Casualties' }, { key: 'Peak size', label: 'Peak size' }, { key: 'Length', label: 'Length' },
    { key: 'date', label: 'Date' }, { key: 'winner', label: 'Winner' }], DEFAULT_SORT).map(field => ({ ...field, visibleWhen: biggestView })),
];
export const definition = {
  kind: 'land-battles', version: 1, title: 'Land battles', icon: 'shield',
  description: 'Observed land battles, their recorded sides and division counts, with a biggest-battles leaderboard.',
  timeModes: ['campaign', 'follow', 'range'], defaultTime: 'campaign', scope: 'history', data: true, history: true,
  minSize: { w: 4, h: 6 }, defaultSize: { w: 10, h: 14 }, countries: { max: Number.MAX_SAFE_INTEGER, all: true },
  availability: { product: 'land-battles-manifest', params: {} },
  options: OPTIONS,
} as const satisfies WidgetMeta;

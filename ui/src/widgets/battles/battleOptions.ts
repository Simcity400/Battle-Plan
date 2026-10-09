import type { OptionField } from '../../dashboard/options';
import { LOG_OPTIONS } from '../../charts/log/logOptions.ts';
import { TABLE_OPTIONS, tableSortOptions } from '../../charts/table/options.ts';

export const DEFAULT_SORT = { column: 'measure', direction: 'descending' } as const;
export const DEFAULT_TOP_N = 30;
export function battleOptions(measures: readonly { value: string; label: string }[]): readonly OptionField[] {
  const log = (options: Readonly<Record<string, unknown>>) => options.view !== 'biggest';
  const biggest = (options: Readonly<Record<string, unknown>>) => options.view === 'biggest';
  return [
    { key: 'view', section: 'options', label: 'View', kind: 'choice', default: 'log',
      choices: [{ value: 'log', label: 'Battle log' }, { value: 'biggest', label: 'Biggest battles' }] },
    { key: 'measure', section: 'options', label: 'Biggest by', kind: 'choice', default: measures[0].value, visibleWhen: biggest, choices: measures },
    { key: 'topN', section: 'options', label: 'Top battles', kind: 'number', min: 1, max: Infinity, step: 1, default: DEFAULT_TOP_N, visibleWhen: biggest },
    ...LOG_OPTIONS.filter(field => field.key !== 'chips' && field.key !== 'density').map(field => ({ ...field, visibleWhen: log })),
    ...TABLE_OPTIONS.map(field => field.key === 'density' ? field : { ...field, visibleWhen: biggest }),
    ...tableSortOptions([{ key: 'measure', label: 'Leaderboard measure' }, ...measures.map(measure => ({ key: measure.value, label: measure.label })),
      { key: 'date', label: 'Date' }, { key: 'winner', label: 'Winner' }], DEFAULT_SORT).map(field => ({ ...field, visibleWhen: biggest })),
  ];
}

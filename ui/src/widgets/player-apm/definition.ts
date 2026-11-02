import type { OptionField } from '../../dashboard/options';
import { TABLE_OPTIONS, tableSortOptions } from '../../charts/table/options.ts';

export const SHARED_OPTIONS: readonly OptionField[] = [
  { key: 'group', section: 'options', label: 'Compare', kind: 'choice', default: 'country',
    choices: [{ value: 'country', label: 'Countries' }, { value: 'player', label: 'Players' }] },
  { key: 'players', section: 'options', label: 'Players', kind: 'content-set', catalogue: true,
    automatic: 'All players in the selected countries', default: '', visibleWhen: options => options.group === 'player' },
  { key: 'showPausedActions', section: 'options', label: 'Show paused actions', kind: 'toggle', default: false },
];

export const GRAPH_OPTIONS: readonly OptionField[] = [
  ...SHARED_OPTIONS,
  { key: 'yMin', section: 'axes', label: 'Value axis from', hint: 'Empty starts at zero', kind: 'number', min: 0, max: 1e9, step: 1, default: null },
  { key: 'yMax', section: 'axes', label: 'Value axis to', hint: 'Empty fits the data', kind: 'number', min: 0, max: 1e9, step: 1, default: null },
];

export const DEFAULT_SORT = { column: 'apm', direction: 'descending' } as const;
export const TABLE_COLUMNS = [{ key: 'name', label: 'Country / player' }, { key: 'apm', label: 'eAPM' },
  { key: 'orders', label: 'Orders' }, { key: 'paused', label: 'Paused orders' }, { key: 'minutes', label: 'Unpaused minutes' }];
export const TABLE_APM_OPTIONS: readonly OptionField[] = [...SHARED_OPTIONS, ...TABLE_OPTIONS, ...tableSortOptions(TABLE_COLUMNS, DEFAULT_SORT)];

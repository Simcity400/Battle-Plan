import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';
import { TABLE_OPTIONS, tableSortOptions } from '../../charts/table/options.ts';
import { DRILL_OPTION } from '../../charts/drill/drillState.ts';

export const AGREEMENT = { id: 'agreement', label: 'Agreement' };
export const DIRECTIONS = [{ value: 'sent', label: 'Sent' }, { value: 'received', label: 'Received' }] as const;
export const SORT_COLUMNS = [{ key: 'delivered', label: 'Delivered' }, { key: 'since', label: 'Since' }, { key: 'last', label: 'Last delivery' },
  { key: 'lost', label: 'Lost' }, { key: 'partner', label: 'Countries' }] as const;
export const DEFAULT_SORT = { column: 'delivered', direction: 'descending' } as const;

export const OPTIONS: readonly OptionField[] = [
  { key: 'direction', section: 'options', label: 'Direction', kind: 'choice', default: 'sent', choices: [...DIRECTIONS] },
  { key: 'ended', section: 'options', label: 'Ended agreements', kind: 'choice', default: 'shown',
    choices: [{ value: 'shown', label: 'Shown' }, { value: 'hidden', label: 'Hidden' }] },
  ...tableSortOptions(SORT_COLUMNS, DEFAULT_SORT), ...TABLE_OPTIONS, DRILL_OPTION,
];

export const definition = {
  kind: 'lend-lease-agreements', version: 1, title: 'Lend-lease agreements', icon: 'link', category: 'Economy',
  description: 'Lend-lease each country sends or receives: schedule, delivered and lost.',
  timeModes: ['campaign', 'follow', 'pin'], defaultTime: 'follow', scope: 'history', data: true, history: true,
  minSize: { w: 4, h: 6 }, defaultSize: { w: 6, h: 10 }, countries: { max: Number.MAX_SAFE_INTEGER, all: true }, options: OPTIONS, viewSwitch: { option: 'direction' },
} as const satisfies WidgetMeta;

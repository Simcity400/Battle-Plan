import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';
import { DRILL_OPTION } from '../../charts/drill/drillState.ts';
import { TABLE_OPTIONS, tableSortOptions } from '../../charts/table/options.ts';
import type { Sort } from '../../charts/table/logic';
import { choiceVariants } from '../../dashboard/options.ts';

export const TRANSFER_BREAKDOWN = { id: 'parts', label: 'Country transfers' };
export type TransferView = 'main' | 'transfers' | 'parts';

export const COUNTRY_COLUMNS = [
  { key: 'country', label: 'Country' }, { key: 'gained', label: 'Gained' }, { key: 'lost', label: 'Lost' },
  { key: 'net', label: 'Net' }, { key: 'netVictoryPoints', label: 'Net VP' },
] as const;
export const TRANSFER_COLUMNS = [
  { key: 'state', label: 'State' }, { key: 'from', label: 'From' }, { key: 'to', label: 'To' },
  { key: 'date', label: 'Date' }, { key: 'victoryPoints', label: 'VP' },
] as const;
export const COUNTRY_SORT: Sort = { column: 'net', direction: 'descending' };
export const TRANSFER_SORT: Sort = { column: 'date', direction: 'descending' };
export const TRANSFER_SORT_KEY = 'transferSort';

export const OPTIONS: readonly OptionField[] = [
  { key: 'show', section: 'options', label: 'Show', kind: 'choice', default: 'main', choices: [
    { value: 'main', label: 'Countries' }, { value: 'transfers', label: 'Transfers' }, { value: TRANSFER_BREAKDOWN.id, label: TRANSFER_BREAKDOWN.label },
  ] },
  { key: 'selection', section: 'options', label: 'Transfer selection', kind: 'content-choice', automatic: 'All transfers', default: '', visibleWhen: () => false },
  DRILL_OPTION,
  { key: 'scope', section: 'options', label: 'Scope', kind: 'choice', default: 'campaign',
    choices: [{ value: 'campaign', label: 'Whole campaign' }, { value: 'conference', label: 'One conference' }] },
  { key: 'conference', section: 'options', label: 'Conference', kind: 'content-choice', automatic: 'Newest conference shown', default: '',
    visibleWhen: options => options.scope === 'conference' },
  ...TABLE_OPTIONS.filter(option => option.key !== 'rankMarkers'),
  { ...tableSortOptions(COUNTRY_COLUMNS, COUNTRY_SORT)[0], visibleWhen: options => options.show !== 'transfers' && options.show !== TRANSFER_BREAKDOWN.id },
  { ...tableSortOptions(TRANSFER_COLUMNS, TRANSFER_SORT)[0], key: TRANSFER_SORT_KEY, visibleWhen: options => options.show === 'transfers' || options.show === TRANSFER_BREAKDOWN.id },
];

export const definition = {
  kind: 'territory-transfers', version: 2, title: 'Territory transfers', category: 'Diplomacy', icon: 'flows',
  description: 'Who gained and lost states in peace settlements, and when.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'follow', scope: 'history', data: true, history: true,
  countries: { max: Number.MAX_SAFE_INTEGER, all: true },
  defaultCountries: { kind: 'all' }, minSize: { w: 4, h: 6 }, defaultSize: { w: 8, h: 12 }, options: OPTIONS,
  migrate: (_from, widget) => widget,
  variants: choiceVariants(OPTIONS.find(field => field.key === 'show'), () => 'Territory transfer log', ['transfers']),
} as const satisfies WidgetMeta;

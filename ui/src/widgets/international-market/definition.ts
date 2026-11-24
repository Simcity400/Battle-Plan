import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';
import { choiceVariants } from '../../dashboard/options.ts';
import { tableFamilyOptions, type TableBase } from '../../families/table/settings.ts';
import { DRILL_OPTION } from '../../charts/drill/drillState.ts';

export const DETAILS = { id: 'details', label: 'Market details' };
export const VIEWS = [{ value: 'contracts', label: 'Contracts' }, { value: 'requests', label: 'Requests' }, { value: 'inventory', label: 'Inventory' }] as const;
export const SORTS = {
  contracts: { column: 'collected', direction: 'descending' },
  requests: { column: 'amount', direction: 'descending' },
  inventory: { column: 'amount', direction: 'descending' },
} as const;

const MARKET_TABLE_BASE: TableBase = { density: 'normal', bars: 'all', overflow: 'scroll', showRank: false, sort: SORTS.contracts };

export const OPTIONS: readonly OptionField[] = [
  { key: 'view', section: 'options', label: 'View', kind: 'choice', default: 'contracts', choices: [...VIEWS] },
  ...tableFamilyOptions([], MARKET_TABLE_BASE),
  DRILL_OPTION,
];

export const definition = {
  kind: 'international-market', version: 1, title: 'International market', icon: 'globe', category: 'Economy',
  description: 'Market contracts, open requests and listed equipment, with convoys and payment.',
  timeModes: ['follow', 'pin'], defaultTime: 'follow', scope: 'moment', data: true, history: false,
  minSize: { w: 6, h: 6 }, defaultSize: { w: 12, h: 10 }, countries: { max: Number.MAX_SAFE_INTEGER, all: true },
  defaultCountries: { kind: 'all' }, options: OPTIONS, viewSwitch: { option: 'view' }, playsBackDated: true,
  variants: choiceVariants(OPTIONS.find(field => field.key === 'view'), choice => `Market ${choice.label.toLowerCase()}`),
} as const satisfies WidgetMeta;

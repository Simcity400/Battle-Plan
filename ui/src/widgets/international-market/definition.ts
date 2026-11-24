import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';
import { choiceVariants } from '../../dashboard/options.ts';
import { columnsControl } from '../../families/settingsControls.ts';
import type { TableColumnSpec } from '../../families/table/model.ts';
import { OFFERED, TABLE_SETTINGS_KEY, type TableBase } from '../../families/table/settings.ts';
import type { MarketTab } from './marketData.ts';
import { DRILL_OPTION } from '../../charts/drill/drillState.ts';

export const DETAILS = { id: 'details', label: 'Market details' };
export const VIEWS = [{ value: 'contracts', label: 'Contracts' }, { value: 'requests', label: 'Requests' }, { value: 'inventory', label: 'Inventory' }] as const;
export const SORTS = {
  contracts: { column: 'collected', direction: 'descending' },
  requests: { column: 'amount', direction: 'descending' },
  inventory: { column: 'amount', direction: 'descending' },
} as const;

export const MARKET_TABS: readonly MarketTab[] = ['contracts', 'requests', 'inventory'];

export function marketColumns(view: MarketTab): TableColumnSpec[] {
  if (view === 'inventory') {
    return [
      {
        key: 'seller',
        label: 'Seller',
        as: 'flag',
        offered: OFFERED.country,
        showNamesDefault: false,
        pinned: true,
        priority: 0,
        text: true,
        hideable: false,
      },
      {
        key: 'equipment',
        label: 'Equipment',
        as: 'icon',
        cellKind: 'item',
        offered: OFFERED.text,
        pinned: true,
        priority: 0,
        text: true,
        hideable: false,
        minWidth: 200,
      },
      {
        key: 'amount',
        label: 'On market',
        as: 'faint',
        barEligible: true,
        offered: OFFERED.number,
        priority: 1,
        numeric: true,
        format: 'fixed5Amount',
        widest: ['99,999'],
      },
      {
        key: 'level',
        label: 'Price level',
        as: 'plain',
        barEligible: false,
        offered: OFFERED.text,
        priority: 2,
        text: true,
        minWidth: 104,
      },
    ];
  }
  if (view === 'requests') {
    return [
      {
        key: 'buyer',
        label: 'Buyer',
        as: 'flag',
        offered: OFFERED.country,
        showNamesDefault: false,
        pinned: true,
        priority: 0,
        text: true,
        hideable: false,
      },
      {
        key: 'seller',
        label: 'Seller',
        as: 'flag',
        offered: OFFERED.country,
        showNamesDefault: false,
        pinned: true,
        priority: 0,
        text: true,
        hideable: false,
      },
      {
        key: 'equipment',
        label: 'Wants',
        as: 'icon',
        cellKind: 'item',
        offered: OFFERED.text,
        priority: 1,
        text: true,
        minWidth: 200,
      },
      {
        key: 'amount',
        label: 'Amount',
        as: 'faint',
        barEligible: true,
        offered: OFFERED.number,
        priority: 1,
        numeric: true,
        format: 'fixed5Amount',
        widest: ['99,999'],
      },
      {
        key: 'level',
        label: 'Price offered',
        as: 'plain',
        barEligible: false,
        offered: OFFERED.text,
        priority: 2,
        text: true,
        minWidth: 110,
      },
    ];
  }
  return [
    {
      key: 'seller',
      label: 'Seller',
      as: 'flag',
      offered: OFFERED.country,
      showNamesDefault: false,
      pinned: true,
      priority: 0,
      text: true,
      hideable: false,
    },
    {
      key: 'buyer',
      label: 'Buyer',
      as: 'flag',
      offered: OFFERED.country,
      showNamesDefault: false,
      pinned: true,
      priority: 0,
      text: true,
      hideable: false,
    },
    {
      key: 'equipment',
      label: 'Equipment',
      as: 'icon',
      cellKind: 'item',
      offered: OFFERED.text,
      priority: 1,
      text: true,
      minWidth: 200,
    },
    {
      key: 'price',
      label: 'Price',
      unit: 'CIC',
      as: 'plain',
      barEligible: false,
      offered: OFFERED.number,
      priority: 2,
      numeric: true,
      format: 'fixed5Amount',
      minWidth: 84,
      widest: ['99,999'],
    },
    {
      key: 'convoys',
      label: 'Convoys',
      unit: 'used / needed',
      as: 'plain',
      cellKind: 'outOf',
      barEligible: false,
      offered: OFFERED.number,
      priority: 3,
      numeric: true,
      minWidth: 86,
      widest: ['99 / 99'],
    },
    {
      key: 'collected',
      label: 'Collected',
      unit: 'CIC',
      as: 'faint',
      barEligible: true,
      offered: OFFERED.number,
      priority: 1,
      numeric: true,
      format: 'fixed5Amount',
      minWidth: 96,
      widest: ['99,999'],
    },
    {
      key: 'deliveries',
      label: 'Deliveries',
      as: 'faint',
      barEligible: true,
      offered: OFFERED.number,
      priority: 2,
      numeric: true,
      format: 'integer',
      minWidth: 88,
      widest: ['999'],
    },
  ];
}

export function marketTableBase(view: MarketTab): TableBase {
  return {
    density: 'normal',
    barsBehind: 'all',
    showRank: false,
    sort: SORTS[view],
  };
}

export const marketSettingsKey = (view: MarketTab) => `${TABLE_SETTINGS_KEY}:${view}`;

export function marketTableOptions(view: MarketTab): readonly OptionField[] {
  return [columnsControl(marketColumns(view), marketTableBase(view), { key: marketSettingsKey(view) })];
}

export const OPTIONS: readonly OptionField[] = [
  { key: 'view', section: 'options', label: 'View', kind: 'choice', default: 'contracts', choices: [...VIEWS] },
  DRILL_OPTION,
];

export const definition = {
  kind: 'international-market', version: 1, title: 'International market', icon: 'globe', category: 'Economy',
  description: 'Market contracts, open requests and listed equipment, with convoys and payment.',
  timeModes: ['follow', 'pin'], defaultTime: 'follow', scope: 'moment', data: true, history: false,
  minSize: { w: 6, h: 6 }, defaultSize: { w: 12, h: 10 }, countries: { max: Number.MAX_SAFE_INTEGER, all: true },
  defaultCountries: { kind: 'all' }, options: OPTIONS, viewSwitch: { option: 'view' },
  variants: choiceVariants(OPTIONS.find(field => field.key === 'view'), choice => `Market ${choice.label.toLowerCase()}`),
} as const satisfies WidgetMeta;

import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField, OptionValue } from '../../dashboard/options';
import type { CountrySelection, WidgetTime } from '../../dashboard/model';
import { DRILL_OPTION, breakdownOption } from '../../charts/drill/drillState.ts';
import type { TableColumnSpec } from '../../families/table/model.ts';
import { OFFERED, tableFamilyOptions, type TableBase } from '../../families/table/settings.ts';

const mainView = (options: Readonly<Record<string, OptionValue>>) => options.show !== 'efficiency' && options.show !== 'output';
const tableOptionsVisible = (options: Readonly<Record<string, OptionValue>>, _countries?: CountrySelection, mode?: WidgetTime['mode']) =>
  mainView(options) && (mode === 'follow' || mode === 'pin');
const endedLinesVisible = (options: Readonly<Record<string, OptionValue>>, _countries?: CountrySelection, mode?: WidgetTime['mode']) =>
  mainView(options) && (mode === 'campaign' || mode === 'range');
export const PRODUCTION_LINES_COLUMNS: readonly TableColumnSpec[] = [
  {
    key: 'picture',
    label: '',
    as: 'plain',
    cellKind: 'kind',
    offered: OFFERED.text,
    pinned: true,
    priority: 0,
    sortable: false,
    hideable: false,
  },
  {
    key: 'equipment',
    label: 'Equipment',
    as: 'plain',
    cellKind: 'item',
    offered: OFFERED.text,
    pinned: true,
    priority: 0,
    text: true,
    hideable: false,
    minWidth: 180,
    widest: ['Infantry Equipment II'],
  },
  {
    key: 'factories',
    label: 'Factories',
    unit: 'assigned / maximum',
    as: 'faint',
    cellKind: 'outOf',
    barEligible: true,
    offered: OFFERED.number,
    priority: 1,
    numeric: true,
    format: 'integer',
    minWidth: 96,
    widest: ['150 / 150'],
  },
  {
    key: 'efficiency',
    label: 'Efficiency',
    as: 'track',
    cellKind: 'track',
    trackTone: 'accent',
    barEligible: false,
    offered: OFFERED.number,
    priority: 1,
    numeric: true,
    minWidth: 118,
    widest: ['100%'],
  },
  {
    key: 'output',
    label: 'Output',
    unit: 'per week',
    as: 'faint',
    barEligible: true,
    offered: OFFERED.number,
    priority: 2,
    numeric: true,
    minWidth: 96,
    widest: ['9,999.9'],
  },
  {
    key: 'total',
    label: 'Produced',
    unit: 'in total',
    as: 'faint',
    barEligible: true,
    offered: OFFERED.number,
    priority: 2,
    numeric: true,
    minWidth: 96,
    widest: ['999,999'],
  },
  {
    key: 'priority',
    label: 'Priority',
    as: 'plain',
    barEligible: false,
    offered: OFFERED.number,
    priority: 4,
    numeric: true,
    format: 'integer',
    defaultHidden: true,
    minWidth: 68,
  },
  {
    key: 'cost',
    label: 'IC',
    unit: 'per unit',
    as: 'plain',
    barEligible: false,
    offered: OFFERED.number,
    priority: 5,
    numeric: true,
    defaultHidden: true,
    minWidth: 78,
  },
  {
    key: 'mio',
    label: 'MIO',
    as: 'icon',
    offered: OFFERED.text,
    priority: 6,
    text: true,
    defaultHidden: true,
    minWidth: 132,
  },
];

export const PRODUCTION_LINES_BASE: TableBase = {
  density: 'normal',
  barsBehind: 'all',
  overflow: 'scroll',
  showRank: false,
  sort: { column: 'factories', direction: 'descending' },
};


export const OPTIONS: readonly OptionField[] = [
  { key: 'productionCountry', section: 'options', label: 'Selected country', kind: 'content-choice', automatic: 'Selected country', default: '', visibleWhen: () => false },
  breakdownOption([{ id: 'efficiency', label: 'Efficiency breakdown' }, { id: 'output', label: 'Output breakdown' }], 'Production lines'),
  { key: 'line', section: 'options', label: 'Production line', kind: 'content-choice', automatic: 'Choose a production line', default: '', visibleWhen: options => options.show === 'efficiency' || options.show === 'output' },
  { key: 'factor', section: 'options', label: 'Production factor', kind: 'content-choice', automatic: 'Default breakdown', default: '', visibleWhen: () => false },
  DRILL_OPTION,
  ...tableFamilyOptions(PRODUCTION_LINES_COLUMNS, PRODUCTION_LINES_BASE).map(field => ({ ...field, visibleWhen: tableOptionsVisible })),
  { key: 'ended', section: 'options', label: 'Ended lines', visibleWhen: endedLinesVisible, kind: 'choice', default: 'shown',
    choices: [{ value: 'shown', label: 'Shown' }, { value: 'hidden', label: 'Hidden' }] },
];

export const definition = {
  kind: 'production-lines', version: 1, title: 'Production lines', icon: 'grid',
  description: 'A country’s production lines: factories, efficiency, output and totals, live or across the campaign.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'follow', scope: 'history', data: true, history: true,
  minSize: { w: 5, h: 6 }, defaultSize: { w: 12, h: 14 },
  options: OPTIONS,
} as const satisfies WidgetMeta;

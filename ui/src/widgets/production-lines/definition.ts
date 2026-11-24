import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField, OptionValue } from '../../dashboard/options';
import type { CountrySelection, WidgetTime } from '../../dashboard/model';
import { DRILL_OPTION, breakdownOption } from '../../charts/drill/drillState.ts';
import { tableFamilyOptions, type TableBase } from '../../families/table/settings.ts';

const mainView = (options: Readonly<Record<string, OptionValue>>) => options.show !== 'efficiency' && options.show !== 'output';
const tableOptionsVisible = (options: Readonly<Record<string, OptionValue>>, _countries?: CountrySelection, mode?: WidgetTime['mode']) =>
  mainView(options) && (mode === 'follow' || mode === 'pin');
const endedLinesVisible = (options: Readonly<Record<string, OptionValue>>, _countries?: CountrySelection, mode?: WidgetTime['mode']) =>
  mainView(options) && (mode === 'campaign' || mode === 'range');
const PROD_TABLE_BASE: TableBase = { density: 'normal', barsBehind: 'all', overflow: 'scroll', showRank: false, sort: { column: 'priority', direction: 'ascending' } };

export const OPTIONS: readonly OptionField[] = [
  { key: 'productionCountry', section: 'options', label: 'Selected country', kind: 'content-choice', automatic: 'Selected country', default: '', visibleWhen: () => false },
  breakdownOption([{ id: 'efficiency', label: 'Efficiency breakdown' }, { id: 'output', label: 'Output breakdown' }], 'Production lines'),
  { key: 'line', section: 'options', label: 'Production line', kind: 'content-choice', automatic: 'Choose a production line', default: '', visibleWhen: options => options.show === 'efficiency' || options.show === 'output' },
  { key: 'factor', section: 'options', label: 'Production factor', kind: 'content-choice', automatic: 'Default breakdown', default: '', visibleWhen: () => false },
  DRILL_OPTION,
  { key: 'order', section: 'options', label: 'Order', visibleWhen: tableOptionsVisible, kind: 'choice', default: 'game',
    choices: [{ value: 'game', label: 'Game order' }, { value: 'factories', label: 'Factories' }, { value: 'output', label: 'Output' }] },
  { key: 'columns', section: 'options', label: 'Columns', visibleWhen: tableOptionsVisible, kind: 'choice', default: 'basic',
    choices: [{ value: 'basic', label: 'Basic' }, { value: 'more', label: '+ Priority, IC per unit and MIO' }] },
  ...tableFamilyOptions([], PROD_TABLE_BASE).map(field => ({ ...field, visibleWhen: tableOptionsVisible })),
  { key: 'ended', section: 'options', label: 'Ended lines', visibleWhen: endedLinesVisible, kind: 'choice', default: 'shown',
    choices: [{ value: 'shown', label: 'Shown' }, { value: 'hidden', label: 'Hidden' }] },
];

export const definition = {
  kind: 'production-lines', version: 1, title: 'Production lines', icon: 'grid',
  description: 'A country’s production lines: factories, efficiency, output and totals, live or across the campaign.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'follow', scope: 'history', data: true, history: true,
  minSize: { w: 5, h: 6 }, defaultSize: { w: 12, h: 14 },
  options: OPTIONS, playsBackDated: true,
} as const satisfies WidgetMeta;

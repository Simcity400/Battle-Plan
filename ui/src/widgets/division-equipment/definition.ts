import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';
import { REACHED_COUNTRY_OPTION } from '../../dashboard/reachedCountry.ts';
import { choiceVariants } from '../../dashboard/options.ts';
import { OFFERED, tableFamilyOptions, type TableBase, type TableColumnDefaults } from '../../families/table/settings.ts';
import { COUNT_FOR_OPTION } from '../division-template/definition.ts';

export const VIEWS = [{ value: 'equipment', label: 'By equipment' }, { value: 'template', label: 'By template' }] as const;
export const VIEW_OPTION: OptionField = { key: 'view', section: 'options', label: 'View', kind: 'choice', default: 'equipment', choices: [...VIEWS] };

export const TYPE_COLUMNS: readonly TableColumnDefaults[] = [
  { key: 'type', label: 'Equipment', as: 'plain', offered: OFFERED.text },
  { key: 'held', label: 'In divisions', as: 'faint', tone: 'grey', offered: OFFERED.number },
  { key: 'needed', label: 'Needed', as: 'faint', tone: 'grey', offered: OFFERED.number },
  { key: 'missing', label: 'Missing', as: 'plain', offered: ['plain'] },
  { key: 'fill', label: 'Fill', as: 'meterBefore', tone: 'grey', offered: OFFERED.number },
  { key: 'short', label: 'Divisions short', as: 'faint', tone: 'grey', offered: OFFERED.number },
];
export const TEMPLATE_COLUMNS: readonly TableColumnDefaults[] = [
  { key: 'kind', label: 'Division type', as: 'plain', offered: ['plain'] },
  { key: 'template', label: 'Template', as: 'plain', offered: OFFERED.text },
  { key: 'divisions', label: 'Divisions', as: 'faint', tone: 'grey', offered: OFFERED.number },
  { key: 'manpower', label: 'Manpower', as: 'meterBefore', tone: 'grey', offered: OFFERED.number },
  { key: 'strength', label: 'Strength', as: 'meterBefore', tone: 'grey', offered: OFFERED.number },
  { key: 'fill', label: 'Equipment fill', as: 'meterBefore', tone: 'grey', offered: OFFERED.number },
  { key: 'short', label: 'Types short', as: 'faint', tone: 'grey', offered: OFFERED.number },
];
export const TYPE_BASE: TableBase = { density: 'normal', barsBehind: 'all', overflow: 'tooltip', sort: { column: 'missing', direction: 'descending' } };
export const TEMPLATE_BASE: TableBase = { density: 'normal', barsBehind: 'all', overflow: 'tooltip', sort: { column: 'fill', direction: 'ascending' } };
export const TYPE_TABLE_KEY = 'equipmentTable';
export const TEMPLATE_TABLE_KEY = 'templateTable';

const tableField = (columns: readonly TableColumnDefaults[], base: TableBase, key: string): OptionField => ({ ...tableFamilyOptions(columns, base)[0]!, key });
export const TYPE_TABLE_OPTION = tableField(TYPE_COLUMNS, TYPE_BASE, TYPE_TABLE_KEY);
export const TEMPLATE_TABLE_OPTION = tableField(TEMPLATE_COLUMNS, TEMPLATE_BASE, TEMPLATE_TABLE_KEY);

export const OPTIONS: readonly OptionField[] = [REACHED_COUNTRY_OPTION, VIEW_OPTION, COUNT_FOR_OPTION, TYPE_TABLE_OPTION, TEMPLATE_TABLE_OPTION];
export const definition = {
  kind: 'division-equipment', version: 1, title: 'Division equipment', icon: 'cube', category: 'Army',
  description: 'What a country’s divisions hold, need and are missing, by equipment type or by template.',
  timeModes: ['campaign', 'follow', 'pin'], defaultTime: 'follow', scope: 'history', data: true, history: true,
  countries: { max: 1 }, minSize: { w: 4, h: 6 }, defaultSize: { w: 8, h: 11 }, options: OPTIONS, viewSwitch: { option: 'view' },
  variants: choiceVariants(VIEW_OPTION, choice => `Division equipment ${choice.label.toLowerCase()}`),
} as const satisfies WidgetMeta;

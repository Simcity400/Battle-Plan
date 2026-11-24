import type { OptionField } from '../../dashboard/options';
import { TABLE_OPTIONS } from '../../charts/table/options.ts';
import { OFFERED, tableFamilyOptions, type TableBase, type TableColumnDefaults } from '../../families/table/settings.ts';

export const SHARED_OPTIONS: readonly OptionField[] = [
  { key: 'group', section: 'options', label: 'Compare', kind: 'choice', default: 'country',
    choices: [{ value: 'country', label: 'Countries' }, { value: 'player', label: 'Players' }] },
  { key: 'players', section: 'options', label: 'Players', kind: 'content-set', catalogue: true,
    automatic: 'All players in the selected countries', default: '', visibleWhen: options => options.group === 'player' },
  { key: 'showPausedActions', section: 'options', label: 'Show paused actions', kind: 'toggle', default: false },
];

export const DEFAULT_SORT = { column: 'apm', direction: 'descending' } as const;
export const TABLE_BASE: TableBase = { density: 'normal', barsBehind: 'all', overflow: 'scroll', showRank: false, sort: DEFAULT_SORT };
const COLUMN_DEFAULTS: readonly TableColumnDefaults[] = [
  { key: 'country', label: 'Country', as: 'flag', offered: OFFERED.country, showNamesDefault: false },
  { key: 'name', label: 'Player', as: 'plain', offered: OFFERED.text, hideable: false },
  { key: 'apm', label: 'eAPM', as: 'faint', offered: OFFERED.number },
  { key: 'orders', label: 'Actions', as: 'faint', offered: OFFERED.number },
  { key: 'paused', label: 'Paused actions', as: 'faint', offered: OFFERED.number },
  { key: 'minutes', label: 'Played', as: 'plain', offered: OFFERED.number },
];
export const TABLE_APM_OPTIONS: readonly OptionField[] = [
  ...SHARED_OPTIONS,
  ...TABLE_OPTIONS.filter(field => field.key === 'rankMarkers'),
  ...tableFamilyOptions(COLUMN_DEFAULTS, TABLE_BASE),
];

import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';
import { choiceVariants } from '../../dashboard/options.ts';
import { LOG_OPTIONS } from '../../charts/log/logOptions.ts';
import { TABLE_OPTIONS, tableSortOptions } from '../../charts/table/options.ts';

export const VIEWS = [{ value: 'proposals', label: 'Proposals' }, { value: 'pacts', label: 'Pacts' }] as const;
export const COLUMNS = [{ key: 'kind', label: 'Kind' }, { key: 'from', label: 'From' }, { key: 'to', label: 'To' },
  { key: 'since', label: 'Since' }, { key: 'until', label: 'Until' }, { key: 'status', label: 'Status' }] as const;
export const DEFAULT_SORT = { column: 'until', direction: 'descending' } as const;
const pacts = (options: Readonly<Record<string, unknown>>) => options.view === 'pacts';

export const OPTIONS: readonly OptionField[] = [
  { key: 'view', section: 'options', label: 'View', kind: 'choice', default: 'proposals', choices: VIEWS },
  { key: 'waiting', section: 'options', label: 'Waiting only', kind: 'toggle', default: false,
    hint: 'Only proposals still in the queue at the moment shown.', visibleWhen: options => !pacts(options) },
  ...LOG_OPTIONS.filter(field => field.key !== 'chips').map(field => ({ ...field, visibleWhen: (options: Readonly<Record<string, unknown>>) => !pacts(options) })),
  { key: 'kinds', section: 'options', label: 'Kinds', kind: 'content-set', default: '', automatic: 'All kinds', visibleWhen: pacts },
  { key: 'ended', section: 'options', label: 'Ended pacts', kind: 'choice', default: 'shown', visibleWhen: pacts,
    choices: [{ value: 'shown', label: 'Shown' }, { value: 'hidden', label: 'Hidden' }] },
  ...[...TABLE_OPTIONS.filter(field => field.key !== 'cellBars' && field.key !== 'rankMarkers'), ...tableSortOptions(COLUMNS, DEFAULT_SORT)]
    .map(field => ({ ...field, visibleWhen: pacts })),
];

export const definition = {
  kind: 'diplomatic-pacts', version: 1, title: 'Diplomatic pacts & proposals', icon: 'link', category: 'Diplomacy',
  description: 'Proposals waiting in each country’s queue, and every recorded pact and relation with its dates.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'campaign', scope: 'history', data: true, history: true,
  minSize: { w: 4, h: 6 }, defaultSize: { w: 12, h: 10 }, countries: { max: Number.MAX_SAFE_INTEGER, all: true },
  options: OPTIONS,
  variants: choiceVariants(OPTIONS.find(field => field.key === 'view'), () => 'Diplomatic pacts'),
} as const satisfies WidgetMeta;

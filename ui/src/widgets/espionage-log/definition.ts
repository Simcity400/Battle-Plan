import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';
import { LOG_OPTIONS } from '../../charts/log/logOptions.ts';

export const OPTIONS: readonly OptionField[] = [
  { key: 'types', section: 'options', label: 'Operation types', hint: 'Names and artwork from this campaign’s content; none chosen shows every type', kind: 'content-set', automatic: 'Every type', default: '' },
  { key: 'countryRole', section: 'options', label: 'Match chosen countries as', kind: 'choice', default: 'owner',
    choices: [{ value: 'owner', label: 'Owner' }, { value: 'target', label: 'Target' }, { value: 'either', label: 'Owner or target' }] },
  { key: 'runningOrder', section: 'options', label: 'Running operations', hint: 'While Live, preparing and running operations precede finished operations', kind: 'choice', default: 'start',
    choices: [{ value: 'start', label: 'Newest start first' }, { value: 'preparation', label: 'Highest preparation first' }] },
  ...LOG_OPTIONS,
];
export const definition = {
  kind: 'espionage-log', version: 1, title: 'Espionage operations log', icon: 'target',
  description: 'Operations from every country: their target, preparation, selected phases and recorded ending.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'campaign', scope: 'history', data: true, history: true,
  countries: { max: Number.MAX_SAFE_INTEGER, all: true }, minSize: { w: 4, h: 6 }, defaultSize: { w: 9, h: 14 }, options: OPTIONS,
} as const satisfies WidgetMeta;

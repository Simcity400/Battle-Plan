import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';
import { LOG_OPTIONS } from '../../charts/log/logOptions.ts';

export const SHOW_OPTION: OptionField = { key: 'show', section: 'options', label: 'Show', kind: 'choice', default: 'both',
  choices: [{ value: 'both', label: 'Career and awards' }, { value: 'career', label: 'Career' }, { value: 'awards', label: 'Awards' }] };
export const OPTIONS: readonly OptionField[] = [
  { key: 'division', section: 'options', label: 'Division', kind: 'content-choice', automatic: 'Choose a division', default: '' },
  SHOW_OPTION, ...LOG_OPTIONS.filter(option => option.key !== 'chips'),
];
export const definition = {
  kind: 'division-service-record', version: 1, title: 'Division service record', icon: 'history', category: 'Army',
  description: 'One division’s recorded career and awards, newest first.',
  timeModes: ['campaign', 'follow', 'pin'], defaultTime: 'campaign', scope: 'history', data: true, history: true,
  countries: { max: Number.MAX_SAFE_INTEGER, all: true }, minSize: { w: 4, h: 5 }, defaultSize: { w: 6, h: 10 }, options: OPTIONS,
} as const satisfies WidgetMeta;

import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';
import { choiceVariants } from '../../dashboard/options.ts';

export const VIEWS = [{ value: 'live', label: 'Current' }, { value: 'history', label: 'History' }] as const;
export const OPTIONS: readonly OptionField[] = [
  { key: 'view', section: 'options', label: 'View', kind: 'choice', default: 'live', choices: VIEWS },
  { key: 'grow', section: 'axes', label: 'Grow with playback', kind: 'toggle', default: false,
    visibleWhen: options => options.view === 'history' },
];
export const definition = {
  kind: 'special-projects', version: 2, title: 'Special projects', description: 'Projects in progress with their scientists, completed projects, or each project’s progress over the campaign.',
  icon: 'flask', timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'follow', scope: 'history', data: true, history: true,
  countries: { max: Number.MAX_SAFE_INTEGER }, minSize: { w: 4, h: 5 }, defaultSize: { w: 7, h: 9 }, options: OPTIONS, viewSwitch: { option: 'view' }, playsBackDated: true,
  migrate: (_from, { options: { completedRetentionDays: _retention, ...options } = {}, ...widget }) => ({ ...widget,
    ...(Object.keys(options).length ? { options } : {}) }),
  variants: choiceVariants(OPTIONS.find(field => field.key === 'view'), () => 'Special project history'),
} as const satisfies WidgetMeta;

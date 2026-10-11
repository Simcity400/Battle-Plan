import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';

export const VIEWS = [{ value: 'live', label: 'Current' }, { value: 'history', label: 'History' }] as const;
export const OPTIONS: readonly OptionField[] = [
  { key: 'view', section: 'options', label: 'View', kind: 'choice', default: 'live', choices: VIEWS, visibleWhen: () => false },
  { key: 'completedRetentionDays', section: 'options', label: 'Keep completed projects', kind: 'number', default: 240,
    min: 0, max: 36500, step: 1, unit: 'game days', visibleWhen: options => options.view === 'live' },
  { key: 'grow', section: 'axes', label: 'Grow with playback', kind: 'toggle', default: false,
    visibleWhen: options => options.view === 'history' },
];
export const definition = {
  kind: 'special-projects', version: 1, title: 'Special projects', description: 'Projects, prototype progress and assigned scientists, or their campaign timeline.',
  icon: 'flask', timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'campaign', scope: 'history', data: true, history: true,
  countries: { max: Number.MAX_SAFE_INTEGER }, minSize: { w: 4, h: 5 }, defaultSize: { w: 7, h: 9 }, options: OPTIONS,
} as const satisfies WidgetMeta;

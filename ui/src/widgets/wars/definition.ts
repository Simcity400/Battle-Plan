import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';
import { choiceVariants } from '../../dashboard/options.ts';

export const OPTIONS: readonly OptionField[] = [
  { key: 'view', section: 'options', label: 'View', kind: 'choice', default: 'gantt',
    choices: [{ value: 'gantt', label: 'Timeline' }, { value: 'countries', label: 'Country by country' }] },
  { key: 'colour', section: 'options', label: 'Colour', kind: 'choice', default: 'war',
    choices: [{ value: 'war', label: 'By war' }, { value: 'side', label: 'By side' }],
    visibleWhen: options => options.view === 'countries' },
  { key: 'grow', section: 'axes', label: 'Grow with playback', kind: 'toggle', default: false },
];

export const definition = {
  kind: 'wars', version: 3, title: 'Wars timeline', icon: 'shield',
  description: 'Wars over time, with participants, capitulations and casualties.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'follow', scope: 'history', data: true, history: true,
  minSize: { w: 4, h: 5 }, defaultSize: { w: 8, h: 10 },
  countries: { max: 1 }, options: OPTIONS,
  factionColours: true,
  migrate: (_from, { options: { viewpoint, ...options } = {}, ...widget }) => ({ ...widget, options,
    countries: viewpoint === 'country' && widget.countries.kind === 'tags' && widget.countries.tags.length ? widget.countries : { kind: 'selected' } }),
  variants: choiceVariants(OPTIONS.find(field => field.key === 'view'), () => 'Wars by country'),
} as const satisfies WidgetMeta;

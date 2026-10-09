import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';

export const OPTIONS: readonly OptionField[] = [
  { key: 'view', section: 'options', label: 'View', kind: 'choice', default: 'gantt',
    choices: [{ value: 'gantt', label: 'Gantt of wars' }, { value: 'countries', label: 'Country by country' }] },
  { key: 'colour', section: 'options', label: 'Colour', kind: 'choice', default: 'war',
    choices: [{ value: 'war', label: 'By war' }, { value: 'side', label: 'By side' }],
    visibleWhen: options => options.view === 'countries' },
  { key: 'viewpoint', section: 'options', label: 'Viewpoint', kind: 'choice', default: 'player',
    choices: [{ value: 'player', label: "Recording’s player" }, { value: 'country', label: 'Picked country' }],
    hint: 'For a picked viewpoint, choose one country in Countries.' },
  { key: 'grow', section: 'axes', label: 'Grow with playback', kind: 'toggle', default: false,
    hint: 'With Live on, grow the time axis as the campaign plays.' },
];

export const definition = {
  kind: 'wars', version: 2, title: 'Wars timeline', icon: 'shield',
  description: 'The native war overview over time, with participants, capitulations and exact casualties.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'campaign', scope: 'history', data: true, history: true,
  minSize: { w: 4, h: 5 }, defaultSize: { w: 8, h: 10 },
  countries: { max: 1, fixed: true }, options: OPTIONS,
  defaultCountries: { kind: 'tags', tags: [] },
  migrate: (_from, widget) => ({ ...widget, countries: widget.countries.kind === 'selected' ? { kind: 'tags', tags: [] } : widget.countries }),
} as const satisfies WidgetMeta;

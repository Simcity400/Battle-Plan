import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';

export const SHOW_CHOICES = [{ value: 'share', label: 'Share of side' }, { value: 'totals', label: 'Totals' }] as const;

export const OPTIONS: readonly OptionField[] = [
  { key: 'war', section: 'options', label: 'War', kind: 'content-choice', default: '', automatic: 'The viewpoint’s main war' },
  { key: 'show', section: 'options', label: 'Show', kind: 'choice', default: 'share', choices: SHOW_CHOICES,
    hint: 'Totals scales each row to its own larger side: the terms have different units.' },
];

export const definition = {
  kind: 'war-contribution', version: 1, title: 'War contribution', icon: 'swords', category: 'Diplomacy',
  description: 'Who carried each side of a war: every member’s share of damage, ground, losses and lend-lease.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'follow', scope: 'history', data: true, history: true,
  minSize: { w: 6, h: 6 }, defaultSize: { w: 12, h: 10 },
  countries: { max: 1 }, options: OPTIONS, viewSwitch: { option: 'show' },
} as const satisfies WidgetMeta;

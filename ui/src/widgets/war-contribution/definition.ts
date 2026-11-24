import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';

export const VIEW_CHOICES = [
  { value: 'sides', label: 'Sides' },
  { value: 'measure', label: 'By measure' },
] as const;

export const SHOW_CHOICES = [
  { value: 'share', label: 'Share of side' },
  { value: 'totals', label: 'Amounts' },
] as const;

export const OPTIONS: readonly OptionField[] = [
  { key: 'view', section: 'options', label: 'View', kind: 'choice', default: 'sides', choices: VIEW_CHOICES },
  { key: 'war', section: 'options', label: 'War', kind: 'content-choice', default: '', automatic: 'The viewpoint’s main war' },
  {
    key: 'show',
    section: 'options',
    label: 'Show',
    kind: 'choice',
    default: 'share',
    choices: SHOW_CHOICES,
    hint: 'Amounts shows the raw score terms for each measure.',
  },
];

export const definition = {
  kind: 'war-contribution', version: 1, title: 'War contribution', icon: 'swords', category: 'Diplomacy',
  description: 'Who carried each side of a war: every member’s share of damage, ground, losses and lend-lease.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'follow', scope: 'history', data: true, history: true,
  playsBackDated: true,
  minSize: { w: 6, h: 6 }, defaultSize: { w: 12, h: 10 },
  countries: { max: 1 }, options: OPTIONS, viewSwitch: { option: 'view' },
} as const satisfies WidgetMeta;

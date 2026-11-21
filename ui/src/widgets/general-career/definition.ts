import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';
export const GROW_WITH_PLAYBACK_OPTION: OptionField = {
  key: 'growWithPlayback', section: 'time', label: 'Grow with playback', kind: 'toggle', default: false,
};
export const OPTIONS: readonly OptionField[] = [
  { key: 'leader', section: 'options', label: 'Leader', kind: 'content-choice', automatic: 'Top general', default: '', catalogue: true,
    hint: 'Automatic: the top army leader of this widget’s countries, ranked as the Generals leaderboard ranks by default' },
  GROW_WITH_PLAYBACK_OPTION,
];
export const definition = {
  kind: 'general-career', version: 1, title: 'General career', icon: 'history',
  description: 'A general or admiral’s skill, traits, commands and battles.',
  timeModes: ['campaign', 'follow', 'pin'], defaultTime: 'follow', scope: 'history', data: true, history: true,
  countries: { max: Number.MAX_SAFE_INTEGER, all: true },
  defaultCountries: { kind: 'selected' },
  minSize: { w: 4, h: 7 }, defaultSize: { w: 6, h: 10 }, options: OPTIONS,
} as const satisfies WidgetMeta;

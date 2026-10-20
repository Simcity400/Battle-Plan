import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';
import { REACHED_COUNTRY_OPTION } from '../../dashboard/reachedCountry.ts';
import { TABLE_OPTIONS } from '../../charts/table/options.ts';

export const OPTIONS: readonly OptionField[] = [REACHED_COUNTRY_OPTION, ...TABLE_OPTIONS.filter(option => option.key !== 'rankMarkers')];
export const definition = {
  kind: 'training-queue', version: 1, title: 'Training queue', icon: 'helmet', category: 'Army',
  description: 'Divisions in training under each deployment line, with training, equipment and manpower.',
  timeModes: ['campaign', 'follow', 'pin'], defaultTime: 'follow', scope: 'moment', data: true, history: true,
  countries: { max: 1 }, minSize: { w: 4, h: 5 }, defaultSize: { w: 7, h: 9 }, options: OPTIONS,
} as const satisfies WidgetMeta;

import type { WidgetMeta } from './definitions';
import { sectionFields } from './options.ts';
import type { OptionValue } from './options';

/** Every widget's settings panel has these sections, always in this order. */
export const SETTINGS_SECTIONS = ['campaign', 'countries', 'time', 'size', 'options', 'axes', 'interaction', 'reset'] as const;
export type SettingsSection = typeof SETTINGS_SECTIONS[number];

export const SECTION_TITLES: Record<SettingsSection, string> = {
  campaign: 'Campaign', countries: 'Countries', time: 'Time', size: 'Size',
  options: 'Options', axes: 'Axes', interaction: 'Interaction', reset: 'Reset',
};

/** The sections this widget shows: one appears only when it has something to set. */
export function settingsSections(meta: WidgetMeta | undefined, resolved: Readonly<Record<string, OptionValue>>, canReset: boolean): SettingsSection[] {
  const fields = meta?.options;
  const reads = meta?.data !== false;
  const timeModes = meta?.history ? meta.timeModes.length > 1 : false;
  const has: Record<SettingsSection, boolean> = {
    campaign: reads,
    countries: reads,
    time: timeModes || sectionFields(fields, 'time', resolved).length > 0,
    size: true,
    options: sectionFields(fields, 'options', resolved).length > 0,
    axes: sectionFields(fields, 'axes', resolved).length > 0,
    interaction: sectionFields(fields, 'interaction', resolved).length > 0,
    reset: canReset,
  };
  return SETTINGS_SECTIONS.filter(section => has[section]);
}

import type { WidgetMeta } from './definitions';
import { sectionFields } from './options.ts';
import type { OptionValue } from './options';

/** Every widget's settings panel has these sections, always in this order. */
export const SETTINGS_SECTIONS = ['campaign', 'countries', 'time', 'size', 'options', 'axes', 'interaction'] as const;
export type SettingsSection = typeof SETTINGS_SECTIONS[number];

export const SECTION_TITLES: Record<SettingsSection, string> = {
  campaign: 'Campaign', countries: 'Countries', time: 'Time', size: 'Size',
  options: 'Options', axes: 'Axes', interaction: 'Interaction',
};

/** The sections this widget shows: one appears only when it has something to set. */
export function settingsSections(meta: WidgetMeta | undefined, resolved: Readonly<Record<string, OptionValue>>): SettingsSection[] {
  const fields = meta?.options;
  const reads = meta?.data !== false;
  const timeModes = reads && (meta?.timeModes.length ?? 0) > 1;
  const has: Record<SettingsSection, boolean> = {
    campaign: reads,
    countries: reads && meta?.countrySelection !== false,
    time: timeModes || sectionFields(fields, 'time', resolved).length > 0,
    size: true,
    options: sectionFields(fields, 'options', resolved).length > 0,
    axes: sectionFields(fields, 'axes', resolved).length > 0,
    interaction: sectionFields(fields, 'interaction', resolved).length > 0,
  };
  return SETTINGS_SECTIONS.filter(section => has[section]);
}

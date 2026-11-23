import type { WidgetMeta } from './definitions';
import { sectionFields } from './options.ts';
import type { OptionSection, OptionValue } from './options';
import type { CountrySelection, WidgetTime } from './model';

export const SETTINGS_SECTIONS = ['campaign', 'countries', 'time', 'size', 'options', 'map', 'factionColours', 'axes', 'interaction'] as const;
export type SettingsSection = typeof SETTINGS_SECTIONS[number];

export const SECTION_TITLES: Record<SettingsSection, string> = {
  campaign: 'Campaign', countries: 'Countries', time: 'Time', size: 'Size',
  options: 'Options', map: 'Map', factionColours: 'Colours', axes: 'Axes', interaction: 'Interaction',
};

export function settingsSections(meta: WidgetMeta | undefined, resolved: Readonly<Record<string, OptionValue>>,
  countries?: CountrySelection, time?: WidgetTime['mode']): SettingsSection[] {
  const fields = meta?.options;
  const shown = (section: OptionSection) => sectionFields(fields, section, resolved, countries, time).length > 0;
  const reads = meta?.data !== false;
  const timeModes = reads && (meta?.timeModes.length ?? 0) > 1;
  const has: Record<SettingsSection, boolean> = {
    campaign: reads,
    countries: reads && meta?.countrySelection !== false,
    time: timeModes || shown('time'),
    size: true,
    options: shown('options'),
    map: meta?.kind === 'map',
    factionColours: reads && meta?.factionColours === true,
    axes: shown('axes'),
    interaction: shown('interaction'),
  };
  return SETTINGS_SECTIONS.filter(section => has[section]);
}

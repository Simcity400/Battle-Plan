import type { OptionField } from './options';

/** The faction widgets show the chosen country's faction at the shown hour, or any
 * other faction picked here. */
export const FACTION_OPTION: OptionField = { key: 'faction', section: 'options', label: 'Faction', kind: 'content-choice',
  default: '', automatic: 'The country’s faction' };

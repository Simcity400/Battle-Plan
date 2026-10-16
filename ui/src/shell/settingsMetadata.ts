import type { IconName } from '../components/Icons';
export type SettingsSection = 'folders' | 'appearance' | 'graphics' | 'preparation' | 'diagnostics';

export const settingsSections: { id: SettingsSection; label: string; icon: IconName; description: string }[] = [
  { id: 'folders', label: 'Folders', icon: 'folder', description: 'Where the game, your Paradox user files and your recordings live.' },
  { id: 'appearance', label: 'Appearance', icon: 'palette', description: 'Themes, backdrop, glass and typography. Saved on this device.' },
  { id: 'graphics', label: 'Map graphics', icon: 'map', description: 'Defaults for every campaign map. Campaigns and individual map widgets can override them.' },
  { id: 'preparation', label: 'Preparation', icon: 'sparkle', description: 'Background preparation for opened recordings. Saved on this device.' },
  { id: 'diagnostics', label: 'Diagnostics', icon: 'info', description: 'How complete the open recording is, and tools for when the map misbehaves.' },
];

/** A search result: a section, and the row to reveal by element id or by its visible label. */
export interface SettingsTarget { section: SettingsSection; title: string; id?: string; keywords?: string }

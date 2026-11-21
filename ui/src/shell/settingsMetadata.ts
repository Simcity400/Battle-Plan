import type { IconName } from '../components/Icons';
export type SettingsSection = 'folders' | 'appearance' | 'graphics' | 'updates' | 'diagnostics';

export const settingsSections: { id: SettingsSection; label: string; icon: IconName; description: string }[] = [
  { id: 'folders', label: 'Folders', icon: 'folder', description: 'Where the game, your Paradox user files and your recordings live, and how new recordings are stored.' },
  { id: 'appearance', label: 'Appearance', icon: 'palette', description: 'Themes, backdrop, glass, charts, typography and analytics pages. Saved on this device.' },
  { id: 'graphics', label: 'Map', icon: 'map', description: 'Graphics and sound for every campaign map.' },
  { id: 'updates', label: 'Updates', icon: 'download', description: 'Check for new versions and choose which releases you get.' },
  { id: 'diagnostics', label: 'Diagnostics', icon: 'info', description: 'How complete the open campaign is, and tools for when the map misbehaves.' },
];

export interface SettingsTarget { section: SettingsSection; title: string; id?: string; fallback?: string; keywords?: string }

import type { IconName } from '../components/Icons';
export type SettingsSection = 'folders' | 'appearance' | 'graphics' | 'recording' | 'updates' | 'diagnostics';

export const settingsSections: { id: SettingsSection; label: string; icon: IconName; description: string }[] = [
  { id: 'folders', label: 'Folders', icon: 'folder', description: 'Where the game, your Paradox user files and your recordings live.' },
  { id: 'appearance', label: 'Appearance', icon: 'palette', description: 'Themes, backdrop, glass and typography. Saved on this device.' },
  { id: 'graphics', label: 'Map graphics', icon: 'map', description: 'Defaults for every campaign map. Campaigns and individual map widgets can override them.' },
  { id: 'recording', label: 'Recording', icon: 'file', description: 'Compression while recording. Saved on this device.' },
  { id: 'updates', label: 'Updates', icon: 'refresh-cw', description: 'The version of Battle Plan and Battle Plan Recorder, which update together, and the channel they follow.' },
  { id: 'diagnostics', label: 'Diagnostics', icon: 'info', description: 'How complete the open recording is, and tools for when the map misbehaves.' },
];

export interface SettingsTarget { section: SettingsSection; title: string; id?: string; keywords?: string }

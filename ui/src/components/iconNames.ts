export const ICON_NAMES = [
  'plane', 'walk', 'map', 'globe', 'grid', 'table', 'chart-bars', 'chart-line', 'parliament', 'handshake', 'flag', 'eye', 'layout', 'settings', 'folder', 'folder-open',
  'file', 'arrow', 'refresh-cw', 'refresh', 'info', 'alert', 'close', 'check', 'crown', 'trash', 'calendar', 'clock', 'link', 'external', 'edit', 'save', 'plus', 'minus',
  'chevron-left', 'chevron-right', 'chevron-down', 'chevron-up', 'chevrons-left', 'chevrons-right', 'sidebar', 'search', 'star', 'star-filled',
  'copy', 'download', 'upload', 'image', 'palette', 'type', 'sun', 'moon', 'monitor', 'gamepad', 'sliders', 'grip', 'more', 'eject', 'pin',
  'target', 'flask', 'sparkle', 'play', 'volume', 'volume-off', 'pause', 'layers', 'history', 'helmet', 'anchor', 'droplet', 'hammer',
  'sunburst', 'factory', 'flows', 'shield', 'hierarchy', 'swords', 'flag-torn', 'flag-plus', 'flag-x', 'olive-branch', 'users-plus',
  'users-minus', 'shield-check', 'ballot', 'lock',
] as const;

export type IconName = typeof ICON_NAMES[number];

const names: ReadonlySet<string> = new Set(ICON_NAMES);
export const isIconName = (value: unknown): value is IconName => typeof value === 'string' && names.has(value);

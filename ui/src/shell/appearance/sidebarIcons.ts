import type { IconStyles } from './iconStyle.ts';

export const SIDEBAR_ICONS = {
  analytics: { label: 'Analytics', dark: '#a78bfa', light: '#7c3aed' },
  map: { label: 'Campaign map', dark: '#34d399', light: '#047857' },
  settings: { label: 'Settings', dark: '#94a3b8', light: '#475569' },
  folders: { label: 'Folders', dark: '#fbbf24', light: '#a16207' },
  appearance: { label: 'Appearance', dark: '#f472b6', light: '#be185d' },
  graphics: { label: 'Map graphics', dark: '#38bdf8', light: '#0369a1' },
  recording: { label: 'Recording', dark: '#94a3b8', light: '#475569' },
  diagnostics: { label: 'Diagnostics', dark: '#fb923c', light: '#c2410c' },
  overview: { label: 'Overview', dark: '#a78bfa', light: '#7c3aed' },
  politics: { label: 'Politics', dark: '#f472b6', light: '#be185d' },
  diplomacy: { label: 'Diplomacy', dark: '#2dd4bf', light: '#0f766e' },
  economy: { label: 'Economy', dark: '#fbbf24', light: '#a16207' },
  research: { label: 'Research', dark: '#c084fc', light: '#9333ea' },
  land: { label: 'Army', dark: '#a3b763', light: '#4d5e1a' },
  air: { label: 'Air', dark: '#7dd3fc', light: '#0284c7' },
  navy: { label: 'Navy', dark: '#3b82f6', light: '#1e40af' },
  occupation: { label: 'Occupation', dark: '#f87171', light: '#b91c1c' },
  intelligence: { label: 'Intelligence', dark: '#e879f9', light: '#a21caf' },
} as const;

export type SidebarIconId = keyof typeof SIDEBAR_ICONS | `page-${string}`;

export const isSidebarIconId = (id: string): id is SidebarIconId => Object.hasOwn(SIDEBAR_ICONS, id) || /^page-[a-zA-Z0-9_-]{1,60}$/.test(id);
export const defaultSidebarIcon = (id: SidebarIconId) => SIDEBAR_ICONS[id as keyof typeof SIDEBAR_ICONS] ?? SIDEBAR_ICONS.overview;
export type SidebarIconStyles = IconStyles<SidebarIconId>;

export const sidebarIconIds = Object.keys(SIDEBAR_ICONS) as (keyof typeof SIDEBAR_ICONS)[];

import { useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import type { KeyboardEvent } from 'react';
import { Icon } from '../components/Icons';
import type { IconName } from '../components/Icons';
import ReplayFolderSetting from './ReplayFolderSetting';
import type { DashboardClient } from '../dashboard/client';
import type { ReplayLibraryState } from '../dashboard/replayLibrary';
import MapDiagnostics from './MapDiagnostics';
import MapAppearance, { MAP_OPTION_LABELS } from '../components/MapAppearance';
import { VIDEO_OPTION_LABELS } from './VideoSettings';
import GameFolderSettings from './GameFolderSettings';
import type { ContentFolders, DesktopState, ReplayMetadata } from './api';
import AppearanceSettings, { newThemeId, SettingsBlock } from './appearance/AppearanceSettings';
import ThemeEditor from './appearance/ThemeEditor';
import type { ThemeDraft } from './appearance/ThemeEditor';
import { BUILT_IN_THEMES } from './appearance/themes';
import { uniqueThemeName } from './appearance/themeFile';
import type { Appearance } from './appearance/useAppearance';
import './settings.css';

export type SettingsSection = 'folders' | 'appearance' | 'graphics' | 'diagnostics';

export const settingsSections: { id: SettingsSection; label: string; icon: IconName; description: string }[] = [
  { id: 'folders', label: 'Folders', icon: 'folder', description: 'Where the game, your Paradox user files and your recordings live.' },
  { id: 'appearance', label: 'Appearance', icon: 'palette', description: 'Themes, backdrop, glass and typography. Saved on this device.' },
  { id: 'graphics', label: 'Map graphics', icon: 'map', description: 'Defaults for every campaign map. Campaigns and individual map widgets can override them.' },
  { id: 'diagnostics', label: 'Diagnostics', icon: 'info', description: 'Recording coverage and the most recent native map report.' },
];

/** A search result: a section, and the row to reveal by element id or by its visible label. */
export interface SettingsTarget { section: SettingsSection; title: string; id?: string; keywords?: string }

const searchItems: SettingsTarget[] = [
  { section: 'folders', title: 'the game installation', keywords: 'game folder install steam path' },
  { section: 'folders', title: 'Paradox user folder', keywords: 'documents mods user path' },
  { section: 'folders', title: 'Replay folder', keywords: 'recordings library replays path' },
  { section: 'appearance', title: 'Colour scheme', id: 'appearance-scheme', keywords: 'color dark light system mode' },
  { section: 'appearance', title: 'Themes', id: 'appearance-themes', keywords: 'theme palette colours colors accent' },
  { section: 'appearance', title: 'Import theme', id: 'appearance-themes', keywords: 'vs code vscode json file paste' },
  { section: 'appearance', title: 'Create theme', id: 'appearance-themes', keywords: 'custom editor colour color' },
  { section: 'appearance', title: 'Backdrop', id: 'appearance-backdrop', keywords: 'background image wallpaper glow ambient' },
  { section: 'appearance', title: 'Glass', id: 'appearance-glass', keywords: 'blur opacity transparency frosted' },
  { section: 'appearance', title: 'Corner radius', id: 'appearance-radius', keywords: 'rounding rounded corners' },
  { section: 'appearance', title: 'Motion', id: 'appearance-motion', keywords: 'animation reduced transitions' },
  { section: 'appearance', title: 'Interface font', id: 'appearance-interface-font', keywords: 'typeface font inter typography' },
  { section: 'appearance', title: 'Display font', id: 'appearance-display-font', keywords: 'typeface headings typography' },
  ...[...MAP_OPTION_LABELS.slice(0, 2), ...VIDEO_OPTION_LABELS, ...MAP_OPTION_LABELS.slice(2)]
    .map(title => ({ section: 'graphics' as const, title, keywords: title === MAP_OPTION_LABELS[0] ? 'msaa' : title === VIDEO_OPTION_LABELS[1] ? 'zoom ui size' : 'map graphics' })),
  { section: 'graphics', title: 'Visual mods', keywords: 'mods artwork' },
  { section: 'diagnostics', title: 'Recording', id: 'diagnostics-recording', keywords: 'gaps branches coverage format' },
  { section: 'diagnostics', title: 'Native map', id: 'diagnostics-map', keywords: 'viewer report' },
];

function search(query: string): SettingsTarget[] {
  const words = query.trim().toLowerCase().split(/\s+/).filter(Boolean);
  if (!words.length) return [];
  return searchItems.filter(item => {
    const text = `${item.title} ${settingsSections.find(section => section.id === item.section)!.label} ${item.keywords ?? ''}`.toLowerCase();
    return words.every(word => text.includes(word));
  });
}

/** The row a target names, by id or by the label text a settings row shows. */
function findTarget(root: HTMLElement, target: SettingsTarget): HTMLElement | null {
  if (target.id) return root.querySelector<HTMLElement>(`#${CSS.escape(target.id)}`);
  const labels = root.querySelectorAll<HTMLElement>('.folder-field-title, .map-option-row > span:first-child, .map-visual-mods > summary, .settings-row-text > strong');
  const label = [...labels].find(element => element.textContent?.trim().toLowerCase().startsWith(target.title.toLowerCase()));
  return label?.closest<HTMLElement>('.folder-setting-row, .compact-folder-form, .map-option-row, .map-visual-mods, .settings-row') ?? null;
}

/** The sidebar while Settings is open: back, search, then the sections. */
export function SettingsNav({ section, onBack, onSection, onTarget, disabled }: {
  section: SettingsSection; onBack: () => void; onSection: (section: SettingsSection) => void; onTarget: (target: SettingsTarget) => void; disabled?: boolean;
}) {
  const [query, setQuery] = useState('');
  const [active, setActive] = useState(0);
  const input = useRef<HTMLInputElement>(null);
  const results = useMemo(() => search(query), [query]);
  const searching = query.trim().length > 0;
  useEffect(() => setActive(index => Math.min(index, Math.max(results.length - 1, 0))), [results.length]);
  useEffect(() => {
    const focus = (event: globalThis.KeyboardEvent) => {
      const typing = event.target instanceof HTMLElement && event.target.closest('input, textarea, select, [contenteditable]');
      if (event.key !== '/' || typing || event.ctrlKey || event.metaKey || event.altKey) return;
      event.preventDefault();
      input.current?.focus();
    };
    window.addEventListener('keydown', focus);
    return () => window.removeEventListener('keydown', focus);
  }, []);
  const choose = (target: SettingsTarget) => { onTarget(target); setQuery(''); };
  const keys = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.key === 'Escape' && query) { event.preventDefault(); event.stopPropagation(); setQuery(''); }
    if (!results.length) return;
    if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
      event.preventDefault();
      setActive(index => (index + (event.key === 'ArrowDown' ? 1 : -1) + results.length) % results.length);
    }
    if (event.key === 'Enter') { event.preventDefault(); choose(results[active]); }
  };
  return <>
    <nav className="sidebar-menu" aria-label="Settings">
      <button type="button" className="sidebar-row" disabled={disabled} onClick={onBack}><Icon name="arrow" className="is-back" /><span>Back</span></button>
      <label className="sidebar-search">
        <Icon name="search" />
        <input ref={input} type="search" value={query} placeholder="Search settings" aria-label="Search settings" role="combobox" aria-autocomplete="list"
          aria-expanded={searching && results.length > 0} aria-controls={searching && results.length ? 'settings-search-results' : undefined}
          aria-activedescendant={searching && results[active] ? `settings-result-${active}` : undefined}
          onChange={event => { setQuery(event.currentTarget.value); setActive(0); }} onKeyDown={keys} />
        {searching ? <button type="button" className="icon-button small" aria-label="Clear search" onClick={() => { setQuery(''); input.current?.focus(); }}><Icon name="close" /></button>
          : <kbd aria-hidden="true">/</kbd>}
      </label>
    </nav>
    {searching ? <div className="sidebar-menu" id="settings-search-results" role="listbox" aria-label="Settings search results">
      {results.map((item, index) => <button key={`${item.section}:${item.title}`} id={`settings-result-${index}`} type="button" role="option" tabIndex={-1}
        aria-selected={index === active} className="sidebar-row sidebar-result" onMouseMove={() => setActive(index)} onClick={() => choose(item)}>
        <Icon name={settingsSections.find(section => section.id === item.section)!.icon} />
        <span><strong>{item.title}</strong><small>{settingsSections.find(section => section.id === item.section)!.label}</small></span>
      </button>)}
      {!results.length && <p className="sidebar-empty" role="status">No settings found</p>}
    </div> : <nav className="sidebar-menu" aria-label="Settings sections">
      {settingsSections.map(item => <button key={item.id} type="button" className="sidebar-row" aria-current={section === item.id ? 'page' : undefined}
        onClick={() => onSection(item.id)}><Icon name={item.icon} /><span>{item.label}</span></button>)}
    </nav>}
  </>;
}

export default function Settings({ section, target, onTargetShown, appearance, library, client, native, metadata, folders, folderError, onFoldersChange }: {
  section: SettingsSection; target: SettingsTarget | null; onTargetShown: () => void;
  folders: ContentFolders | null; folderError: string | null; onFoldersChange: (folders: ContentFolders) => void; native: DesktopState | null; metadata: ReplayMetadata | null;
  library: ReplayLibraryState; client: DashboardClient; appearance: Appearance;
}) {
  const page = useRef<HTMLDivElement>(null);
  const [editor, setEditor] = useState<ThemeDraft | null>(null);
  const details = settingsSections.find(item => item.id === section)!;
  useLayoutEffect(() => { page.current?.scrollTo(0, 0); }, [section]);
  useEffect(() => { if (section !== 'appearance') setEditor(null); }, [section]);
  useEffect(() => {
    if (!target || target.section !== section) return;
    // Map graphics arrive asynchronously; look for the row for a short while.
    let attempts = 0;
    let timer: ReturnType<typeof setTimeout>;
    const reveal = () => {
      const element = page.current && findTarget(page.current, target);
      if (!element) { if (++attempts < 20) timer = setTimeout(reveal, 80); else onTargetShown(); return; }
      element.scrollIntoView({ block: 'center', behavior: appearance.resolved.reducedMotion ? 'auto' : 'smooth' });
      element.classList.remove('is-search-target');
      void element.offsetWidth;
      element.classList.add('is-search-target');
      onTargetShown();
    };
    timer = setTimeout(reveal, 0);
    return () => clearTimeout(timer);
  }, [target, section, onTargetShown, appearance.resolved.reducedMotion]);
  const saveTheme = (draft: ThemeDraft) => {
    const id = draft.id ?? newThemeId();
    const others = [...BUILT_IN_THEMES, ...appearance.settings.custom].filter(theme => theme.id !== id).map(theme => theme.name);
    const theme = { id, name: uniqueThemeName(draft.name, others), variants: draft.variants };
    appearance.update(previous => {
      const custom = previous.custom.some(item => item.id === id) ? previous.custom.map(item => item.id === id ? theme : item) : [...previous.custom, theme];
      const pick = (mode: 'dark' | 'light') => theme.variants[mode] ? id : previous.themes[mode] === id ? BUILT_IN_THEMES[0].id : previous.themes[mode];
      const scheme = previous.scheme !== 'system' && !theme.variants[previous.scheme] ? (theme.variants.dark ? 'dark' : 'light') : previous.scheme;
      return { ...previous, custom, scheme, themes: { dark: pick('dark'), light: pick('light') } };
    });
    setEditor(null);
  };
  return <div className={`settings-layout${editor ? ' is-editing-theme' : ''}`}>
    <div className="settings-page" ref={page}>
      <div className="settings-content">
        <header className="settings-page-header"><h1>{details.label}</h1><p>{details.description}</p></header>
        {section === 'folders' && <>
          {folders && !folders.contentReady && <div className="notice" role="status"><Icon name="sparkle" /><span>Welcome to Battle Plan. Choose your game installation and Paradox user folders to load the game’s original assets and mods.</span></div>}
          <SettingsBlock id="folders-game" title="Game">
            <GameFolderSettings folders={folders} native={native} client={client} onFoldersChange={onFoldersChange} />
          </SettingsBlock>
          <SettingsBlock id="folders-recordings" title="Recordings">
            <ReplayFolderSetting compact library={library} client={client} />
          </SettingsBlock>
          {folderError && <p className="map-menu-error" role="alert">{folderError}</p>}
        </>}
        {section === 'appearance' && <AppearanceSettings appearance={appearance} onEdit={setEditor} />}
        {section === 'graphics' && <SettingsBlock id="graphics-defaults" title="Defaults">
          <div className="global-map-settings">
            <MapAppearance key={`${folders?.game ?? ''}:${folders?.user ?? ''}`} global source={null} client={client} userFolder={folders?.user} />
          </div>
        </SettingsBlock>}
        {section === 'diagnostics' && <MapDiagnostics native={native} metadata={metadata} />}
      </div>
    </div>
    {editor && <ThemeEditor key={editor.id ?? 'new'} draft={editor} onSave={saveTheme} onClose={() => setEditor(null)} onPreview={appearance.setPreview} />}
  </div>;
}

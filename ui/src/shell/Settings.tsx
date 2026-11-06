import { useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import type { KeyboardEvent } from 'react';
import { Icon } from '../components/Icons';
import ReplayFolderSetting from './ReplayFolderSetting';
import type { DashboardClient } from '../dashboard/client';
import type { ReplayLibraryState } from '../dashboard/replayLibrary';
import MapDiagnostics from './MapDiagnostics';
import MapAppearance, { MAP_OPTION_LABELS } from '../components/MapAppearance';
import { VIDEO_OPTION_LABELS } from './VideoSettings';
import GameFolderSettings from './GameFolderSettings';
import PreparationSetting from './PreparationSetting';
import RecordingCompressionSetting from './RecordingCompressionSetting';
import type { ContentFolders, DesktopState, ReplayMetadata } from './api';
import AppearanceSettings, { newThemeId, SettingsBlock } from './appearance/AppearanceSettings';
import ThemeEditor from './appearance/ThemeEditor';
import type { ThemeDraft } from './appearance/ThemeEditor';
import { BUILT_IN_THEMES } from './appearance/themes';
import { uniqueThemeName } from './appearance/themeFile';
import { MAX_CUSTOM_THEMES } from './appearance/settings';
import type { Appearance } from './appearance/useAppearance';
import { StyledIcon } from './appearance/StyledIcon';
import { isSidebarIconId } from './appearance/sidebarIcons';
import type { SidebarIconStyles } from './appearance/sidebarIcons';
import type { BranchChoice } from '../dashboard/Dashboard';
import './settings.css';

export { settingsSections } from './settingsMetadata';
export type { SettingsSection, SettingsTarget } from './settingsMetadata';
import { settingsSections } from './settingsMetadata';
import type { SettingsSection, SettingsTarget } from './settingsMetadata';

const searchItems: SettingsTarget[] = [
  { section: 'preparation', title: 'Prepare recordings in the background', id: 'preparation-background', keywords: 'automatic cache background preparation enable disable stop' },
  { section: 'recording', title: 'Recording compression', id: 'recording-compression', keywords: 'low high live compression cpu stop save size' },
  { section: 'folders', title: 'Game installation', keywords: 'game folder install steam path' },
  { section: 'folders', title: 'Paradox user folder', keywords: 'documents mods user path' },
  { section: 'folders', title: 'Replay folder', keywords: 'recordings library replays path' },
  { section: 'appearance', title: 'Colour scheme', id: 'appearance-scheme', keywords: 'color dark light system mode' },
  { section: 'appearance', title: 'Themes', id: 'appearance-themes', keywords: 'theme palette colours colors accent' },
  { section: 'appearance', title: 'Import theme', id: 'appearance-themes', keywords: 'vs code vscode json file paste' },
  { section: 'appearance', title: 'Create theme', id: 'appearance-themes', keywords: 'custom editor colour color' },
  { section: 'appearance', title: 'Backdrop', id: 'appearance-backdrop', keywords: 'background image wallpaper glow ambient sidebar top bar main view' },
  { section: 'appearance', title: 'Contrast', id: 'appearance-contrast', keywords: 'colours colors borders text readability' },
  { section: 'appearance', title: 'Main view glass', id: 'appearance-glass', keywords: 'glass blur opacity transparency frosted panels widgets' },
  { section: 'appearance', title: 'Sidebar and top bar glass', id: 'appearance-chrome-glass', keywords: 'glass blur opacity transparency frosted sidebar header top bar' },
  { section: 'appearance', title: 'Corner radius', id: 'appearance-radius', keywords: 'rounding rounded corners' },
  { section: 'appearance', title: 'Motion', id: 'appearance-motion', keywords: 'animation reduced transitions' },
  { section: 'appearance', title: 'Chart motion', id: 'appearance-chart-motion', keywords: 'charts animation easing playback smooth cinematic snappy' },
  { section: 'appearance', title: 'Top-bar flag', id: 'appearance-header-flag', keywords: 'flag header country size native large smooth' },
  { section: 'appearance', title: 'Country colours', id: 'appearance-country-colours', keywords: 'charts map colours colors contrast raw adjusted' },
  { section: 'appearance', title: 'Interface font', id: 'appearance-interface-font', keywords: 'typeface font inter typography size' },
  { section: 'appearance', title: 'Reset analytics pages', id: 'appearance-pages', keywords: 'pages tabs layouts defaults restore analytics' },
  { section: 'appearance', title: 'Headings font', id: 'appearance-display-font', keywords: 'typeface display headings typography size advanced' },
  { section: 'appearance', title: 'Tables font', id: 'appearance-tables-font', keywords: 'typeface monospace numeric typography size' },
  { section: 'appearance', title: 'Diagnostics font', id: 'appearance-diagnostics-font', keywords: 'typeface monospace report typography size advanced' },
  ...(/mac|iphone|ipad|ipod/i.test(navigator.platform) ? [{ section: 'appearance' as const, title: 'Font smoothing', id: 'appearance-font-smoothing', keywords: 'typography antialias grayscale advanced' }] : []),
  { section: 'appearance', title: 'Word wrapping', id: 'appearance-word-wrap', keywords: 'tables text lines wrap' },
  ...[...MAP_OPTION_LABELS.slice(0, 2), ...VIDEO_OPTION_LABELS, ...MAP_OPTION_LABELS.slice(2)]
    .map(title => ({ section: 'graphics' as const, title, keywords: title === MAP_OPTION_LABELS[0] ? 'msaa' : title === VIDEO_OPTION_LABELS[1] ? 'zoom ui size' : title === VIDEO_OPTION_LABELS[2] ? 'counters icons labels size gui_scale' : 'map graphics' })),
  { section: 'graphics', title: 'Visual mods', keywords: 'mods artwork' },
  { section: 'diagnostics', title: 'Technical overlay', id: 'diagnostics-overlay', keywords: 'diagnostics stats frame rate debug overlay f3 map' },
  { section: 'diagnostics', title: 'Recording', id: 'diagnostics-recording', keywords: 'gaps branches coverage complete stopped early notes missing' },
  { section: 'diagnostics', title: 'Map problems', id: 'diagnostics-map', keywords: 'map report viewer problems errors' },
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
  const title = target.title.toLowerCase();
  const text = (element: HTMLElement) => element.textContent?.trim().toLowerCase() ?? '';
  const label = [...labels].find(element => text(element) === title) ?? [...labels].find(element => text(element).startsWith(title));
  return label?.closest<HTMLElement>('.folder-setting-row, .compact-folder-form, .map-option-row, .map-visual-mods, .settings-row') ?? null;
}

/** The sidebar while Settings is open: back, search, then the sections. */
export function SettingsNav({ section, icons, onBack, onSection, onTarget, disabled }: {
  section: SettingsSection; icons: SidebarIconStyles; onBack: () => void; onSection: (section: SettingsSection) => void; onTarget: (target: SettingsTarget) => void; disabled?: boolean;
}) {
  const [query, setQuery] = useState('');
  const [active, setActive] = useState(0);
  const input = useRef<HTMLInputElement>(null);
  const results = useMemo(() => search(query), [query]);
  const searching = query.trim().length > 0;
  const sectionIcon = (id: SettingsSection) => <StyledIcon icon={isSidebarIconId(id) ? icons[id]?.icon : undefined}
    fallback={settingsSections.find(item => item.id === id)!.icon} colour={`var(--sidebar-icon-${id})`} />;
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
        aria-selected={index === active} className="sidebar-row sidebar-result" data-sidebar-icon={item.section} onMouseMove={() => setActive(index)} onClick={() => choose(item)}>
        {sectionIcon(item.section)}
        <span><strong>{item.title}</strong><small>{settingsSections.find(section => section.id === item.section)!.label}</small></span>
      </button>)}
      {!results.length && <p className="sidebar-empty" role="status">No settings found</p>}
    </div> : <nav className="sidebar-menu" aria-label="Settings sections">
      {settingsSections.map(item => <button key={item.id} type="button" className="sidebar-row" data-sidebar-icon={item.id} aria-current={section === item.id ? 'page' : undefined}
        onClick={() => onSection(item.id)}>{sectionIcon(item.id)}<span>{item.label}</span></button>)}
    </nav>}
  </>;
}

export default function Settings({ section, target, onTargetShown, appearance, library, client, native, metadata, branch, folders, folderError, onFoldersChange, onResetPages }: {
  section: SettingsSection; target: SettingsTarget | null; onTargetShown: () => void;
  folders: ContentFolders | null; folderError: string | null; onFoldersChange: (folders: ContentFolders) => void; native: DesktopState | null; metadata: ReplayMetadata | null; branch: BranchChoice | null;
  library: ReplayLibraryState; client: DashboardClient; appearance: Appearance; onResetPages?: (() => void) | null;
}) {
  const page = useRef<HTMLDivElement>(null);
  const [editor, setEditor] = useState<ThemeDraft | null>(null);
  // Unsaved new themes have no id, so each one counts as its own editing session.
  const [newSession, setNewSession] = useState(0);
  const editTheme = (draft: ThemeDraft) => { setEditor(draft); if (draft.id === null) setNewSession(session => session + 1); };
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
      // Advanced map parts sit in folded sections; open the one holding the target.
      const folded = element.closest('details');
      if (folded) folded.open = true;
      element.scrollIntoView({ block: 'center', behavior: appearance.resolved.reducedMotion ? 'auto' : 'smooth' });
      element.classList.remove('is-search-target');
      void element.offsetWidth;
      element.classList.add('is-search-target');
      onTargetShown();
    };
    timer = setTimeout(reveal, 0);
    return () => clearTimeout(timer);
  }, [target, section, onTargetShown, appearance.resolved.reducedMotion]);
  const saveTheme = (draft: ThemeDraft): string | null => {
    const id = draft.id ?? newThemeId();
    // A new theme past the limit is refused here, rather than dropped when next loaded.
    if (!appearance.settings.custom.some(item => item.id === id) && appearance.settings.custom.length >= MAX_CUSTOM_THEMES)
      return `You can keep up to ${MAX_CUSTOM_THEMES} custom themes. Delete one first.`;
    const others = [...BUILT_IN_THEMES, ...appearance.settings.custom].filter(theme => theme.id !== id).map(theme => theme.name);
    const theme = { id, name: uniqueThemeName(draft.name, others), variants: draft.variants };
    appearance.update(previous => {
      const custom = previous.custom.some(item => item.id === id) ? previous.custom.map(item => item.id === id ? theme : item) : [...previous.custom, theme];
      const pick = (mode: 'dark' | 'light') => theme.variants[mode] ? id : previous.themes[mode] === id ? BUILT_IN_THEMES[0].id : previous.themes[mode];
      const scheme = previous.scheme !== 'system' && !theme.variants[previous.scheme] ? (theme.variants.dark ? 'dark' : 'light') : previous.scheme;
      return { ...previous, custom, scheme, themes: { dark: pick('dark'), light: pick('light') } };
    });
    setEditor(null);
    return null;
  };
  return <div className={`settings-layout${editor ? ' is-editing-theme' : ''}`}>
    <div className="settings-page" ref={page}>
      <div className="settings-content">
        <header className="settings-page-header"><h1>{details.label}</h1><p>{details.description}</p></header>
        {section === 'folders' && <>
          {folders && !folders.contentReady && <div className="notice" role="status"><Icon name="sparkle" /><span>Welcome to Battle Plan. Choose your game installation and Paradox user folders to load the game’s original assets and mods.</span></div>}
          <SettingsBlock id="folders-game" title="Game">
            <GameFolderSettings folders={folders} client={client} onFoldersChange={onFoldersChange} />
          </SettingsBlock>
          <SettingsBlock id="folders-recordings" title="Recordings">
            <ReplayFolderSetting compact library={library} client={client} />
          </SettingsBlock>
          {folderError && <p className="map-menu-error" role="alert">{folderError}</p>}
        </>}
        {section === 'appearance' && <><AppearanceSettings appearance={appearance} searchTarget={target?.id} onEdit={editTheme} /><ResetPages onReset={onResetPages} /></>}
        {section === 'graphics' && <SettingsBlock id="graphics-defaults" title="Defaults">
          <div className="global-map-settings">
            <MapAppearance key={`${folders?.game ?? ''}:${folders?.user ?? ''}`} global source={null} client={client} userFolder={folders?.user} />
          </div>
        </SettingsBlock>}
        {section === 'preparation' && client.preparationPreferences && <SettingsBlock id="preparation-background" title="Background preparation"><PreparationSetting client={client} /></SettingsBlock>}
        {section === 'recording' && <SettingsBlock id="recording-compression" title="Recording compression"><RecordingCompressionSetting client={client} /></SettingsBlock>}
        {section === 'diagnostics' && <MapDiagnostics native={native} metadata={metadata} branch={branch} />}
      </div>
    </div>
    {editor && <ThemeEditor key={editor.id ?? `new-${newSession}`} draft={editor} onSave={saveTheme} onClose={() => setEditor(null)} onPreview={appearance.setPreview} />}
  </div>;
}

/** Restores the built-in analytics pages and every page's layout; a second click confirms.
 * Without `onReset` the pages and layouts are already the defaults, so there is no button;
 * null while that is still being worked out. */
function ResetPages({ onReset }: { onReset?: (() => void) | null }) {
  const [confirming, setConfirming] = useState(false);
  useEffect(() => {
    if (!confirming) return;
    const timer = setTimeout(() => setConfirming(false), 4000);
    return () => clearTimeout(timer);
  }, [confirming]);
  return <SettingsBlock id="appearance-pages" title="Analytics pages">
    <div className="settings-row"><span className="settings-row-text"><strong>Reset to defaults</strong>
      <small>{onReset ? 'Restores the built-in pages with their names, order and widget layouts, and removes pages you added. Loaded recordings stay open.'
        : onReset === null ? 'Checking your pages and layouts…' : 'Your pages and layouts are the defaults.'}</small></span>
      {onReset && <button type="button" className={`button small${confirming ? ' is-danger' : ''}`} onClick={() => {
        if (!confirming) { setConfirming(true); return; }
        onReset(); setConfirming(false);
      }}><Icon name="refresh" />{confirming ? 'Click again to reset' : 'Reset to defaults'}</button>}</div>
  </SettingsBlock>;
}

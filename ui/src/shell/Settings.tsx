import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import type { Dispatch, KeyboardEvent, SetStateAction } from 'react';
import { Icon } from '../components/Icons';
import ReplayFolderSetting from './ReplayFolderSetting';
import type { DashboardClient } from '../dashboard/client';
import type { ReplayLibraryState } from '../dashboard/replayLibrary';
import MapDiagnostics from './MapDiagnostics';
import MapAppearance, { MAP_OPTION_LABELS, MAP_SOUND_LABELS } from '../components/MapAppearance';
import { VIDEO_OPTION_LABELS } from './VideoSettings';
import GameFolderSettings from './GameFolderSettings';
import { actionFailure } from '../dashboard/queryError';
import { FailureLine } from '../components/FailureLine';
import RecordingCompressionSetting from './RecordingCompressionSetting';
import UpdateSettings from '../update/UpdateSettings';
import type { ContentFolders, DesktopState, ReplayMetadata } from './api';
import AppearanceSettings, { newThemeId } from './appearance/AppearanceSettings';
import { ConfirmRow, SettingsBlock } from './SettingsBlock';
import ThemeEditor, { discardDraft, draftChanged, useConfirmPopover } from './appearance/ThemeEditor';
import type { ThemeDraft, ThemeSaveFailure } from './appearance/ThemeEditor';
import { BUILT_IN_THEMES } from './appearance/themes';
import { uniqueThemeName } from './appearance/themeFile';
import { MAX_CUSTOM_THEMES } from './appearance/settings';
import type { Appearance } from './appearance/useAppearance';
import { StyledIcon } from './appearance/StyledIcon';
import { isSidebarIconId } from './appearance/sidebarIcons';
import type { SidebarIconStyles } from './appearance/sidebarIcons';
import type { BranchChoice } from '../dashboard/Dashboard';
import './settings.css';

import { settingsSections } from './settingsMetadata';
import type { SettingsSection, SettingsTarget } from './settingsMetadata';

const searchItems: SettingsTarget[] = [
  { section: 'folders', title: 'Game installation', keywords: 'game folder install steam path' },
  { section: 'folders', title: 'Paradox user folder', keywords: 'documents mods user path' },
  { section: 'folders', title: 'Recordings folder', keywords: 'recordings library campaigns save path' },
  { section: 'folders', title: 'Recording compression', id: 'recording-compression', keywords: 'recording recordings recorder low high live compression cpu stop save size file storage' },
  { section: 'folders', title: 'Auto compress', id: 'recording-auto-compress', fallback: 'recording-compression', keywords: 'recording recordings recorder compression background smaller file after saving storage' },
  { section: 'appearance', title: 'Colour scheme', id: 'appearance-scheme', keywords: 'color dark light system mode' },
  { section: 'appearance', title: 'Themes', id: 'appearance-themes', keywords: 'theme palette colours colors accent' },
  { section: 'appearance', title: 'Import theme', id: 'appearance-import-theme', keywords: 'vs code vscode json file paste' },
  { section: 'appearance', title: 'Create theme', id: 'appearance-create-theme', keywords: 'custom editor colour color' },
  { section: 'appearance', title: 'Backdrop', id: 'appearance-backdrop', keywords: 'background image wallpaper glow ambient sidebar top bar main view' },
  { section: 'appearance', title: 'Contrast', id: 'appearance-contrast', keywords: 'colours colors borders text readability' },
  { section: 'appearance', title: 'Main view glass', id: 'appearance-glass', keywords: 'glass blur opacity transparency frosted panels widgets' },
  { section: 'appearance', title: 'Sidebar and top bar glass', id: 'appearance-chrome-glass', keywords: 'glass blur opacity transparency frosted sidebar header top bar' },
  { section: 'appearance', title: 'Corner radius', id: 'appearance-radius', keywords: 'rounding rounded corners' },
  { section: 'appearance', title: 'Motion', id: 'appearance-motion', keywords: 'animation reduced transitions' },
  { section: 'appearance', title: 'Playback frame rate', id: 'appearance-playback-rate', keywords: 'dashboard fps refresh uncapped' },
  { section: 'appearance', title: 'Chart motion', id: 'appearance-chart-motion', keywords: 'charts animation easing playback smooth cinematic snappy' },
  { section: 'appearance', title: 'Top-bar flag', id: 'appearance-header-flag', keywords: 'flag header country size native large smooth' },
  { section: 'appearance', title: 'Country colours', id: 'appearance-country-colours', keywords: 'charts map colours colors contrast raw adjusted' },
  { section: 'appearance', title: 'Interface font', id: 'appearance-interface-font', keywords: 'typeface font inter typography size' },
  { section: 'appearance', title: 'Headings font', id: 'appearance-display-font', keywords: 'typeface display headings typography size advanced' },
  { section: 'appearance', title: 'Tables font', id: 'appearance-tables-font', keywords: 'typeface monospace numeric typography size' },
  { section: 'appearance', title: 'Diagnostics font', id: 'appearance-diagnostics-font', keywords: 'typeface monospace report typography size advanced' },
  ...(/mac|iphone|ipad|ipod/i.test(navigator.platform) ? [{ section: 'appearance' as const, title: 'Font smoothing', id: 'appearance-font-smoothing', keywords: 'typography antialias grayscale advanced' }] : []),
  { section: 'appearance', title: 'Word wrapping', id: 'appearance-word-wrap', keywords: 'tables text lines wrap' },
  { section: 'appearance', title: 'Analytics pages', id: 'appearance-pages', keywords: 'reset built-in pages tabs layouts defaults restore' },
  { section: 'graphics', title: 'Game settings', id: 'graphics-defaults', keywords: 'map graphics revert defaults follow game' },
  ...[...MAP_OPTION_LABELS.slice(0, 2), ...VIDEO_OPTION_LABELS, ...MAP_OPTION_LABELS.slice(2)]
    .map(title => ({ section: 'graphics' as const, title, keywords: title === MAP_OPTION_LABELS[0] ? 'msaa graphics' : title === VIDEO_OPTION_LABELS[1] ? 'fps cap limit frames a second battery' : title === VIDEO_OPTION_LABELS[2] ? 'zoom ui size' : title === VIDEO_OPTION_LABELS[3] ? 'counters icons labels size gui_scale' : MAP_SOUND_LABELS.includes(title) ? 'sound audio music' : 'graphics' })),
  { section: 'graphics', title: 'Sound', id: 'graphics-sound', keywords: 'audio volume music' },
  { section: 'graphics', title: 'Visual mods', id: 'graphics-visual-mods', keywords: 'mods artwork' },
  { section: 'updates', title: 'Check for updates', id: 'updates-version', keywords: 'version update upgrade download install restart' },
  { section: 'updates', title: 'Update channel', id: 'updates-channel', keywords: 'stable nightly channel track beta' },
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

const CONTROLS = 'input:not([type="hidden"]):not(:disabled), textarea:not(:disabled), button:not(:disabled):not(.font-reset):not(.map-option-inherit):not(.icon-button.small), summary';

function focusTarget(element: HTMLElement) {
  const control = element.matches(CONTROLS) ? element : element.querySelector<HTMLElement>(CONTROLS);
  if (control) { control.focus({ preventScroll: true }); return; }
  if (!element.hasAttribute('tabindex')) element.tabIndex = -1;
  element.focus({ preventScroll: true });
}

function findTarget(root: HTMLElement, target: SettingsTarget, fallback: boolean): HTMLElement | null {
  if (target.id) return root.querySelector<HTMLElement>(`#${CSS.escape(target.id)}`) ?? (fallback && target.fallback ? root.querySelector<HTMLElement>(`#${CSS.escape(target.fallback)}`) : null);
  const labels = root.querySelectorAll<HTMLElement>('.folder-field-title, .map-option-row > span:first-child, .settings-row-text > strong');
  const title = target.title.toLowerCase();
  const text = (element: HTMLElement) => element.textContent?.trim().toLowerCase() ?? '';
  const label = [...labels].find(element => text(element) === title) ?? [...labels].find(element => text(element).startsWith(title));
  return label?.closest<HTMLElement>('.folder-setting-row, .map-option-row, .settings-row') ?? null;
}

export function SettingsNav({ section, icons, onBack, onSection, onTarget, disabled }: {
  section: SettingsSection; icons: SidebarIconStyles; onBack: () => void; onSection: (section: SettingsSection) => void; onTarget: (target: SettingsTarget) => void; disabled?: boolean;
}) {
  const [query, setQuery] = useState('');
  const [active, setActive] = useState(0);
  const input = useRef<HTMLInputElement>(null);
  const list = useRef<HTMLDivElement>(null);
  const results = useMemo(() => search(query), [query]);
  const searching = query.trim().length > 0;
  const sectionIcon = (id: SettingsSection) => <StyledIcon icon={isSidebarIconId(id) ? icons[id]?.icon : undefined}
    fallback={settingsSections.find(item => item.id === id)!.icon} colour={isSidebarIconId(id) ? icons[id]?.colour : undefined} />;
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
      const next = (active + (event.key === 'ArrowDown' ? 1 : -1) + results.length) % results.length;
      setActive(next);
      list.current?.children[next]?.scrollIntoView({ block: 'nearest' });
    }
    if (event.key === 'Enter') { event.preventDefault(); choose(results[active]); }
  };
  return <>
    <nav className="sidebar-menu settings-nav-head" aria-label="Settings">
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
    {searching ? <div ref={list} className="sidebar-menu settings-search-results" id="settings-search-results" role="listbox" aria-label="Settings search results">
      {results.map((item, index) => <button key={`${item.section}:${item.title}`} id={`settings-result-${index}`} type="button" role="option" tabIndex={-1}
        aria-selected={index === active} className="sidebar-row sidebar-result" data-sidebar-icon={item.section} onMouseMove={() => setActive(index)} onClick={() => choose(item)}>
        {sectionIcon(item.section)}
        <span><strong>{item.title}</strong><small>{settingsSections.find(section => section.id === item.section)!.label}</small></span>
      </button>)}
      {!results.length && <p className="sidebar-search-empty" role="status">No matching settings.</p>}
    </div> : <nav className="sidebar-menu" aria-label="Settings sections">
      {settingsSections.map(item => <button key={item.id} type="button" className="sidebar-row" data-sidebar-icon={item.id} aria-current={section === item.id ? 'page' : undefined}
        onClick={() => onSection(item.id)}>{sectionIcon(item.id)}<span>{item.label}</span></button>)}
    </nav>}
  </>;
}

export interface ThemeEditing { draft: ThemeDraft; original: ThemeDraft; session: number; focus?: boolean; typing?: boolean }

export default function Settings({ section, target, onTargetShown, appearance, library, client, native, metadata, branch, folders, folderError, onFoldersRetry, onFoldersChange, onResetPages, themeEditing, onThemeEditing }: {
  section: SettingsSection; target: SettingsTarget | null; onTargetShown: () => void;
  folders: ContentFolders | null; folderError: string | null; onFoldersRetry?: () => void; onFoldersChange: (folders: ContentFolders) => void; native: DesktopState | null; metadata: ReplayMetadata | null; branch: BranchChoice | null;
  library: ReplayLibraryState; client: DashboardClient; appearance: Appearance; onResetPages?: (() => void) | null;
  themeEditing: ThemeEditing | null; onThemeEditing: Dispatch<SetStateAction<ThemeEditing | null>>;
}) {
  const page = useRef<HTMLDivElement>(null);
  const editing = section === 'appearance' ? themeEditing : null;
  const editorOpener = useRef<HTMLElement | null>(null);
  const replace = useConfirmPopover();
  const startEditing = (draft: ThemeDraft, opener: HTMLElement | null) => {
    editorOpener.current = opener;
    onThemeEditing(previous => ({ draft, original: draft, session: (previous?.session ?? 0) + 1, focus: true }));
  };
  const editTheme = (draft: ThemeDraft) => {
    const opener = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    if (!themeEditing || (!themeEditing.typing && !draftChanged(themeEditing.draft, themeEditing.original))) { startEditing(draft, opener); return; }
    replace.ask(opener, discardDraft(themeEditing.draft, `The theme editor has unsaved changes. ${draft.id ? 'Editing' : 'Starting'} “${draft.name}” discards them.`), () => startEditing(draft, opener));
  };
  const closeEditor = () => {
    onThemeEditing(null);
    const opener = editorOpener.current;
    editorOpener.current = null;
    requestAnimationFrame(() => (opener?.isConnected ? opener : page.current?.querySelector<HTMLElement>('#appearance-create-theme'))?.focus({ preventScroll: true }));
  };
  const setDraft = useCallback((draft: ThemeDraft, typing: boolean) => onThemeEditing(previous => previous && { ...previous, draft, typing }), [onThemeEditing]);
  const editorFocused = useCallback(() => onThemeEditing(previous => previous?.focus ? { ...previous, focus: false } : previous), [onThemeEditing]);
  const editorEscape = useRef<((anchor: HTMLElement | null) => void) | null>(null);
  const details = settingsSections.find(item => item.id === section)!;
  const [rereading, setRereading] = useState(false);
  const foldersUnread = folderError ? actionFailure('Couldn’t read the folder settings.', folderError) : null;
  const rereadFolders = () => {
    if (!client.contentFolders) { onFoldersRetry?.(); return; }
    setRereading(true);
    void client.contentFolders(new AbortController().signal).then(onFoldersChange, () => undefined).finally(() => setRereading(false));
  };

  const reserveNotice = useCallback((notice: HTMLDivElement | null) => {
    const scroller = notice?.closest<HTMLElement>('.settings-page');
    if (!notice || !scroller) return;
    const observer = new ResizeObserver(() => { scroller.style.scrollPaddingTop = `${notice.offsetHeight}px`; });
    observer.observe(notice);
    return () => { observer.disconnect(); scroller.style.scrollPaddingTop = ''; };
  }, []);

  useLayoutEffect(() => { page.current?.scrollTo(0, 0); }, [section]);
  useEffect(() => {
    if (!target || target.section !== section) return;
    let attempts = 0;
    let timer: ReturnType<typeof setTimeout>;
    const reveal = () => {
      const element = page.current && findTarget(page.current, target, attempts >= 8);
      if (!element) { if (++attempts < 20) timer = setTimeout(reveal, 80); else onTargetShown(); return; }
      const folded = element.closest('details');
      if (folded) folded.open = true;
      element.scrollIntoView({ block: 'center', behavior: appearance.resolved.reducedMotion ? 'auto' : 'smooth' });
      element.classList.remove('is-search-target');
      void element.offsetWidth;
      element.classList.add('is-search-target');
      focusTarget(element);
      onTargetShown();
    };
    timer = setTimeout(reveal, 0);
    return () => clearTimeout(timer);
  }, [target, section, onTargetShown, appearance.resolved.reducedMotion]);
  const saveTheme = (draft: ThemeDraft): ThemeSaveFailure | null => {
    const id = draft.id ?? newThemeId();
    if (!appearance.settings.custom.some(item => item.id === id) && appearance.settings.custom.length >= MAX_CUSTOM_THEMES)
      return { text: `You can keep up to ${MAX_CUSTOM_THEMES} custom themes. Delete one first.`, retry: false };
    const others = [...BUILT_IN_THEMES, ...appearance.settings.custom].filter(theme => theme.id !== id).map(theme => theme.name);
    const theme = { id, name: uniqueThemeName(draft.name, others), variants: draft.variants };
    const failed = appearance.commit(previous => {
      const custom = previous.custom.some(item => item.id === id) ? previous.custom.map(item => item.id === id ? theme : item) : [...previous.custom, theme];
      const pick = (mode: 'dark' | 'light') => theme.variants[mode] ? id : previous.themes[mode] === id ? BUILT_IN_THEMES[0].id : previous.themes[mode];
      const scheme = previous.scheme !== 'system' && !theme.variants[previous.scheme] ? (theme.variants.dark ? 'dark' : 'light') : previous.scheme;
      return { ...previous, custom, scheme, themes: { dark: pick('dark'), light: pick('light') } };
    });
    if (failed) return { text: 'Couldn’t save the theme on this device.', retry: true };
    closeEditor();
    return null;
  };
  return <div className={`settings-layout${editing ? ' is-editing-theme' : ''}`}>
    <div className="settings-page" ref={page} onKeyDown={event => {
      if (!editing || event.key !== 'Escape' || event.defaultPrevented) return;
      event.preventDefault(); event.stopPropagation();
      editorEscape.current?.(null);
    }}>
      <div className="settings-content">
        <header className="settings-page-header"><h1>{details.label}</h1><p>{details.description}</p></header>
        {section === 'appearance' && appearance.notice && <div className="settings-page-notice" ref={reserveNotice}>
          <div className="notice warning" role="alert"><Icon name="alert" /><span>{appearance.notice}</span></div></div>}
        {section === 'folders' && <>
          {folders && !folders.contentReady && <div className="notice" role="status"><Icon name="sparkle" /><span>Welcome to BattlePlan. Choose your game installation and Paradox user folders to load the game’s original assets and mods.</span></div>}
          {foldersUnread && <FailureLine className="settings-failure" message={foldersUnread.text} detail={foldersUnread.detail} onRetry={rereadFolders} retryDisabled={rereading} />}
          <SettingsBlock id="folders-game" title="Game">
            <GameFolderSettings folders={folders} unread={!!folderError && !folders} client={client} onFoldersChange={onFoldersChange} />
          </SettingsBlock>
          <SettingsBlock id="folders-recordings" title="Recordings">
            <ReplayFolderSetting library={library} client={client} />
            <RecordingCompressionSetting client={client} />
          </SettingsBlock>
        </>}
        {section === 'appearance' && <><AppearanceSettings appearance={appearance} searchTarget={target?.id} onEdit={editTheme} /><ResetPages onReset={onResetPages} /></>}
        {section === 'graphics' && <MapAppearance key={`${folders?.game ?? ''}:${folders?.user ?? ''}`} global source={null} client={client} userFolder={folders?.user} />}
        {section === 'updates' && <SettingsBlock id="updates" title="Version and channel"><UpdateSettings /></SettingsBlock>}
        {section === 'diagnostics' && <MapDiagnostics native={native} metadata={metadata} branch={branch} />}
      </div>
    </div>
    {editing && <ThemeEditor key={editing.session} draft={editing.draft} original={editing.original} onDraft={setDraft} onSave={saveTheme} onClose={closeEditor} onPreview={appearance.setPreview}
      focusName={!!editing.focus} onFocused={editorFocused} escape={editorEscape} />}
    {replace.element}
  </div>;
}

function ResetPages({ onReset }: { onReset?: (() => void) | null }) {
  const [confirming, setConfirming] = useState(false);
  const trigger = useRef<HTMLButtonElement>(null);
  const cancel = () => { setConfirming(false); requestAnimationFrame(() => trigger.current?.focus()); };
  return <SettingsBlock id="appearance-pages" title="Analytics pages">
    {confirming && onReset ? <ConfirmRow title="Reset all analytics pages?" action="Reset pages" onCancel={cancel} onConfirm={() => { setConfirming(false); onReset(); }}
      detail="Pages you added and every layout change are removed, and the built-in pages return. This can’t be undone." />
      : <div className="settings-row"><span className="settings-row-text"><strong>Built-in pages and layouts</strong>
        <small>{onReset ? 'Restores the built-in pages with their names, order and widget layouts, and removes pages you added. Open campaigns stay open.'
          : onReset === null ? 'Checking your pages and layouts…' : 'Your pages and layouts are the defaults.'}</small></span>
        {onReset && <button ref={trigger} type="button" className="button small" onClick={() => setConfirming(true)}><Icon name="refresh" />Reset pages</button>}</div>}
  </SettingsBlock>;
}

import { actionFailure, failureText, type Failure, type PlainFailure } from './queryError.ts';
import { FailureLine } from '../components/FailureLine';
import { useEffect, useRef, useState } from 'react';
import type { KeyboardEvent, ReactNode } from 'react';
import type { DashboardClient, ReplayFile } from './client';
import type { Campaign, Relocation } from './campaigns';
import { campaignSlots, followRelocations, sameCampaignPath } from './campaigns';
import { distinguishingFolder, distinguishingFolders, qualifiedRecordingName, recordingDisplayName, recordingFilename, recordingNameMatches } from './recordingNames';
import type { SourceBinding } from './sources';
import type { ReplayLibraryState } from './replayLibrary';
import { Icon } from '../components/Icons';
import { SourceFlag } from '../components/CountryFlag';
import { PartialRecordingBadge, partialRecordingText } from '../shell/recordingNotice';
import { useContextMenu } from '../components/ContextMenu';
import type { MenuItem } from '../components/ContextMenu';
import { Popover, PopoverHeading, usePopover } from '../components/Popover';
import CountryPicker from '../components/CountryPicker';
import { saveCampaignFlag, useCampaignFlags } from './campaignFlags';

const LIBRARY_OPEN_KEY = 'battleplan.sidebar.archive-open';
const LIBRARY_FIRST_PAGE = 10;
const LIBRARY_PAGE = 25;

export default function ReplayLibraryMenu({ library, client, campaigns, bindings, defaultId, countryTags, onLoad, onUnload, onDefault, onRenamed, relocations, onOpenReady, slots, errors }: {
  library: ReplayLibraryState; client: DashboardClient; campaigns: Campaign[]; bindings: Record<string, SourceBinding>; defaultId: string | null;
  countryTags: Record<string, string | null>;
  onLoad: (path: string, label: string, slotId: string) => void; onUnload: (id: string) => void; onDefault: (id: string) => void;
  onRenamed: (from: string, to: string) => void;
  relocations: readonly Relocation[];
  slots: ReturnType<typeof campaignSlots>; errors: Record<string, Failure>;
  onOpenReady?: (open: (() => void) | null) => void;
}) {
  const [query, setQuery] = useState('');
  const [pickedFiles, setPickedFiles] = useState<ReplayFile[]>([]);
  const followed = useRef(0);
  useEffect(() => {
    const fresh = relocations.slice(followed.current);
    followed.current = relocations.length;
    if (fresh.length) setPickedFiles(files => files.map(file => {
      const path = followRelocations(file.path, fresh);
      return path === file.path ? file : { ...file, path, name: recordingFilename(path) };
    }));
  }, [relocations]);
  const [picking, setPicking] = useState(false);
  const [pickError, setPickError] = useState<SidebarFailure | null>(null);
  const [actionError, setActionError] = useState<SidebarFailure | null>(null);
  const [renaming, setRenaming] = useState<string | null>(null);
  const editingPath = useRef<string | null>(null);
  const contextMenu = useContextMenu('Campaign actions');
  const flagMenu = usePopover('start');
  const [flagCampaign, setFlagCampaign] = useState<string | null>(null);
  const flags = useCampaignFlags();
  const displayName = recordingDisplayName;
  const flagSource = flagCampaign ? bindings[flagCampaign]?.source : null;
  const opening = (path: string) => campaigns.some(campaign => campaign.loading && sameCampaignPath(campaign.path, path));
  const beginRename = (path: string) => {
    if (!client.renameRecording || opening(path)) return;
    setActionError(null); editingPath.current = path; refocus.current = null; refocused.current = null; setRenaming(path);
  };
  const cancelRename = () => { editingPath.current = null; setRenaming(null); };
  const refocus = useRef<string | null>(null);
  const refocused = useRef<HTMLElement | null>(null);
  const focusAfterRename = (path: string) => (element: HTMLButtonElement | null) => {
    if (!element || !refocus.current || !sameCampaignPath(refocus.current, path)) return;
    refocus.current = null; refocused.current = element;
    element.focus({ preventScroll: true });
  };
  const commitRename = (path: string, draft: string) => {
    if (!editingPath.current || !sameCampaignPath(editingPath.current, path)) return;
    cancelRename();
    const name = draft.trim();
    if (!name || name === displayName(path) || !client.renameRecording || opening(path)) return;
    renameFile(path, name);
  };
  const renameFile = (path: string, name: string) => {
    if (!client.renameRecording) return;
    void client.renameRecording(path, name).then(({ path: renamed, warning }) => {
      const row = refocused.current, active = row?.ownerDocument.activeElement;
      refocused.current = null;
      if (row && (active === row || active === row.ownerDocument.body)) refocus.current = renamed;
      setActionError(warning ? { ...actionFailure('Renamed, but Windows didn’t confirm the name was saved.', warning) } : null);
      setPickedFiles(files => files.map(file => sameCampaignPath(file.path, path) ? { ...file, path: renamed, name: recordingFilename(renamed) } : file));
      onRenamed(path, renamed);
      void library.refresh();
    }, (error: unknown) => setActionError(sidebarFailure('Couldn’t rename the recording.', error, () => { if (!opening(path)) renameFile(path, name); })));
  };
  const renamingPath = (path: string) => renaming !== null && sameCampaignPath(renaming, path);
  const renameInput = (path: string, leading: ReactNode) => <div className="sidebar-row is-renaming">{leading}
    <input className="sidebar-row-input" aria-label="Campaign name" defaultValue={displayName(path)} autoFocus
      onFocus={event => event.currentTarget.select()} onBlur={event => commitRename(path, event.currentTarget.value)}
      onKeyDown={event => {
        if (event.key === 'Enter') { event.preventDefault(); refocus.current = path; event.currentTarget.blur(); }
        else if (event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); refocus.current = path; cancelRename(); }
      }} /></div>;
  const saveFlag = (campaignKey: string, tag: string | null) => {
    const report = (message: string | null, warning?: string | null) => setActionError(message ? sidebarFailure('Couldn’t change the campaign flag.', message, () => saveFlag(campaignKey, tag))
      : warning ? actionFailure('Flag changed, but Windows didn’t confirm it was saved.', warning) : null);
    report(saveCampaignFlag(campaignKey, tag, report));
  };
  const loadError = (path: string) => Object.entries(errors).find(([failed]) => sameCampaignPath(failed, path))?.[1];
  const actions = (path: string, campaign?: Campaign, button?: HTMLButtonElement): MenuItem[] => {
    const source = campaign ? bindings[campaign.id]?.source : null;
    return [
      ...(!campaign && loadError(path) && !opening(path) ? [{ label: 'Try again', icon: 'refresh' as const, run: () => load(path) }] : []),
      ...(client.renameRecording && !campaign?.loading && !opening(path) ? [{ label: 'Rename', icon: 'edit' as const, run: () => beginRename(path) }] : []),
      ...(source && campaign && button ? [
        { label: 'Change flag', icon: 'flag' as const, run: () => { setFlagCampaign(campaign.id); flagMenu.trigger.current = button; flagMenu.openAt(() => button.getBoundingClientRect()); } },
        ...(flags[source.metadata.campaignKey] ? [{ label: 'Use the shown country’s flag', icon: 'refresh' as const, run: () => saveFlag(source.metadata.campaignKey, null) }] : []),
        { label: 'Unload', icon: 'close' as const, run: () => onUnload(campaign.id) },
      ] : []),
    ];
  };
  const rowKey = (event: KeyboardEvent<HTMLButtonElement>, path: string, campaign?: Campaign) => {
    if (event.key === 'F2') { event.preventDefault(); event.stopPropagation(); beginRename(path); }
    else if (event.key === 'ContextMenu' || event.key === 'F10' && event.shiftKey) {
      const items = actions(path, campaign, event.currentTarget);
      if (!items.length) return;
      event.preventDefault(); event.stopPropagation();
      const rect = event.currentTarget.getBoundingClientRect();
      contextMenu.open(rect.left, rect.bottom, items);
    }
  };
  const [libraryOpen, setLibraryOpen] = useState(() => { try { return localStorage.getItem(LIBRARY_OPEN_KEY) !== 'closed'; } catch { return true; } });
  const [libraryToggled, setLibraryToggled] = useState(false);
  const [libraryShown, setLibraryShown] = useState(LIBRARY_FIRST_PAGE);
  const [libraryOpening, setLibraryOpening] = useState<string[]>([]);
  const campaignRows = useRef(new Map<string, HTMLButtonElement>());
  const searchInput = useRef<HTMLInputElement>(null);
  const slotId = slots.empty[0]?.id ?? 'new';
  const pickRecording = async () => {
    if (!client.chooseRecording || picking) return;
    setPicking(true); setPickError(null);
    try {
      const path = await client.chooseRecording();
      if (!path) return;
      const name = recordingFilename(path);
      setPickedFiles(files => [{ path, name, folder: '' }, ...files.filter(file => !sameCampaignPath(file.path, path))]);
      setQuery('');
      onLoad(path, name, slotId);
    } catch (error) { setPickError(sidebarFailure('Couldn’t open a recording.', error, () => { void latestPick.current(); })); }
    finally { setPicking(false); }
  };
  const latestPick = useRef(pickRecording);
  latestPick.current = pickRecording;
  useEffect(() => { onOpenReady?.(() => { void latestPick.current(); }); return () => onOpenReady?.(null); }, [onOpenReady]);
  useEffect(() => {
    const focus = (event: globalThis.KeyboardEvent) => {
      const input = searchInput.current;
      const typing = event.target instanceof HTMLElement && event.target.closest('input, textarea, select, [contenteditable]');
      if (event.key !== '/' || event.defaultPrevented || typing || event.ctrlKey || event.metaKey || event.altKey || !input?.getClientRects().length) return;
      event.preventDefault();
      input.focus();
    };
    window.addEventListener('keydown', focus);
    return () => window.removeEventListener('keydown', focus);
  }, []);
  const load = (path: string) => {
    if (opening(path)) return;
    setLibraryOpening(paths => [...paths.filter(item => !sameCampaignPath(item, path)), path]);
    onLoad(path, recordingFilename(path), slotId);
  };
  const files: ReplayFile[] = [...pickedFiles.filter(picked => !library.files.some(file => sameCampaignPath(file.path, picked.path))), ...library.files];
  for (const campaign of campaigns) if (!files.some(file => sameCampaignPath(file.path, campaign.path)) && bindings[campaign.id]?.source) {
    files.unshift({ path: campaign.path, name: campaign.label, folder: '' });
  }
  const openingFromLibrary = (campaign: Campaign) => !bindings[campaign.id]?.source && libraryOpening.some(path => sameCampaignPath(path, campaign.path))
    && files.some(file => sameCampaignPath(file.path, campaign.path));
  const visible = campaigns.filter(campaign => bindings[campaign.id]?.source || campaign.loading && !openingFromLibrary(campaign));
  const needle = query.trim().toLowerCase();
  const sharedNames = distinguishingFolders(files.map(file => file.path));
  const folderShown = (path: string) => distinguishingFolder(sharedNames, path);
  const fullName = (path: string) => qualifiedRecordingName(sharedNames, path);
  const rowLabel = (path: string, failed = false) => {
    const folder = folderShown(path);
    const name = <span data-overflow-label>{displayName(path)}</span>;
    return folder || failed ? <span className="sidebar-row-text">{name}
      {folder && <small className="sidebar-row-folder"><bdi>{folder}</bdi></small>}{failed && <small>Couldn’t load</small>}</span> : name;
  };
  const folderOf = (path: string) => files.find(file => file.folder && sameCampaignPath(file.path, path))?.folder ?? '';
  const matches = (path: string) => recordingNameMatches(path, needle) || folderOf(path).toLowerCase().includes(needle);
  const shown = visible.filter(campaign => matches(campaign.path));
  const archived = files.filter(file => !visible.some(campaign => sameCampaignPath(file.path, campaign.path)));
  const found = needle ? archived.filter(file => matches(file.path)) : [];
  useEffect(() => {
    const settled = libraryOpening.filter(path => !opening(path));
    if (!settled.length) return;
    setLibraryOpening(paths => paths.filter(path => !settled.includes(path)));
    const loaded = campaigns.find(campaign => bindings[campaign.id]?.source && settled.some(path => sameCampaignPath(path, campaign.path)));
    if (!loaded) return;
    const active = document.activeElement;
    if (!active || active === document.body) campaignRows.current.get(loaded.id)?.focus();
    if (needle && matches(loaded.path)) setQuery('');
  }, [libraryOpening, campaigns, bindings]);
  const sole = needle && shown.length + found.length === 1 ? shown[0]?.path ?? found[0].path : null;
  const isSole = (path: string) => sole !== null && sameCampaignPath(sole, path);
  const searchKey = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.key === 'Escape' && query) { event.preventDefault(); event.stopPropagation(); setQuery(''); return; }
    if (event.key !== 'Enter' || sole === null) return;
    event.preventDefault();
    const campaign = shown.find(item => sameCampaignPath(item.path, sole));
    if (!campaign) { load(sole); return; }
    if (!bindings[campaign.id]?.source) return;
    onDefault(campaign.id);
    setQuery('');
    campaignRows.current.get(campaign.id)?.focus();
  };
  const idle = visible.length === 0;
  const emptyShown = idle && !needle;
  const offerOpen = !archived.length && !!client.chooseRecording;
  const emptyText = archived.length ? 'Pick one from the library.' : library.loading ? 'Reading your recordings folder…'
    : library.error ? null : 'Recordings from BattlePlan Recorder appear here.';
  const shelfOpen = libraryOpen || idle && !libraryToggled;
  const toggleLibrary = () => {
    const next = !shelfOpen;
    setLibraryToggled(true); setLibraryOpen(next);
    try { localStorage.setItem(LIBRARY_OPEN_KEY, next ? 'open' : 'closed'); } catch {   }
  };
  const libraryRow = (file: ReplayFile) => {
    const busy = opening(file.path);
    const error = busy ? undefined : loadError(file.path);
    const name = fullName(file.path);
    const editing = renamingPath(file.path);
    return <div className={`loaded-campaign${error ? ' is-failed' : ''}`} role="listitem" key={file.path}>
      {editing ? renameInput(file.path, <Icon name="file" />) : <button ref={focusAfterRename(file.path)} type="button" className="sidebar-row library-row" aria-busy={busy || undefined}
        aria-label={busy ? `Opening ${name}` : error ? `Load ${name}, couldn’t load last time` : `Load ${name}`} data-match={isSole(file.path) || undefined}
        data-detail={error?.message} data-tooltip={error ? failureText(error, 'Couldn’t read the recording.') : undefined} data-tooltip-overflow={name}
        onKeyDown={event => rowKey(event, file.path)}
        onContextMenu={event => { const items = actions(file.path); if (items.length) { event.preventDefault(); event.stopPropagation(); contextMenu.open(event.clientX, event.clientY, items); } }}
        onClick={() => load(file.path)}>
        {busy ? <span className="campaign-loading"><span className="spinner" /></span> : <Icon name={error ? 'alert' : 'file'} />}
        {rowLabel(file.path, !!error)}
      </button>}
      {error && !editing && <button type="button" className="icon-button campaign-action" aria-label={`Try loading ${name} again`} data-tooltip="Try again"
        onClick={() => load(file.path)}><Icon name="refresh" /></button>}
    </div>;
  };
  return <div className="sidebar-campaigns">
    <div className="sidebar-group-heading"><span className="group-label sidebar-group-title">Campaigns</span>
      <button type="button" className="icon-button sidebar-group-action" aria-label="Open a recording" disabled={picking || !client.chooseRecording} hidden={emptyShown && offerOpen}
        data-tooltip={client.chooseRecording ? 'Open a recording…' : 'Open recordings in the desktop app'}
        onClick={() => { void pickRecording(); }}>{picking ? <span className="spinner" /> : <Icon name="plus" />}</button></div>
    {[pickError, actionError].map((error, index) => error && <FailureLine key={index} className="sidebar-failure" message={error.text} detail={error.detail} onRetry={error.retry} onDismiss={error.retry ? undefined : () => setActionError(null)} retryDisabled={index === 0 && picking} />)}
    <label className="sidebar-search"><Icon name="search" />
      <input ref={searchInput} type="search" aria-label="Find a campaign" placeholder="Find a campaign" value={query} onFocus={() => { void library.refresh(); }}
        aria-keyshortcuts={sole ? 'Enter' : undefined} aria-describedby={sole ? 'sidebar-sole-match' : undefined}
        onChange={event => setQuery(event.currentTarget.value)} onKeyDown={searchKey} />
      {query ? <button type="button" className="icon-button small" aria-label="Clear search" onClick={() => { setQuery(''); searchInput.current?.focus(); }}><Icon name="close" /></button>
        : <kbd aria-hidden="true">/</kbd>}
    </label>
    <p className="sr-only" id="sidebar-sole-match" role="status">{sole ? `Press Enter to ${shown.length ? 'show' : 'open'} ${fullName(sole)}.` : ''}</p>
    {library.error && <FailureLine className="sidebar-failure" message="Couldn’t read the recordings folder." detail={library.error}
      onRetry={() => { void library.refresh(); }} retryDisabled={library.loading} />}
    <div className="sidebar-campaigns-empty" data-collapsed={!emptyShown || undefined} inert={!emptyShown}><div><div>
      {emptyText && <p>{emptyText}</p>}
      {offerOpen && <button type="button" className="button small" disabled={picking} onClick={() => { void pickRecording(); }}>
        <Icon name="folder-open" />Open a recording…</button>}
    </div></div></div>
    {!emptyShown && <div className="sidebar-menu loaded-campaigns scroll-fade" role="list" aria-label={needle ? 'Campaign search results' : 'Loaded campaigns'}>
      {shown.map(campaign => {
        const isDefault = campaign.id === defaultId;
        const source = bindings[campaign.id]?.source;
        const partial = source?.metadata.partial;
        const damaged = source?.metadata.damaged;
        const tag = (source ? flags[source.metadata.campaignKey] : undefined) ?? countryTags[campaign.id] ?? null;
        const country = tag ? source?.metadata.countries.find(item => item.tag === tag)?.name : undefined;
        const flag = source ? <SourceFlag client={client} source={source} tag={tag} /> : <span className="campaign-loading"><span className="spinner" /></span>;
        const editing = renamingPath(campaign.path);
        return <div className="loaded-campaign" role="listitem" key={campaign.id}>
          {editing ? renameInput(campaign.path, flag) : <button type="button" className="sidebar-row campaign-row" disabled={!source} aria-current={isDefault ? 'true' : undefined}
            data-shown={isDefault && visible.length > 1 || undefined} data-match={isSole(campaign.path) || undefined} data-tooltip-overflow={fullName(campaign.path)}
            ref={element => {
              if (element) campaignRows.current.set(campaign.id, element); else campaignRows.current.delete(campaign.id);
              focusAfterRename(campaign.path)(element);
            }}
            onKeyDown={event => rowKey(event, campaign.path, campaign)}
            onContextMenu={event => { event.preventDefault(); event.stopPropagation(); contextMenu.open(event.clientX, event.clientY, actions(campaign.path, campaign, event.currentTarget)); }}
            aria-label={`${isDefault ? `${fullName(campaign.path)}, shown campaign` : `Show ${fullName(campaign.path)}`}${partial ? ', partial recording' : ''}`}
            data-tooltip={[country, partial && partialRecordingText(partial)].filter(Boolean).join(' · ') || undefined} onClick={() => onDefault(campaign.id)}>
            {flag}
            {rowLabel(campaign.path)}
            <PartialRecordingBadge partial={partial} damaged={damaged} />
          </button>}
          {source && !editing && <button type="button" className="icon-button campaign-action campaign-unload" aria-label={`Unload ${fullName(campaign.path)}`} data-tooltip="Unload"
            onClick={() => onUnload(campaign.id)}><Icon name="close" /></button>}
        </div>;
      })}
      {found.map(libraryRow)}
      {needle && !shown.length && !found.length && !library.error && <p className="sidebar-search-empty" role="status">{library.loading ? 'Reading your recordings folder…' : 'No matching campaigns.'}</p>}
    </div>}
    {archived.length > 0 && !needle && <div className="sidebar-library">
      <div className="sidebar-group-heading">
        <button type="button" className="sidebar-group-toggle" aria-expanded={shelfOpen} aria-label={`Library, ${archived.length} ${archived.length === 1 ? 'recording' : 'recordings'}`} onClick={toggleLibrary}>
          <span className="group-label">Library</span><span className="sidebar-group-count" aria-hidden="true">{archived.length}</span>
          <Icon name="chevron-down" className="sidebar-group-chevron" /></button>
      </div>
      {shelfOpen && <div className="sidebar-menu sidebar-library-list scroll-fade" role="list" aria-label="Library">
        {archived.slice(0, libraryShown).map(libraryRow)}
        {archived.length > libraryShown && <button type="button" className="sidebar-shelf-more" onClick={() => setLibraryShown(count => count + LIBRARY_PAGE)}>
          Show more</button>}
      </div>}
    </div>}
    {contextMenu.menu}
    <Popover menu={flagMenu} aria-label="Change campaign flag" className="campaign-flag-picker">
      <PopoverHeading title="Campaign flag" menu={flagMenu} />
      {flagMenu.open && flagSource && <CountryPicker key={flagSource.id} inline client={client} source={flagSource} countries={flagSource.metadata.countries}
        allowedTags={flagSource.metadata.countries.map(country => country.tag)} playerCountryId={flagSource.metadata.playerCountryId}
        selectedTag={flags[flagSource.metadata.campaignKey] ?? (flagCampaign ? countryTags[flagCampaign] : null) ?? null}
        onChange={tag => {
          if (tag && flagSource.metadata.countries.some(country => country.tag === tag)) saveFlag(flagSource.metadata.campaignKey, tag);
          flagMenu.close();
        }} />}
    </Popover>
  </div>;
}

type SidebarFailure = PlainFailure & { retry?: () => void };

function sidebarFailure(sentence: string, error: unknown, retry: () => void): SidebarFailure {
  return { ...actionFailure(sentence, error instanceof Error ? error.message : String(error)), retry };
}

import { useEffect, useRef, useState } from 'react';
import type { KeyboardEvent } from 'react';
import type { DashboardClient, ReplayFile } from './client';
import type { Campaign } from './campaigns';
import { campaignSlots, sameCampaignPath } from './campaigns';
import { recordingDisplayName, recordingFilename, recordingNameMatches, recordingNameTooltip, saveRecordingNames, setRecordingName, useRecordingNames } from './recordingNames';
import type { SourceBinding } from './sources';
import type { ReplayLibraryState } from './replayLibrary';
import { Icon } from '../components/Icons';
import { SourceFlag } from '../components/CountryFlag';
import { PartialRecordingBadge, partialRecordingText } from '../shell/recordingNotice';
import { useContextMenu } from '../components/ContextMenu';
import type { MenuItem } from '../components/ContextMenu';
import { Popover, PopoverHeading, usePopover } from '../components/Popover';
import CountryPicker from '../components/CountryPicker';
import { saveCampaignSettings, setCampaignFlag, useCampaignSettings } from './campaignSettings';

const ARCHIVE_OPEN_KEY = 'hoi4-replay.sidebar.archive-open';
// The archive shows recent rows first; the rest stay behind Show more, a page at a time.
const ARCHIVE_FIRST_PAGE = 10;
const ARCHIVE_PAGE = 25;

export default function ReplayLibraryMenu({ library, client, campaigns, bindings, defaultId, countryTags, onLoad, onUnload, onDefault, onOpenReady, slots, errors }: {
  library: ReplayLibraryState; client: DashboardClient; campaigns: Campaign[]; bindings: Record<string, SourceBinding>; defaultId: string | null;
  /** The country each campaign shows: the chosen one, else the recorded player. */
  countryTags: Record<string, string | null>;
  onLoad: (path: string, label: string, slotId: string) => void; onUnload: (id: string) => void; onDefault: (id: string) => void;
  slots: ReturnType<typeof campaignSlots>; errors: Record<string, string>;
  onOpenReady?: (open: (() => void) | null) => void;
}) {
  const [query, setQuery] = useState('');
  const [pickedFiles, setPickedFiles] = useState<ReplayFile[]>([]);
  const [picking, setPicking] = useState(false);
  const [pickError, setPickError] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [actionNotice, setActionNotice] = useState<string | null>(null);
  const [renaming, setRenaming] = useState<string | null>(null);
  const editingPath = useRef<string | null>(null);
  const contextMenu = useContextMenu('Campaign actions');
  const flagMenu = usePopover('start');
  const [flagCampaign, setFlagCampaign] = useState<string | null>(null);
  const settings = useCampaignSettings();
  const names = useRecordingNames();
  const displayName = (path: string) => recordingDisplayName(path, names);
  const flagSource = flagCampaign ? bindings[flagCampaign]?.source : null;
  const beginRename = (path: string) => {
    setActionError(null); editingPath.current = path; setRenaming(path);
  };
  const cancelRename = () => { editingPath.current = null; setRenaming(null); };
  const commitRename = (path: string, draft: string) => {
    if (!editingPath.current || !sameCampaignPath(editingPath.current, path)) return;
    cancelRename();
    setActionError(saveRecordingNames(setRecordingName(names, path, draft)));
  };
  const renameInput = (path: string) => <input className="campaign-rename" aria-label="Recording name" defaultValue={displayName(path)} autoFocus
    onFocus={event => event.currentTarget.select()} onBlur={event => commitRename(path, event.currentTarget.value)}
    onKeyDown={event => {
      if (event.key === 'Enter') { event.preventDefault(); event.currentTarget.blur(); }
      else if (event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); cancelRename(); }
    }} />;
  const saveFlag = (campaignKey: string, tag: string | null) => setActionError(saveCampaignSettings(setCampaignFlag(settings, campaignKey, tag)));
  const actions = (path: string, campaign?: Campaign, button?: HTMLButtonElement): MenuItem[] => {
    const source = campaign ? bindings[campaign.id]?.source : null;
    return [
      { label: 'Rename', icon: 'edit', run: () => beginRename(path) },
      ...(client.compactRecording ? [{ label: 'Compact', run: () => {
        setActionError(null); setActionNotice(null);
        void client.compactRecording!(path).then(progress => {
          setActionNotice(`Compaction started. See Settings for progress and results.${progress.gameRunning ? ' The game is running; compacting may slow it briefly.' : ''}`);
        }, error => setActionError(error instanceof Error ? error.message : String(error)));
      } }] : []),
      ...(source && campaign && button ? [
        { label: 'Change flag', run: () => { setFlagCampaign(campaign.id); flagMenu.trigger.current = button; flagMenu.openAt(() => button.getBoundingClientRect()); } },
        ...(settings.campaigns[source.metadata.campaignKey]?.flag ? [{ label: "Use the shown country's flag", run: () => saveFlag(source.metadata.campaignKey, null) }] : []),
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
  const [archiveOpen, setArchiveOpen] = useState(() => { try { return localStorage.getItem(ARCHIVE_OPEN_KEY) === 'open'; } catch { return false; } });
  const [archiveShown, setArchiveShown] = useState(ARCHIVE_FIRST_PAGE);
  const toggleArchive = () => setArchiveOpen(open => {
    try { localStorage.setItem(ARCHIVE_OPEN_KEY, open ? 'closed' : 'open'); } catch { /* the choice lasts this session */ }
    return !open;
  });
  // A new campaign fills the layout's first free slot; the sidebar then chooses which one is shown.
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
    } catch (error) { setPickError(error instanceof Error ? error.message : String(error)); }
    finally { setPicking(false); }
  };
  const latestPick = useRef(pickRecording);
  latestPick.current = pickRecording;
  useEffect(() => { onOpenReady?.(() => { void latestPick.current(); }); return () => onOpenReady?.(null); }, [onOpenReady]);
  const files: ReplayFile[] = [...pickedFiles.filter(picked => !library.files.some(file => sameCampaignPath(file.path, picked.path))), ...library.files];
  for (const campaign of campaigns) if (!files.some(file => sameCampaignPath(file.path, campaign.path)) && bindings[campaign.id]?.source) {
    files.unshift({ path: campaign.path, name: campaign.label, folder: '' });
  }
  const visible = campaigns.filter(campaign => bindings[campaign.id]?.source || campaign.loading);
  const needle = query.trim().toLowerCase();
  const shown = visible.filter(campaign => recordingNameMatches(campaign.path, needle, names));
  const archived = files.filter(file => !visible.some(campaign => sameCampaignPath(file.path, campaign.path)));
  const found = needle ? archived.filter(file => recordingNameMatches(file.path, needle, names) || file.folder.toLowerCase().includes(needle)) : [];
  const loadRow = (file: ReplayFile) => {
    const error = Object.entries(errors).find(([path]) => sameCampaignPath(path, file.path))?.[1];
    return <div className="loaded-campaign" role="listitem" key={file.path}>
      {renaming && sameCampaignPath(renaming, file.path) ? renameInput(file.path) : <button type="button" className="sidebar-row" data-tooltip={[recordingNameTooltip(file.path, names), error].filter(Boolean).join(' · ')}
        aria-label={`Load ${displayName(file.path)}`} onKeyDown={event => rowKey(event, file.path)}
        onContextMenu={event => { const items = actions(file.path); if (items.length) { event.preventDefault(); event.stopPropagation(); contextMenu.open(event.clientX, event.clientY, items); } }}
        onClick={() => { onLoad(file.path, recordingFilename(file.path), slotId); setQuery(''); }}><Icon name="file" /><span>{displayName(file.path)}</span></button>}
    </div>;
  };
  return <div className="sidebar-campaigns">
    <div className="sidebar-group-heading"><span>Campaigns</span>
      <button type="button" className="icon-button small" aria-label="Open a recording" disabled={picking || !client.chooseRecording}
        data-tooltip={client.chooseRecording ? 'Open a recording' : 'Open recordings in the desktop app'}
        onClick={() => { void pickRecording(); }}>{picking ? <span className="spinner" /> : <Icon name="plus" />}</button></div>
    {pickError && <p className="dash-inline-warning" role="alert">{pickError}</p>}
    {actionNotice && <p className="settings-help" role="status">{actionNotice}</p>}
    {actionError && <p className="dash-inline-warning" role="alert">{actionError}</p>}
    <label className="sidebar-search"><Icon name="search" />
      <input type="search" aria-label="Search recordings" placeholder="Find a campaign…" value={query} onFocus={() => { void library.refresh(); }}
        onChange={event => setQuery(event.currentTarget.value)} onKeyDown={event => { if (event.key === 'Escape') setQuery(''); }} /></label>
    <div className="sidebar-menu loaded-campaigns" role="list" aria-label="Loaded campaigns">
      {shown.map(campaign => {
        const isDefault = campaign.id === defaultId;
        const source = bindings[campaign.id]?.source;
        const partial = source?.metadata.partial;
        const damaged = source?.metadata.damaged;
        const tag = (source ? settings.campaigns[source.metadata.campaignKey]?.flag : undefined) ?? countryTags[campaign.id] ?? null;
        const country = tag ? source?.metadata.countries.find(item => item.tag === tag)?.name : undefined;
        return <div className="loaded-campaign" role="listitem" key={campaign.id}>
          {renaming && sameCampaignPath(renaming, campaign.path) ? renameInput(campaign.path) : <button type="button" className="sidebar-row campaign-row" disabled={!source} aria-current={isDefault ? 'true' : undefined}
            onKeyDown={event => rowKey(event, campaign.path, campaign)}
            onContextMenu={event => { event.preventDefault(); event.stopPropagation(); contextMenu.open(event.clientX, event.clientY, actions(campaign.path, campaign, event.currentTarget)); }}
            aria-label={`${isDefault ? `${displayName(campaign.path)}, shown campaign` : `Show ${displayName(campaign.path)}`}${partial ? ', partial recording' : ''}`}
            data-tooltip={[recordingNameTooltip(campaign.path, names), country, partial && partialRecordingText(partial)].filter(Boolean).join(' · ')} onClick={() => onDefault(campaign.id)}>
            {source ? <SourceFlag client={client} source={source} tag={tag} /> : <span className="campaign-loading"><span className="spinner" /></span>}
            <span>{displayName(campaign.path)}</span>
            <PartialRecordingBadge partial={partial} damaged={damaged} />
          </button>}
          {source && !(renaming && sameCampaignPath(renaming, campaign.path)) && <button type="button" className="icon-button small campaign-unload" aria-label={`Unload ${displayName(campaign.path)}`} data-tooltip="Unload"
            onClick={() => onUnload(campaign.id)}><Icon name="close" /></button>}
        </div>;
      })}
      {found.map(loadRow)}
      {needle && !shown.length && !found.length && <p className="sidebar-search-empty">{library.loading ? 'Reading folder…' : 'No matching recordings.'}</p>}
    </div>
    {/* Recordings that are not open stay out of the way until asked for. */}
    {archived.length > 0 && <div className="sidebar-archive">
      <button type="button" className="sidebar-shelf-toggle" aria-expanded={archiveOpen} onClick={toggleArchive}>
        <span>{archiveOpen ? 'Archive' : `Archive (${archived.length})`}</span><span className="sidebar-shelf-line" aria-hidden="true" /><Icon name="chevron-down" /></button>
      {/* Search results already contain archive rows; each recording has one editor. */}
      {archiveOpen && !needle && <div className="sidebar-menu sidebar-archive-list" role="list" aria-label="Archived recordings">
        {archived.slice(0, archiveShown).map(loadRow)}
        {archived.length > archiveShown && <button type="button" className="sidebar-shelf-more" onClick={() => setArchiveShown(count => count + ARCHIVE_PAGE)}>
          Show more</button>}
      </div>}
    </div>}
    {contextMenu.menu}
    <Popover menu={flagMenu} aria-label="Change campaign flag" className="campaign-flag-picker">
      <PopoverHeading title="Campaign flag" menu={flagMenu} />
      {flagMenu.open && flagSource && <CountryPicker key={flagSource.id} inline client={client} source={flagSource} countries={flagSource.metadata.countries}
        allowedTags={flagSource.metadata.countries.map(country => country.tag)} playerCountryId={flagSource.metadata.playerCountryId}
        selectedTag={settings.campaigns[flagSource.metadata.campaignKey]?.flag ?? (flagCampaign ? countryTags[flagCampaign] : null) ?? null}
        onChange={tag => {
          if (tag && flagSource.metadata.countries.some(country => country.tag === tag)) saveFlag(flagSource.metadata.campaignKey, tag);
          flagMenu.close();
        }} />}
    </Popover>
  </div>;
}

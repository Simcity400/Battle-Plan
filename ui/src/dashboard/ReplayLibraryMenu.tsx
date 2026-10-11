import { useEffect, useRef, useState } from 'react';
import type { DashboardClient, ReplayFile } from './client';
import type { Campaign } from './campaigns';
import { campaignSlots, sameCampaignPath } from './campaigns';
import type { SourceBinding } from './sources';
import type { ReplayLibraryState } from './replayLibrary';
import { Icon } from '../components/Icons';
import { SourceFlag } from '../components/CountryFlag';
import FactionColoursButton from './FactionColours';
import { PartialRecordingBadge, partialRecordingText } from '../shell/recordingNotice';

const replayFilename = (path: string) => path.replaceAll('\\', '/').split('/').pop()!;
const displayName = (path: string) => replayFilename(path).replace(/\.(?:battleplan|hoi4replay)$/i, '');
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
      const name = replayFilename(path);
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
  const shown = visible.filter(campaign => displayName(campaign.path).toLowerCase().includes(needle));
  const archived = files.filter(file => !visible.some(campaign => sameCampaignPath(file.path, campaign.path)));
  const found = needle ? archived.filter(file => `${file.name} ${file.folder}`.toLowerCase().includes(needle)) : [];
  const loadRow = (file: ReplayFile) => {
    const error = Object.entries(errors).find(([path]) => sameCampaignPath(path, file.path))?.[1];
    return <div className="loaded-campaign" role="listitem" key={file.path}>
      <button type="button" className="sidebar-row" data-tooltip={error ?? `Load ${replayFilename(file.path)}`}
        onClick={() => { onLoad(file.path, replayFilename(file.path), slotId); setQuery(''); }}><Icon name="file" /><span>{file.name}</span></button>
    </div>;
  };
  return <div className="sidebar-campaigns">
    <div className="sidebar-group-heading"><span>Campaigns</span>
      <button type="button" className="icon-button small" aria-label="Open a recording" disabled={picking || !client.chooseRecording}
        data-tooltip={client.chooseRecording ? 'Open a recording' : 'Open recordings in the desktop app'}
        onClick={() => { void pickRecording(); }}>{picking ? <span className="spinner" /> : <Icon name="plus" />}</button></div>
    {pickError && <p className="dash-inline-warning" role="alert">{pickError}</p>}
    <label className="sidebar-search"><Icon name="search" />
      <input type="search" aria-label="Search recordings" placeholder="Find a campaign…" value={query} onFocus={() => { void library.refresh(); }}
        onChange={event => setQuery(event.currentTarget.value)} onKeyDown={event => { if (event.key === 'Escape') setQuery(''); }} /></label>
    <div className="sidebar-menu loaded-campaigns" role="list" aria-label="Loaded campaigns">
      {shown.map(campaign => {
        const isDefault = campaign.id === defaultId;
        const source = bindings[campaign.id]?.source;
        const partial = source?.metadata.partial;
        const tag = countryTags[campaign.id] ?? null;
        const country = tag ? source?.metadata.countries.find(item => item.tag === tag)?.name : undefined;
        return <div className="loaded-campaign" role="listitem" key={campaign.id}>
          <button type="button" className="sidebar-row campaign-row" disabled={!source} aria-current={isDefault ? 'true' : undefined}
            aria-label={`${isDefault ? `${displayName(campaign.path)}, shown campaign` : `Show ${displayName(campaign.path)}`}${partial ? ', partial recording' : ''}`}
            data-tooltip={[displayName(campaign.path), country, partial && partialRecordingText(partial)].filter(Boolean).join(' · ')} onClick={() => onDefault(campaign.id)}>
            {source ? <SourceFlag client={client} source={source} tag={tag} /> : <span className="campaign-loading"><span className="spinner" /></span>}
            <span>{displayName(campaign.path)}</span>
            <PartialRecordingBadge partial={partial} />
          </button>
          {source && <FactionColoursButton client={client} source={source} label={displayName(campaign.path)} />}
          {source && <button type="button" className="icon-button small campaign-unload" aria-label={`Unload ${displayName(campaign.path)}`} data-tooltip="Unload"
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
      {archiveOpen && <div className="sidebar-menu sidebar-archive-list" role="list" aria-label="Archived recordings">
        {archived.slice(0, archiveShown).map(loadRow)}
        {archived.length > archiveShown && <button type="button" className="sidebar-shelf-more" onClick={() => setArchiveShown(count => count + ARCHIVE_PAGE)}>
          Show more</button>}
      </div>}
    </div>}
  </div>;
}

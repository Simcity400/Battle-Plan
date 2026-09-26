import { Popover, usePopover } from '../components/Popover';
import { useEffect, useId, useState } from 'react';
import type { DashboardClient, ReplayFile } from './client';
import type { Campaign } from './campaigns';
import { campaignSlots, sameCampaignPath } from './campaigns';
import type { SourceBinding } from './sources';
import type { ReplayLibraryState } from './replayLibrary';
import ReplayFolderSetting from '../shell/ReplayFolderSetting';
import { Icon } from '../components/Icons';
import ReplayFilename from '../components/ReplayFilename';
import { SourceFlag } from '../components/CountryFlag';
import FactionColoursButton from './FactionColours';

const replayFilename = (path: string) => path.replaceAll('\\', '/').split('/').pop()!;
const displayName = (path: string) => replayFilename(path).replace(/\.(?:battleplan|hoi4replay)$/i, '');

export default function ReplayLibraryMenu({ library, client, campaigns, bindings, defaultId, countryTags, onLoad, onUnload, onDefault, onOpenReady, slots, errors }: {
  library: ReplayLibraryState; client: DashboardClient; campaigns: Campaign[]; bindings: Record<string, SourceBinding>; defaultId: string | null;
  /** The country each campaign shows: the chosen one, else the recorded player. */
  countryTags: Record<string, string | null>;
  onLoad: (path: string, label: string, slotId: string) => void; onUnload: (id: string) => void; onDefault: (id: string) => void;
  slots: ReturnType<typeof campaignSlots>; errors: Record<string, string>;
  onOpenReady?: (open: (() => void) | null) => void;
}) {
  const id = useId();
  const menu = usePopover('start');
  const { trigger, popup, open } = menu;
  const [query, setQuery] = useState('');
  const [pickedFiles, setPickedFiles] = useState<ReplayFile[]>([]);
  const [picking, setPicking] = useState(false);
  const [pickError, setPickError] = useState<string | null>(null);
  const [selectedSlot, setSelectedSlot] = useState('');
  const slotId = slots.empty.some(role => role.id === selectedSlot) || (selectedSlot === 'new' && slots.canAdd)
    ? selectedSlot : slots.empty[0]?.id ?? 'new';
  const canLoad = slots.empty.length > 0 || slots.canAdd;
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
  useEffect(() => { onOpenReady?.(() => popup.current?.showPopover()); return () => onOpenReady?.(null); }, [onOpenReady]);
  const files: ReplayFile[] = [...pickedFiles.filter(picked => !library.files.some(file => sameCampaignPath(file.path, picked.path))), ...library.files];
  for (const campaign of campaigns) if (!files.some(file => sameCampaignPath(file.path, campaign.path)) && bindings[campaign.id]?.source) {
    files.unshift({ path: campaign.path, name: campaign.label, folder: '' });
  }
  const rows = files.filter(file => `${file.name} ${file.folder}`.toLowerCase().includes(query.trim().toLowerCase()));
  const visible = campaigns.filter(campaign => bindings[campaign.id]?.source || campaign.loading);
  return <div className="sidebar-campaigns">
    <div className="sidebar-group-heading"><span>Campaigns</span>
      <button ref={trigger} type="button" className="icon-button small" popoverTarget={id} aria-haspopup="dialog" aria-expanded={open}
        aria-label="Open a recording" data-tooltip="Open a recording"><Icon name="plus" /></button></div>
    <div className="sidebar-menu loaded-campaigns" role="list" aria-label="Loaded campaigns">
      {visible.map(campaign => {
        const isDefault = campaign.id === defaultId;
        const source = bindings[campaign.id]?.source;
        const tag = countryTags[campaign.id] ?? null;
        const country = tag ? source?.metadata.countries.find(item => item.tag === tag)?.name : undefined;
        return <div className="loaded-campaign" role="listitem" key={campaign.id}>
          <button type="button" className="sidebar-row campaign-row" disabled={!source} aria-current={isDefault ? 'true' : undefined}
            aria-label={isDefault ? `${displayName(campaign.path)}, shown campaign` : `Show ${displayName(campaign.path)}`}
            data-tooltip={[displayName(campaign.path), country].filter(Boolean).join(' · ')} onClick={() => onDefault(campaign.id)}>
            {source ? <SourceFlag client={client} source={source} tag={tag} /> : <span className="campaign-loading"><span className="spinner" /></span>}
            <span>{displayName(campaign.path)}</span>
          </button>
          {source && <FactionColoursButton client={client} source={source} label={displayName(campaign.path)} />}
          {source && <button type="button" className="icon-button small campaign-unload" aria-label={`Unload ${displayName(campaign.path)}`} data-tooltip="Unload"
            onClick={() => onUnload(campaign.id)}><Icon name="close" /></button>}
        </div>;
      })}
      {!visible.length && <button type="button" className="sidebar-row sidebar-row-hint" onClick={() => popup.current?.showPopover()}>
        <Icon name="folder-open" /><span>Open a recording…</span></button>}
    </div>
    <Popover menu={menu} id={id} className="replay-library-menu" aria-label="Recordings"
      onToggle={event => { const next = event.newState === 'open'; if (next) void library.refresh(); else setQuery(''); }}>
      {open && <>
        <div className="ui-popover-heading"><strong>Recordings</strong><button type="button" className="icon-button ui-popover-action" aria-label="Close recordings" onClick={menu.close}><Icon name="close" /></button></div>
        <div className="replay-library-tools">
          <label className="search-field"><Icon name="search" /><input data-popover-autofocus className="replay-library-search" aria-label="Search recordings" placeholder="Find a campaign…" value={query} onChange={event => setQuery(event.currentTarget.value)} /></label>
          <button type="button" className="button" aria-label="Browse for a recording" data-tooltip="Open a recording from anywhere on this computer" disabled={picking || !canLoad || !client.chooseRecording} onClick={() => { void pickRecording(); }}>
            {picking ? <span className="spinner" /> : <Icon name="folder-open" />}Browse
          </button>
        </div>
        {pickError && <p className="dash-inline-warning" role="alert">{pickError}</p>}
        {slots.empty.length > 0 && <label className="replay-slot"><span>Load into</span><select aria-label="Campaign slot" value={slotId} onChange={event => setSelectedSlot(event.currentTarget.value)}>
          {slots.empty.map(role => <option key={role.id} value={role.id}>{role.label}</option>)}
          {slots.canAdd && <option value="new">New campaign slot</option>}
        </select></label>}
        {!canLoad && <p className="replay-library-empty">Unload a campaign to make room.</p>}
        {(library.folder || files.length > 0) && <>
          <div className="replay-library-list">{rows.map(file => {
            const campaign = campaigns.find(campaign => sameCampaignPath(file.path, campaign.path));
            const binding = campaign ? bindings[campaign.id] : null;
            const label = replayFilename(file.path);
            const error = binding?.error ?? Object.entries(errors).find(([path]) => sameCampaignPath(path, file.path))?.[1];
            const busy = campaign?.loading || binding?.loading;
            return <div className={`replay-library-row${binding?.source ? ' is-loaded' : ''}`} key={file.path}>
              <span className="replay-library-icon" aria-hidden="true"><Icon name="file" /></span>
              <div className="replay-library-name"><strong><ReplayFilename path={file.path} /></strong>
                {error ? <small className="replay-library-error" role="alert">{error}</small> : file.folder ? <small>{file.folder}</small> : null}</div>
              {binding?.source && <span className="pill positive">{campaign?.id === defaultId ? 'Default' : 'Loaded'}</span>}
              <button type="button" className={`button small${binding?.source ? ' quiet' : ''}`} disabled={busy || (!binding?.source && !campaign && !canLoad)}
                onClick={() => binding?.source && campaign ? onUnload(campaign.id) : onLoad(file.path, label, slotId)}>
                {busy ? <><span className="spinner" />Loading</> : binding?.source ? 'Unload' : 'Load'}</button>
            </div>;
          })}</div>
          {!rows.length && <p className="replay-library-empty">{library.loading ? 'Reading folder…' : query ? 'No matching recordings.' : 'No replays in this folder yet.'}</p>}
          {library.error && <p className="dash-inline-warning" role="alert">{library.error}</p>}
          {library.folder && <div className="replay-library-folder"><Icon name="folder" /><span data-tooltip={library.folder}>{library.folder}</span></div>}
        </>}
        {!library.folder && <div className="replay-library-setup"><p className="replay-library-empty">Choose the folder where you keep your replays.</p><ReplayFolderSetting library={library} client={client} /></div>}
      </>}
    </Popover>
  </div>;
}

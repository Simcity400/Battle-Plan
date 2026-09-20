import { Popover, usePopover } from '../ui/Popover';
import { useEffect, useId, useState } from 'react';
import type { DashboardClient, ReplayFile } from './client';
import type { Campaign } from './campaigns';
import { campaignSlots, sameCampaignPath } from './campaigns';
import type { SourceBinding } from './sources';
import type { ReplayLibraryState } from './replayLibrary';
import ReplayFolderSetting from '../ReplayFolderSetting';
import { Icon } from '../Icons';
import ReplayFilename from './ReplayFilename';

const replayFilename = (path: string) => path.replaceAll('\\', '/').split('/').pop()!;

export default function ReplayLibraryMenu({ library, client, campaigns, bindings, defaultId, onLoad, onUnload, onDefault, onOpenReady, slots, errors }: {
  library: ReplayLibraryState; client: DashboardClient; campaigns: Campaign[]; bindings: Record<string, SourceBinding>; defaultId: string | null;
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
  const defaultButton = (campaign: Campaign) => <button type="button" className="campaign-default" aria-pressed={campaign.id === defaultId}
    aria-label={campaign.id === defaultId ? `${replayFilename(campaign.path)} is the default campaign` : `Set ${replayFilename(campaign.path)} as default`}
    data-tooltip={campaign.id === defaultId ? 'Default campaign' : 'Set as default'} onClick={() => onDefault(campaign.id)}>{campaign.id === defaultId ? '★' : '☆'}</button>;
  return <div className="sidebar-campaigns">
    <button ref={trigger} type="button" className="rail-button" popoverTarget={id} aria-haspopup="dialog" aria-expanded={open}><Icon name="folder" /><span>Recordings</span><Icon name="chevron-down" /></button>
    <div className="loaded-campaigns">{campaigns.filter(campaign => bindings[campaign.id]?.source).map(campaign => <div className="loaded-campaign" key={campaign.id}>
      <span className="campaign-loaded-dot" aria-label="Loaded" /><ReplayFilename className="campaign-name" path={campaign.path} />{defaultButton(campaign)}
    </div>)}</div>
    <Popover menu={menu} id={id} className="replay-library-menu" aria-label="Recordings"
      onToggle={event => { const next = event.newState === 'open'; if (next) void library.refresh(); else setQuery(''); }}>
      {open && <>
        <div className="replay-library-tools">
          <input data-popover-autofocus className="replay-library-search" aria-label="Search recordings" placeholder="Find a campaign…" value={query} onChange={event => setQuery(event.currentTarget.value)} />
          <button type="button" className="replay-browse" aria-label="Browse for a recording" data-tooltip="Browse for a recording" disabled={picking || !canLoad || !client.chooseRecording} onClick={() => { void pickRecording(); }}>
            {picking ? <span className="spinner" /> : <Icon name="folder" />}
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
            return <div className="replay-library-row" key={file.path}>
              <span className={`campaign-loaded-dot${binding?.source ? '' : ' unloaded'}`} aria-label={binding?.source ? 'Loaded' : 'Not loaded'} data-tooltip={binding?.source ? 'Loaded' : 'Not loaded'} />
              <div className="replay-library-name"><strong><ReplayFilename path={file.path} /></strong>{error && <small className="replay-library-error" role="alert">{error}</small>}</div>
              {campaign && binding?.source && defaultButton(campaign)}
              <button type="button" className="replay-load" disabled={campaign?.loading || binding?.loading || (!binding?.source && !campaign && !canLoad)} onClick={() => binding?.source && campaign ? onUnload(campaign.id) : onLoad(file.path, label, slotId)}>{campaign?.loading || binding?.loading ? 'Loading…' : binding?.source ? 'Unload' : 'Load'}</button>
            </div>;
          })}</div>
          {!rows.length && <p className="replay-library-empty">{library.loading ? 'Reading folder…' : query ? 'No matching recordings.' : 'No replays in this folder yet.'}</p>}
          {library.error && <p className="dash-inline-warning" role="alert">{library.error}</p>}
        </>}
        {!library.folder && <><p className="replay-library-empty">Choose the folder where you keep your replays.</p><ReplayFolderSetting library={library} client={client} /></>}
      </>}
    </Popover>
  </div>;
}

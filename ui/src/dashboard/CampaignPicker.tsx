import { useId, useState } from 'react';
import { Popover, usePopover } from '../components/Popover';
import { Icon } from '../components/Icons';
import { recordingName } from '../shell/api';
import { sameCampaignPath } from './campaigns';
import type { DashboardTemplate, WidgetSpec } from './model';
import type { WidgetRuntime } from './definitions';

const displayName = (path: string) => recordingName(path).replace(/\.(?:battleplan|hoi4replay)$/i, '');

/** A widget's campaign: the default, an open campaign, or a recording to open. Opening a
 * recording loads it once and shares it with every widget and map, as the sidebar does. */
export default function CampaignPicker({ widget, template, runtime }: { widget: WidgetSpec; template: DashboardTemplate; runtime: WidgetRuntime }) {
  const id = useId();
  const menu = usePopover('end');
  const [query, setQuery] = useState('');
  const [error, setError] = useState<string | null>(null);
  const open = runtime.campaigns.filter(campaign => !campaign.loading);
  const loading = runtime.campaigns.filter(campaign => campaign.loading);
  const roleId = widget.campaign === 'default' ? null : template.tracks.find(track => track.id === widget.trackIds[0])?.roleId ?? null;
  const chosen = roleId ? runtime.campaigns.find(campaign => campaign.id === roleId) : null;
  const defaultCampaign = runtime.campaigns.find(campaign => campaign.id === runtime.defaultRoleId);
  const label = widget.campaign === 'default' ? 'Default campaign' : chosen ? displayName(chosen.path)
    : `${template.roles.find(role => role.id === roleId)?.label ?? 'Campaign'} (not loaded)`;
  const words = query.trim().toLowerCase();
  const library = runtime.recordings.filter(file => !runtime.campaigns.some(campaign => sameCampaignPath(campaign.path, file.path))
    && `${file.name} ${file.folder}`.toLowerCase().includes(words));
  const choose = (value: string | null) => {
    setError(null);
    menu.close();
    void runtime.assignCampaign(widget.id, value).catch(cause => setError(cause instanceof Error ? cause.message : String(cause)));
  };
  return <div className="field">
    <span>Campaign</span>
    <button ref={menu.trigger} type="button" className="widget-country-trigger" popoverTarget={id} aria-haspopup="dialog" aria-expanded={menu.open} aria-label={`Widget campaign: ${label}`}>
      <Icon name={widget.campaign === 'default' ? 'star' : 'file'} /><span>{label}</span><Icon name="chevron-down" />
    </button>
    {error && <p className="map-menu-error" role="alert">{error}</p>}
    <Popover menu={menu} portal={false} id={id} className="campaign-picker" aria-label="Choose the widget's campaign" onToggle={event => { if (event.newState === 'closed') setQuery(''); }}>
      <button type="button" className="menu-item" aria-pressed={widget.campaign === 'default'} onClick={() => choose(null)}>
        <Icon name="star" /><span className="menu-item-label">Default campaign<small>{defaultCampaign ? `Now ${displayName(defaultCampaign.path)}` : 'Follows the campaign shown in the sidebar'}</small></span>
        {widget.campaign === 'default' && <Icon name="check" />}
      </button>
      {open.length > 0 && <div className="popover-label">Open</div>}
      {open.map(campaign => <button key={campaign.id} type="button" className="menu-item" aria-pressed={widget.campaign !== 'default' && roleId === campaign.id} onClick={() => choose(campaign.id)}>
        <Icon name="file" /><span className="menu-item-label">{displayName(campaign.path)}</span>
        {widget.campaign !== 'default' && roleId === campaign.id && <Icon name="check" />}
      </button>)}
      {loading.map(campaign => <div key={campaign.id} className="menu-item is-loading" role="status"><span className="spinner" /><span className="menu-item-label">Loading {displayName(campaign.path)}…</span></div>)}
      {runtime.recordings.length > 0 && <>
        <div className="popover-label">Open another recording</div>
        {runtime.recordings.length > 6 && <label className="search-field campaign-picker-search"><Icon name="search" />
          <input value={query} placeholder="Find a recording…" aria-label="Find a recording" onChange={event => setQuery(event.currentTarget.value)} /></label>}
        <div className="campaign-picker-library">{library.map(file => <button key={file.path} type="button" className="menu-item" onClick={() => choose(file.path)}>
          <Icon name="folder-open" /><span className="menu-item-label">{file.name}{file.folder && <small>{file.folder}</small>}</span>
        </button>)}
          {!library.length && <p className="field-hint">{words ? 'No matching recordings.' : 'Every recording in your folder is already open.'}</p>}</div>
        <p className="field-hint campaign-picker-note">Opening a recording keeps it loaded and shares it with every widget and map that uses it.</p>
      </>}
    </Popover>
  </div>;
}

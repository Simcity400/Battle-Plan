import { type Failure, asFailure, failureText } from './queryError.ts';
import { useId, useState } from 'react';
import { Popover, usePopover } from '../components/Popover';
import { Icon } from '../components/Icons';
import { FailureLine } from '../components/FailureLine';
import { sameCampaignPath } from './campaigns';
import { recordingDisplayName, recordingNameMatches } from './recordingNames';
import type { DashboardTemplate, WidgetSpec } from './model';
import type { WidgetRuntime } from './definitions';

export default function CampaignPicker({ widget, template, runtime }: { widget: WidgetSpec; template: DashboardTemplate; runtime: WidgetRuntime }) {
  const id = useId();
  const menu = usePopover('end');
  const [query, setQuery] = useState('');
  const [error, setError] = useState<Failure | null>(null);
  const displayName = recordingDisplayName;
  const open = runtime.campaigns.filter(campaign => !campaign.loading);
  const loading = runtime.campaigns.filter(campaign => campaign.loading);
  const roleId = widget.campaign === 'default' ? null : template.tracks.find(track => track.id === widget.trackIds[0])?.roleId ?? null;
  const chosen = roleId ? runtime.campaigns.find(campaign => campaign.id === roleId) : null;
  const defaultCampaign = runtime.campaigns.find(campaign => campaign.id === runtime.defaultRoleId);
  const label = widget.campaign === 'default' ? 'Default campaign' : chosen ? displayName(chosen.path)
    : `${template.roles.find(role => role.id === roleId)?.label ?? 'Campaign'} (not loaded)`;
  const words = query.trim().toLowerCase();
  const library = runtime.recordings.filter(file => !runtime.campaigns.some(campaign => sameCampaignPath(campaign.path, file.path))
    && (recordingNameMatches(file.path, words) || file.folder.toLowerCase().includes(words)));
  const choose = (value: string | null) => {
    setError(null);
    menu.close();
    void runtime.assignCampaign(widget.id, value).catch(cause => setError(asFailure(cause)));
  };
  return <div className="field">
    <span>Campaign</span>
    <button ref={menu.trigger} type="button" className="widget-country-trigger" popoverTarget={id} aria-haspopup="dialog" aria-expanded={menu.open} aria-label={`Widget campaign: ${label}`}>
      <Icon name={widget.campaign === 'default' ? 'star' : 'file'} /><span>{label}</span><Icon name="chevron-down" />
    </button>
    {error && <FailureLine className="campaign-picker-failure" detail={error.message} message={failureText(error, 'Couldn’t load campaign.')} />}
    <Popover menu={menu} portal={false} id={id} className="campaign-picker" aria-label="Choose the widget’s campaign" onToggle={event => { if (event.newState === 'closed') setQuery(''); }}>
      <button type="button" className="menu-item" aria-pressed={widget.campaign === 'default'} onClick={() => choose(null)}>
        <Icon name="star" /><span className="menu-item-label">Default campaign<small>{defaultCampaign ? `Now ${displayName(defaultCampaign.path)}` : 'Current campaign'}</small></span>
        {widget.campaign === 'default' && <Icon name="check" />}
      </button>
      {open.length > 0 && <div className="group-label">Open</div>}
      {open.map(campaign => <button key={campaign.id} type="button" className="menu-item" data-tooltip-overflow={displayName(campaign.path)} aria-pressed={widget.campaign !== 'default' && roleId === campaign.id} onClick={() => choose(campaign.id)}>
        <Icon name="file" /><span className="menu-item-label" data-overflow-label>{displayName(campaign.path)}</span>
        {widget.campaign !== 'default' && roleId === campaign.id && <Icon name="check" />}
      </button>)}
      {loading.map(campaign => <div key={campaign.id} className="menu-item is-loading" role="status"><span className="spinner" /><span className="menu-item-label">Loading {displayName(campaign.path)}…</span></div>)}
      {runtime.recordingsLoading && !library.length && <p className="field-hint" role="status">Reading your recordings folder…</p>}
      {runtime.recordings.length > 0 && <>
        <div className="group-label">Open another recording</div>
        {runtime.recordings.length > 6 && <label className="search-field campaign-picker-search"><Icon name="search" />
          <input type="search" value={query} placeholder="Find a recording" aria-label="Find a recording" onChange={event => setQuery(event.currentTarget.value)} /></label>}
        <div className="campaign-picker-library">{library.map(file => <button key={file.path} type="button" className="menu-item" data-tooltip-overflow={displayName(file.path)} onClick={() => choose(file.path)}>
          <Icon name="folder-open" /><span className="menu-item-label" data-overflow-label>{displayName(file.path)}{file.folder && <small>{file.folder}</small>}</span>
        </button>)}
          {!library.length && !runtime.recordingsLoading && <p className="field-hint">{words ? 'No matching recordings.' : 'Every recording in your folder is already open.'}</p>}</div>
      </>}
    </Popover>
  </div>;
}

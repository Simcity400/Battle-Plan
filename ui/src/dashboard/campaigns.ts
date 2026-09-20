import type { DashboardTemplate, WidgetSpec } from './model';

/** Session identities are stable across layout changes; paths never enter templates. */
export interface Campaign { id: string; path: string; label: string; loading?: boolean }

export const MAX_ROLES = 12;
export const MAX_TRACKS = 32;

/** Pending opens reserve capacity, but never become saved layout slots. */
export function campaignSlots(template: DashboardTemplate, campaigns: readonly Campaign[]) {
  const reserved = includeCampaigns(template, campaigns.map(campaign => ({ ...campaign, loading: false })));
  const empty = template.roles.filter(role => !campaigns.some(campaign => campaign.id === role.id)
    && (template.tracks.some(track => track.roleId === role.id) || reserved.tracks.length < MAX_TRACKS));
  return { empty, canAdd: reserved.roles.length < MAX_ROLES && reserved.tracks.length < MAX_TRACKS };
}

export function includeCampaigns(template: DashboardTemplate, campaigns: readonly Campaign[]): DashboardTemplate {
  campaigns = campaigns.filter(campaign => !campaign.loading);
  const missing = campaigns.filter((campaign) => !template.roles.some((role) => role.id === campaign.id));
  const missingTracks = campaigns.filter((campaign) => !template.tracks.some((track) => track.roleId === campaign.id));
  if (!missing.length && !missingTracks.length) return template;
  return { ...template,
    roles: [...template.roles, ...missing.map((campaign) => ({ id: campaign.id, label: campaign.label, selectedTag: null }))],
    tracks: [...template.tracks, ...missingTracks.map((campaign) => ({ id: `track_${campaign.id}`, roleId: campaign.id, label: campaign.label, branchIndex: 0, anchorOffset: null }))],
  };
}

/** A fixed choice never changes when the session's default campaign changes. */
export function resolveWidgetCampaign(widget: WidgetSpec, template: Pick<DashboardTemplate, 'tracks'>, defaultRoleId: string | null): WidgetSpec {
  if (widget.campaign !== 'default') return widget;
  const track = template.tracks.find((item) => item.roleId === defaultRoleId);
  return { ...widget, trackIds: track ? [track.id] : [] };
}

export function defaultAfterOpen(current: string | null, opened: string): string {
  return current ?? opened;
}

export function defaultAfterUnload(current: string | null, removed: string, loaded: readonly string[]): string | null {
  return current === removed ? loaded.find(id => id !== removed) ?? null : current;
}

export function sameCampaignPath(first: string, second: string): boolean {
  const normalize = (path: string) => path.replaceAll('\\', '/').toLowerCase()
    .replace(/^\/\/\?\/unc\//, '//').replace(/^\/\/\?\//, '');
  return normalize(first) === normalize(second);
}

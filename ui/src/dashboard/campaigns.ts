import type { DashboardTemplate, WidgetSpec } from './model';

export interface Campaign { id: string; path: string; label: string; loading?: boolean }

export function campaignSlots(template: DashboardTemplate, campaigns: readonly Campaign[]) {
  return { empty: template.roles.filter(role => !campaigns.some(campaign => campaign.id === role.id)) };
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

export const normalizeCampaignPath = (path: string) => path.replaceAll('\\', '/').toLowerCase()
  .replace(/^\/\/\?\/unc\//, '//').replace(/^\/\/\?\//, '');

const namesCampaign = (campaignKey: string) => /^(?:[0-9a-f]{2})*$/i.test(campaignKey)
  && /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(
    campaignKey.replace(/../g, pair => String.fromCharCode(parseInt(pair, 16))));

export const recordingKey = (path: string, campaignKey: string) => namesCampaign(campaignKey)
  ? `${normalizeCampaignPath(path).replace(/\/[^/]*$/, '')}\n${campaignKey}`
  : normalizeCampaignPath(path);

export function sameCampaignPath(first: string, second: string): boolean {
  return normalizeCampaignPath(first) === normalizeCampaignPath(second);
}

export type Relocation = readonly [string, string, string];

export function followRelocations(path: string, relocations: readonly Relocation[]): string {
  const first = relocations.findIndex(([from]) => sameCampaignPath(from, path));
  if (first < 0) return path;
  const identity = relocations[first][2];
  return relocations.slice(first).filter(([, , recording]) => recording === identity)
    .reduce((at, [from, to]) => sameCampaignPath(from, at) ? to : at, path);
}

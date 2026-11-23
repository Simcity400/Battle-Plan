import type { MapOptions } from '../shell/api';
import { MAP_OPTION_DEFAULTS } from '../generated/dashboard.ts';
import type { DashboardClient, DashboardMapView, MapObserveEvent, MapOptionsEvent, MapSettingsTarget, MapToggle, MapToggleEvent, DashboardSource } from './client';
import type { DashboardTemplate } from './model';
import { resolveWidgetCampaign } from './campaigns.ts';

export const CAMPAIGN_MAP_VIEW = 'campaign-map';

export const DEFAULT_MAP_OPTIONS: MapOptions = { country: null, fog: MAP_OPTION_DEFAULTS.fog };

export interface CountryChoice { tag: string; slot: number }

export function observedSwitches(previous: Record<string, number>, event: MapObserveEvent): Record<string, number> {
  const held = previous[event.viewId];
  return held !== undefined && held >= event.country.sequence ? previous : { ...previous, [event.viewId]: event.country.sequence };
}

export function observedRole(viewId: string, template: Pick<DashboardTemplate, 'tracks' | 'widgets'>, defaultRoleId: string | null): string | null {
  if (viewId === CAMPAIGN_MAP_VIEW) return defaultRoleId;
  const widget = template.widgets.find(item => item.id === viewId);
  if (!widget || widget.countries.kind === 'tags') return null;
  const trackId = resolveWidgetCampaign(widget, template, defaultRoleId).trackIds[0];
  return template.tracks.find(track => track.id === trackId)?.roleId ?? null;
}

export const flipFog = (options: MapOptions): MapOptions => ({ ...options, fog: !options.fog });

export function campaignToggled(event: MapToggleEvent, update: (flip: (options: MapOptions) => MapOptions) => void,
  client: DashboardClient, source: Pick<DashboardSource, 'id' | 'revision'> | null): Promise<string | null> {
  if (event.viewId !== CAMPAIGN_MAP_VIEW) return Promise.resolve(null);
  if (event.toggle.setting === 'fog') { update(flipFog); return Promise.resolve(null); }
  if (!source) return Promise.resolve(null);
  return toggleMapSetting(client, { id: CAMPAIGN_MAP_VIEW, sourceId: source.id, revision: source.revision }, event.toggle, { scope: 'campaign' });
}

export class PickerOptions {
  private picked = new Map<string, Pick<MapOptions, 'mapMode' | 'mapModeSlots'>>();
  private fogFlips = new Map<string, () => void>();

  merge(id: string, options: MapOptions | undefined): MapOptions | undefined {
    const picked = this.picked.get(id);
    return picked && options ? { ...options, ...picked } : options;
  }

  changed(event: MapOptionsEvent): void {
    const { mapMode, mapModeSlots } = event.options;
    this.picked.set(event.viewId, { mapMode: mapMode ?? null, mapModeSlots: mapModeSlots ?? [] });
  }

  toggled(viewId: string, toggle: MapToggle): boolean {
    if (toggle.setting !== 'fog') return false;
    this.fogFlips.get(viewId)?.();
    return true;
  }

  onFog(viewId: string, flip: () => void): () => void {
    this.fogFlips.set(viewId, flip);
    return () => { if (this.fogFlips.get(viewId) === flip) this.fogFlips.delete(viewId); };
  }
}

const settingWrites = new Map<string, Promise<string | null>>();

export function toggleMapSetting(client: DashboardClient, view: Pick<DashboardMapView, 'id' | 'sourceId' | 'revision'>,
  toggle: MapToggle, target: MapSettingsTarget = { scope: 'widget', id: view.id }): Promise<string | null> {
  const write = async () => {
    if (!client.flipMapSetting || toggle.setting === 'fog') return null;
    const signal = new AbortController().signal;
    return (await client.flipMapSetting(view.sourceId, view.revision, toggle.setting, signal, target)).saveWarning ?? null;
  };
  const key = JSON.stringify([view.sourceId, view.revision, target]);
  const next = (settingWrites.get(key) ?? Promise.resolve(null)).catch(() => null).then(write);
  settingWrites.set(key, next);
  void next.finally(() => { if (settingWrites.get(key) === next) settingWrites.delete(key); }).catch(() => undefined);
  return next;
}

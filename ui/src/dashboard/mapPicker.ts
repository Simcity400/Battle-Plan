import type { MapOptions } from '../shell/api';
import { MAP_OPTION_DEFAULTS } from '../generated/dashboard.ts';
import type { DashboardClient, DashboardMapView, MapObserveEvent, MapOptionsEvent, MapSettingsTarget, MapToggle, MapToggleEvent, DashboardSource } from './client';
import type { DashboardTemplate } from './model';
import { resolveWidgetCampaign } from './campaigns.ts';

/** The view id the desktop's campaign map reports its picker changes under. */
export const CAMPAIGN_MAP_VIEW = 'campaign-map';

/** A map view's options before any choice: no country chosen, fog at the map option default. */
export const DEFAULT_MAP_OPTIONS: MapOptions = { country: null, fog: MAP_OPTION_DEFAULTS.fog };

/** A campaign's chosen country: its canonical slot, which the map and the dashboard follow through tag changes, and
 * the tag it went by when chosen. */
export interface CountryChoice { tag: string; slot: number }

/** Each view's latest Ctrl+click switch applied, which its options echo: never moved back by an older one. */
export function observedSwitches(previous: Record<string, number>, event: MapObserveEvent): Record<string, number> {
  const held = previous[event.viewId];
  return held !== undefined && held >= event.country.sequence ? previous : { ...previous, [event.viewId]: event.country.sequence };
}

/** The campaign whose chosen country a map's Ctrl+click sets: the default campaign's for the campaign map and the
 * pop-out (both report as the campaign map), an unpinned widget map's own campaign's; none for a widget map whose
 * country is pinned, which keeps its own. */
export function observedRole(viewId: string, template: Pick<DashboardTemplate, 'tracks' | 'widgets'>, defaultRoleId: string | null): string | null {
  if (viewId === CAMPAIGN_MAP_VIEW) return defaultRoleId;
  const widget = template.widgets.find(item => item.id === viewId);
  if (!widget || widget.countries.kind === 'tags') return null;
  const trackId = resolveWidgetCampaign(widget, template, defaultRoleId).trackIds[0];
  return template.tracks.find(track => track.id === trackId)?.roleId ?? null;
}

/** A view's options with fog flipped: a picker fog click applied to the value the host holds now. */
export const flipFog = (options: MapOptions): MapOptions => ({ ...options, fog: !options.fog });

/** The campaign map and pop-out share fog options and campaign-scope graphics. Each native
 * picker click flips the host's value; it never writes a viewer's stale copy of the setting. */
export function campaignToggled(event: MapToggleEvent, update: (flip: (options: MapOptions) => MapOptions) => void,
  client: DashboardClient, source: Pick<DashboardSource, 'id' | 'revision'> | null): Promise<void> {
  if (event.viewId !== CAMPAIGN_MAP_VIEW) return Promise.resolve();
  if (event.toggle.setting === 'fog') { update(flipFog); return Promise.resolve(); }
  if (!source) return Promise.resolve();
  return toggleMapSetting(client, { id: CAMPAIGN_MAP_VIEW, sourceId: source.id, revision: source.revision }, event.toggle, { scope: 'campaign' });
}

/** What each map view's native picker chose, and where its fog toggle goes. The picker owns only
 * the map mode and slots; fog stays the view's own switch, which the toggle sets. */
export class PickerOptions {
  private picked = new Map<string, Pick<MapOptions, 'mapMode' | 'mapModeSlots'>>();
  private fogFlips = new Map<string, () => void>();

  /** A view's options with the map mode and slots its picker chose. */
  merge(id: string, options: MapOptions | undefined): MapOptions | undefined {
    const picked = this.picked.get(id);
    return picked && options ? { ...options, ...picked } : options;
  }

  changed(event: MapOptionsEvent): void {
    const { mapMode, mapModeSlots } = event.options;
    this.picked.set(event.viewId, { mapMode: mapMode ?? null, mapModeSlots: mapModeSlots ?? [] });
  }

  /** Flips a view's fog switch for a fog click; `false` for a toggle the host's settings own. */
  toggled(viewId: string, toggle: MapToggle): boolean {
    if (toggle.setting !== 'fog') return false;
    this.fogFlips.get(viewId)?.();
    return true;
  }

  /** Lets a view's picker flip its fog switch. */
  onFog(viewId: string, flip: () => void): () => void {
    this.fogFlips.set(viewId, flip);
    return () => { if (this.fogFlips.get(viewId) === flip) this.fogFlips.delete(viewId); };
  }
}

/** Each scope's settings writes, in order: a write reads what the last one saved. */
const settingWrites = new Map<string, Promise<void>>();

/** Flips the scope's setting under the service's save lock, reading its current value (own or
 * inherited), so an edit or reset saved meanwhile is what it flips. No other choice is written.
 * Campaign and widget flips are sent one at a time per source and scope, in click order. */
export function toggleMapSetting(client: DashboardClient, view: Pick<DashboardMapView, 'id' | 'sourceId' | 'revision'>,
  toggle: MapToggle, target: MapSettingsTarget = { scope: 'widget', id: view.id }): Promise<void> {
  const write = async () => {
    if (!client.flipMapSetting || toggle.setting === 'fog') return;
    const signal = new AbortController().signal;
    await client.flipMapSetting(view.sourceId, view.revision, toggle.setting, signal, target);
  };
  const key = JSON.stringify([view.sourceId, view.revision, target]);
  const next = (settingWrites.get(key) ?? Promise.resolve()).catch(() => undefined).then(write);
  settingWrites.set(key, next);
  void next.finally(() => { if (settingWrites.get(key) === next) settingWrites.delete(key); }).catch(() => undefined);
  return next;
}

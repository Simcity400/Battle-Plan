import type { MapOptions } from '../shell/api';
import type { DashboardClient, DashboardMapView, MapOptionsEvent, MapToggle, MapToggleEvent } from './client';

/** The view id the desktop's campaign map reports its picker changes under. */
export const CAMPAIGN_MAP_VIEW = 'campaign-map';

/** A view's options with fog flipped: a picker fog click applied to the value the host holds now. */
export const flipFog = (options: MapOptions): MapOptions => ({ ...options, fog: !options.fog });

/** The campaign map's host for a picker toggle: the campaign map's and the pop-out's fog clicks,
 * both under `campaign-map`, each flip the one campaign fog switch in the order they arrive. */
export function campaignToggled(event: MapToggleEvent, update: (flip: (options: MapOptions) => MapOptions) => void): void {
  if (event.viewId === CAMPAIGN_MAP_VIEW && event.toggle.setting === 'fog') update(flipFog);
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

/** Each widget's settings writes, in order: a write reads what the last one saved. */
const settingWrites = new Map<string, Promise<void>>();

/** Flips the widget-scope setting a native picker toggle stands for. The service flips it under
 * the lock its saves take, reading the value when the flip runs (the widget's own, else the one
 * it inherits), so an edit or reset saved meanwhile is what it flips and no other choice is
 * written. A widget's flips are sent one at a time, in click order. */
export function toggleMapSetting(client: DashboardClient, view: DashboardMapView, toggle: MapToggle): Promise<void> {
  const write = async () => {
    if (!client.flipMapSetting || toggle.setting === 'fog') return;
    const signal = new AbortController().signal;
    await client.flipMapSetting(view.sourceId, view.revision, toggle.setting, signal, { scope: 'widget', id: view.id });
  };
  const next = (settingWrites.get(view.id) ?? Promise.resolve()).catch(() => undefined).then(write);
  settingWrites.set(view.id, next);
  void next.finally(() => { if (settingWrites.get(view.id) === next) settingWrites.delete(view.id); }).catch(() => undefined);
  return next;
}

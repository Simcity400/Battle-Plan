import type { ComponentType } from 'react';
import type { IconName } from '../components/Icons';
import type { MapOptions } from '../shell/api';
import type { DashboardClient } from './client';
import type { MapViewsStore } from './geometry';
import type { Alignment, BoundTrack, DashboardTemplate, TrackSpec, WidgetSpec, WidgetTime } from './model';
import type { PlaybackController } from './playback';

/** What every widget receives from the dashboard. Widgets never open files or
 * start their own clock; they read products and follow the shared playback. */
export interface WidgetRuntime {
  campaignTracks: TrackSpec[];
  defaultRoleId: string | null;
  tracks: BoundTrack[];
  clock: PlaybackController;
  maps: MapViewsStore;
  onMapBranch: (trackId: string, branchId: string) => void;
  client: DashboardClient;
  alignment: Alignment;
  nativeHidden: boolean;
}

export interface WidgetProps {
  widget: WidgetSpec;
  runtime: WidgetRuntime;
  active: boolean;
  mapOptions: MapOptions;
  /** A span inside the widget header for a short live summary. */
  headerSlot: HTMLElement | null;
  onDiagnostics: (warnings: string[]) => void;
}

export interface HeaderControlsProps {
  widget: WidgetSpec;
  runtime: WidgetRuntime;
  template: DashboardTemplate;
  editing: boolean;
  onApply: (widget: WidgetSpec) => void;
  onRemove?: () => void;
  onReset?: () => void;
  mapOptions: MapOptions;
  onMapOptions: (options: MapOptions) => void;
  diagnostics: string[];
}

/** Pure widget metadata: safe to import in node tests. */
export interface WidgetMeta {
  kind: string;
  version: number;
  title: string;
  icon: IconName;
  timeModes: readonly WidgetTime['mode'][];
  defaultTime: 'follow' | 'campaign';
  scope: 'moment' | 'history';
  /** Reads recording data: shows the track label and a country choice. */
  data: boolean;
  minSize: { w: number; h: number };
  defaultSize: { w: number; h: number };
}

export interface WidgetDefinition extends WidgetMeta {
  render?: ComponentType<WidgetProps>;
  /** Replaces the default settings gear, as the map's pop-down does. */
  headerControls?: ComponentType<HeaderControlsProps>;
}

/** The React half of a widget; its folder's index.ts exports one. */
export type WidgetRenderer = Pick<WidgetDefinition, 'render' | 'headerControls'>;

const registry = new Map<string, WidgetDefinition>();

/** Register widgets; a later registration of the same kind adds to the earlier one. */
export function registerWidgets(definitions: readonly WidgetDefinition[]): void {
  for (const definition of definitions) registry.set(definition.kind, { ...registry.get(definition.kind), ...definition });
}
export function widgetDefinitions(): WidgetDefinition[] { return [...registry.values()]; }
export function widgetDefinition(kind: string): WidgetDefinition | undefined { return registry.get(kind); }

export function widgetUnavailable(widget: WidgetSpec): string | null {
  const definition = widgetDefinition(widget.kind);
  if (!definition) return `Widget type “${widget.kind}” is not installed. Its saved settings are retained.`;
  if (widget.configVersion !== definition.version) return `Widget settings version ${widget.configVersion} is unsupported. Its saved settings are retained.`;
  if (!definition.timeModes.includes(widget.time.mode)) return 'This widget does not support that time mode. Choose a supported mode in its settings.';
  return null;
}

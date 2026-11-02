import type { ComponentType } from 'react';
import type { IconName } from '../components/Icons';
import type { CaptureMode } from '../generated/dashboard';
import type { MapOptions } from '../shell/api';
import type { DashboardClient, ReplayFile } from './client';
import type { Campaign } from './campaigns';
import type { MapViewsStore } from './geometry';
import type { Alignment, BoundTrack, CountrySelection, DashboardTemplate, TrackSpec, WidgetSpec, WidgetTime } from './model';
import type { PlaybackController } from './playback';
import type { OptionField, OptionValue } from './options';

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
  /** Open campaigns and the replay library, for a widget's campaign choice. */
  campaigns: Campaign[];
  recordings: ReplayFile[];
  /** Points a widget at the default campaign (null), an open campaign's id or a recording path to open. */
  assignCampaign: (widgetId: string, chosen: string | null) => Promise<void>;
  /** Asks the shared clock to show a track's recorded hour ("Jump to"). False when
   * the hour is outside that track's branch; the clock still owns time. */
  requestSeek: (trackId: string, rawHour: number) => boolean;
  /** Adds a widget below another as a layout change, such as a pinned breakdown. */
  addWidgetBelow: (sourceId: string, widget: WidgetSpec) => void;
}

export interface WidgetProps {
  widget: WidgetSpec;
  runtime: WidgetRuntime;
  active: boolean;
  mapOptions: MapOptions;
  /** A span inside the widget header for a short live summary. */
  headerSlot: HTMLElement | null;
  /** A semantic drill's country chip target, separate from the title summary. */
  countrySlot?: HTMLElement | null;
  /** Receives the notes behind what the widget cannot show (unread texts, ambiguous artwork), each
   * named and counted. Tests read them; the dashboard does not display them. */
  onDiagnostics: (warnings: string[]) => void;
  /** Saves one of the widget's own options from inside it (such as chips above a
   * chart), as the settings panel would; absent where the widget is not in a layout. */
  setOption?: (key: string, value: OptionValue) => void;
  /** Saves several of the widget's options as one change, so a stored dashboard never
   * holds only some of them (such as a migration and its marker). */
  setOptions?: (values: Readonly<Record<string, OptionValue>>) => void;
}

export interface HeaderControlsProps {
  widget: WidgetSpec;
  runtime: WidgetRuntime;
  template: DashboardTemplate;
  editing: boolean;
  /** `view` marks a view preference (Live, size) that saves immediately. */
  onApply: (widget: WidgetSpec, view?: boolean) => void;
  onRemove?: () => void;
  onReset?: () => void;
  mapOptions: MapOptions;
  onMapOptions: (options: MapOptions) => void;
}

/** Pure widget metadata: safe to import in node tests. */
export interface WidgetMeta {
  kind: string;
  version: number;
  title: string;
  /** One line for the widget picker. */
  description?: string;
  /** The widget catalogue's group; by default the first subject page that shows the widget. */
  category?: string;
  /** False when a second copy cannot be drawn in the catalogue's preview (the native map). */
  preview?: false;
  icon: IconName;
  timeModes: readonly WidgetTime['mode'][];
  defaultTime: 'follow' | 'campaign';
  scope: 'moment' | 'history';
  /** Reads recording data: shows the track label and, by default, a country choice. */
  data: boolean;
  /** False for whole-world views without a country chip or Countries settings. */
  countrySelection?: boolean;
  /** Uses the campaign's shared faction colours and offers their editor in settings. */
  factionColours?: boolean;
  /** Has a meaningful whole-campaign view, so the header offers a Live switch.
   * Without one (a map) the widget always follows playback. */
  history: boolean;
  minSize: { w: number; h: number };
  defaultSize: { w: number; h: number };
  /** How many countries the widget shows at once (1 when absent); whether it ranks
   * countries by its value (offering a top-N group) and can sum a group into one series.
   * `all`: the widget shows the whole world by default (every faction, every
   * overlord, every country's construction), and chosen countries or a group
   * narrow it. `factions`: offers the shared faction-totals selection. `fixed`:
   * a single picked country stays explicit even when it matches the dashboard. `rankedBy`:
   * what a top N ranks by when it is not the widget's own value. */
  countries?: { max: number; ranked?: boolean; rankedBy?: string; aggregate?: boolean; all?: boolean; factions?: boolean; fixed?: boolean };
  /** The countries a new or reset widget shows (the dashboard's country when absent),
   * such as a race's top 10 of all countries. */
  defaultCountries?: CountrySelection;
  /** The widget's own settings, grouped into the panel's sections. */
  options?: readonly OptionField[];
  /** The capture modes whose recordings hold every table the widget reads; Full only when absent. */
  captureModes?: readonly CaptureMode[];
  /** A lightweight product reporting whether this recording contains the feature.
   * The picker disables the entry until the reply establishes `available: true`. */
  availability?: { product: string; params: Readonly<Record<string, unknown>> };
  /** Upgrades settings saved by an older `version`; null keeps them unchanged. */
  migrate?: (from: number, widget: WidgetSpec) => WidgetSpec | null;
  /** The widget's other views as their own catalogue cards, listed after its own: each adds this
   * kind opening in that view, and the view stays switchable in its settings. */
  variants?: readonly WidgetPreset[];
}

/** Run-time choices for a widget's content-choice and content-set fields, by field
 * key; a content set shows each choice's icon (an image URL) when it has one. */
export type OptionChoices = Readonly<Record<string, readonly { value: string; label: string; icon?: string | null; detail?: string }[]>>;

/** One catalogue entry for a view of the widget (`variants`) or derived from the campaign's content
 * (one idea category). It is its own card and stores the definition's kind with this title and these options. */
export interface WidgetPreset { key: string; initial: Pick<WidgetSpec, 'title' | 'options'> }
/** A content-derived widget's entries for the shown campaign; `reason` says why there are none yet. */
export interface WidgetPresets { presets: readonly WidgetPreset[]; reason: string | null; detail?: string }

export type WidgetCountryChipProps = Pick<WidgetProps, 'widget' | 'runtime' | 'active'> & {
  onSlot: (element: HTMLSpanElement | null) => void;
};

export interface WidgetDefinition extends WidgetMeta {
  render?: ComponentType<WidgetProps>;
  /** Splits the widget into one catalogue entry per content choice, replacing its own card while
   * there are any. The catalogue calls it for every such widget while open; the registry is fixed
   * once widgets register, so the calls keep one order. */
  usePresets?: (definition: WidgetMeta, runtime: WidgetRuntime, track: BoundTrack | undefined, active: boolean) => WidgetPresets;
  /** A hook listing the choices of the widget's content-choice fields from its
   * campaign's content. The settings panel calls it while open. */
  useOptionChoices?: (widget: WidgetSpec, runtime: WidgetRuntime) => OptionChoices;
  /** A semantic drill mounts its countrySlot target in the default chip position. */
  countryChip?: ComponentType<WidgetCountryChipProps>;
  /** Replaces the default settings gear, as the map's pop-down does. */
  headerControls?: ComponentType<HeaderControlsProps>;
}

/** The React half of a widget; its folder's index.ts exports one. */
export type WidgetRenderer = Pick<WidgetDefinition, 'render' | 'headerControls' | 'countryChip' | 'useOptionChoices' | 'usePresets'>;

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

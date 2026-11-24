import type { ComponentType } from 'react';
import type { IconName } from '../components/Icons';
import type { CaptureMode } from '../generated/dashboard';
import type { MapOptions } from '../shell/api';
import type { DashboardClient, ReplayFile } from './client';
import type { Campaign } from './campaigns';
import type { MapViewsStore } from './geometry';
import type { Alignment, BoundTrack, CountrySelection, TrackSpec, WidgetSpec, WidgetTime } from './model';
import type { PlaybackController } from './playback';
import type { OptionField, OptionValue } from './options';
import type { Failure } from './queryError';
import type { PinnedSpec } from './regionTime';
import type { DeclaredProduct } from './request';
import type { FamilyProjection } from '../families/contract';
import type { FamilyId } from '../families/plan';

export interface FamilyWidgetDeclaration {
  readonly family: FamilyId;
  readonly kind: string;
  readonly projection: FamilyProjection<any, any>;
}

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
  campaigns: Campaign[];
  recordings: ReplayFile[];
  assignCampaign: (widgetId: string, chosen: string | null) => Promise<void>;
  requestSeek: (trackId: string, rawHour: number) => boolean;
  addWidgetBelow: (sourceId: string, widget: PinnedSpec) => void;
}

export interface WidgetProps {
  widget: WidgetSpec;
  runtime: WidgetRuntime;
  active: boolean;
  mapOptions: MapOptions;
  headerSlot: HTMLElement | null;
  countrySlot?: HTMLElement | null;
  onDiagnostics: (warnings: string[]) => void;
  setOption?: (key: string, value: OptionValue) => void;
  setOptions?: (values: Readonly<Record<string, OptionValue>>) => void;
  view?: string;
}

export interface WidgetViewSwitch {
  option: string;
  label?: string;
  choices?: readonly { value: string; label: string }[];
  when?: (options: Readonly<Record<string, OptionValue>>) => boolean;
}

export type WidgetSkeletonKind = 'generic' | 'table' | 'chart' | 'log' | 'sunburst' | 'list';

export interface WidgetMeta {
  kind: string;
  version: number;
  title: string;
  subject?: string;
  things?: string;
  viewSwitch?: WidgetViewSwitch;
  skeleton?: WidgetSkeletonKind;
  description?: string;
  category?: string;
  preview?: false;
  icon: IconName;
  timeModes: readonly WidgetTime['mode'][];
  defaultTime: 'follow' | 'campaign';
  scope: 'moment' | 'history';
  data: boolean;
  countrySelection?: boolean;
  factionColours?: boolean;
  history: boolean;
  minSize: { w: number; h: number };
  defaultSize: { w: number; h: number };
  countries?: { max: number; ranked?: boolean; rankedBy?: string; aggregate?: boolean; all?: boolean; factions?: boolean; fixed?: boolean };
  playsBackDated?: boolean;
  defaultCountries?: CountrySelection;
  options?: readonly OptionField[];
  captureModes?: readonly CaptureMode[];
  availability?: { product: DeclaredProduct; params: Readonly<Record<string, unknown>> };
  migrate?: (from: number, widget: WidgetSpec) => WidgetSpec | null;
  variants?: readonly WidgetPreset[];
}

export interface OptionChoice { value: string; label: string; icon?: string | null; detail?: string }
export type ChoicesUnavailable = { loading: true } | { error: Failure; retry: () => void };
export type OptionChoices = Readonly<Record<string, readonly OptionChoice[] | ChoicesUnavailable>>;
export const listedChoices = (choices: OptionChoices[string] | undefined): readonly OptionChoice[] | undefined =>
  choices === undefined || 'loading' in choices || 'error' in choices ? undefined : choices;
export const unavailableChoices = (choices: OptionChoices[string] | undefined): ChoicesUnavailable | undefined =>
  choices !== undefined && ('loading' in choices || 'error' in choices) ? choices : undefined;

export interface WidgetPreset { key: string; initial: Pick<WidgetSpec, 'title' | 'options'> }
export interface WidgetPresets { presets: readonly WidgetPreset[]; reason: string | null; detail?: string }

export type WidgetCountryChipProps = Pick<WidgetProps, 'widget' | 'runtime' | 'active'> & {
  onSlot: (element: HTMLSpanElement | null) => void;
  compact?: boolean;
};

export interface WidgetSubjectMenuProps {
  readonly widget: WidgetSpec;
  readonly runtime: WidgetRuntime;
  readonly onApply: (widget: WidgetSpec) => void;
}

export interface WidgetDefinition extends WidgetMeta {
  render?: ComponentType<WidgetProps>;
  declaration?: FamilyWidgetDeclaration;
  usePresets?: (definition: WidgetMeta, runtime: WidgetRuntime, track: BoundTrack | undefined, active: boolean) => WidgetPresets;
  useOptionChoices?: (widget: WidgetSpec, runtime: WidgetRuntime) => OptionChoices;
  countryChip?: ComponentType<WidgetCountryChipProps>;
  subjectMenu?: ComponentType<WidgetSubjectMenuProps>;
}

export type WidgetRenderer = Pick<WidgetDefinition, 'render' | 'declaration' | 'countryChip' | 'subjectMenu' | 'useOptionChoices' | 'usePresets'>;

const registry = new Map<string, WidgetDefinition>();

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

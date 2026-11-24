import { CHART_MEASURE_COLOURS } from '../../charts/palette.ts';
import type { FamilyAuditRow } from '../auditRow.ts';
import type { TooltipListItem } from '../host/tooltips.ts';
import type { ArtResult, BoundedSeries } from '../plan.ts';

export { CHART_MEASURE_COLOURS };

export interface ChartArtSlot {
  readonly art: ArtResult;
  readonly label?: string;
  readonly tag?: string;
}

export interface ChartMissingSpan {
  readonly firstHour: number;
  readonly lastHour: number;
  readonly reason: string;
}

export interface ChartGap {
  readonly firstHour: number;
  readonly lastHour: number;
  readonly reason: string;
}

export interface ChartEndLabel {
  readonly colorBar?: string;
  readonly flag?: ChartArtSlot;
  readonly icon?: ChartArtSlot;
  readonly name?: string | null;
  readonly shortName?: string | null;
  readonly fullName?: string;
  readonly value: string;
  readonly exactValue?: string;
  readonly valueColor?: string;
  readonly valueMuted?: boolean;
}

export interface ChartInlinePoint {
  readonly hour: number;
  readonly value: number | null;
}

export interface TimeChartSeriesModel {
  readonly id: string;
  readonly key: string;
  readonly name: string;
  readonly color: string;
  readonly role: 'primary' | 'secondary' | 'ceiling' | 'tail';
  readonly step?: boolean;
  readonly dash?: readonly number[];
  readonly opacity?: number;
  readonly width?: number;
  readonly area?: boolean;
  readonly negRed?: boolean;
  readonly bounded?: BoundedSeries | null;
  readonly valueScale?: number;
  readonly points?: readonly ChartInlinePoint[];
  readonly headValue?: number | null;
  readonly headHour?: number | null;
  readonly endLabel?: ChartEndLabel | null;
  readonly flag?: ChartArtSlot;
  readonly icon?: ChartArtSlot;
  readonly missingSpans?: readonly ChartMissingSpan[];
  readonly absentReason?: string | null;
  readonly exact?: { readonly hour: number; readonly value: number | null; readonly reason?: string } | null;
  readonly rank?: number;
}

export interface ChartStackBand {
  readonly id: string;
  readonly key: string;
  readonly name: string;
  readonly color: string;
  readonly opacity?: number;
  readonly values: readonly (number | null)[];
  readonly endLabel?: ChartEndLabel | null;
  readonly flag?: ChartArtSlot;
}

export interface ChartStackModel {
  readonly hours: readonly number[];
  readonly bands: readonly ChartStackBand[];
  readonly step?: boolean;
  readonly total?: {
    readonly color?: string;
    readonly values: readonly (number | null)[];
    readonly endLabel?: ChartEndLabel | null;
  };
}

export interface ChartColumnSegment {
  readonly id: string;
  readonly label: string;
  readonly color: string;
  readonly value: number;
  readonly exactText: string;
}

export interface ChartMonthSlot {
  readonly year: number;
  readonly month: number;
  readonly firstHour: number;
  readonly lastHour: number;
  readonly status: 'recorded' | 'no-rows' | 'not-recorded';
  readonly reason?: string;
  readonly partial?: boolean;
  readonly partialToHour?: number;
  readonly segments: readonly ChartColumnSegment[];
  readonly totalValue: number;
  readonly totalText: string;
}

export interface ChartColumnModel {
  readonly foldUnrecorded: boolean;
  readonly foldThreshold?: number;
  readonly months: readonly ChartMonthSlot[];
  readonly unit?: string;
}

export interface ChartMarkerItem {
  readonly title: string;
  readonly detail?: string;
  readonly icon?: ChartArtSlot;
  readonly flag?: ChartArtSlot;
}

export interface ChartMarker {
  readonly id: string;
  readonly hour: number;
  readonly title: string;
  readonly detail?: string;
  readonly icon?: ChartArtSlot;
  readonly flag?: ChartArtSlot;
  readonly tickColor?: string;
  readonly items?: readonly ChartMarkerItem[];
}

export interface ChartLegendEntry {
  readonly id: string;
  readonly label: string;
  readonly color?: string;
  readonly flag?: ChartArtSlot;
  readonly dash?: readonly number[];
  readonly hatch?: boolean;
}

export interface ChartOverlayLabel {
  readonly position: 'top-right' | 'bottom-right';
  readonly text: string;
  readonly arrow?: 'right' | 'left';
  readonly icon?: ChartArtSlot;
  readonly color?: string;
}

export interface ChartHoverDetail {
  readonly hour: number;
  readonly items: readonly TooltipListItem[];
  readonly replacesSeries?: boolean;
}

export type ChartValueFormat = 'number' | 'percent' | 'compact' | 'signed1' | 'power';

export interface ChartYAxisSpec {
  readonly format: ChartValueFormat;
  readonly min?: number | null;
  readonly max?: number | null;
  readonly fixed?: readonly [number, number] | null;
  readonly ticks?: readonly number[];
  readonly log?: boolean;
  readonly decimals?: number;
}

export interface TimeChartModel {
  readonly kind: string;
  readonly measureTitle: string;
  readonly mode: 'lines' | 'stack' | 'columns';
  readonly hour: number;
  readonly extent: readonly [number, number];
  readonly view?: { readonly x?: readonly [number, number] | null; readonly y?: readonly [number, number] | null };
  readonly yAxis: ChartYAxisSpec;
  readonly series: readonly TimeChartSeriesModel[];
  readonly stack?: ChartStackModel;
  readonly columns?: ChartColumnModel;
  readonly markers?: readonly ChartMarker[];
  readonly gaps?: readonly ChartGap[];
  readonly legend?: readonly ChartLegendEntry[];
  readonly overlays?: readonly ChartOverlayLabel[];
  readonly note?: string | null;
  readonly hoverDetail?: ChartHoverDetail | null;
  readonly warnings?: readonly string[];
  readonly empty?: {
    readonly message: string;
    readonly detail?: string;
    readonly kind?: 'empty' | 'unavailable';
  } | null;
  readonly auditRows?: readonly FamilyAuditRow[];
}

export interface TimeChartSettings {
  readonly foldUnrecorded?: boolean;
  readonly logScale?: boolean;
  readonly showMarkers?: boolean;
  readonly showArea?: boolean;
}

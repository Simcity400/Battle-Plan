import type { CountryFlagLook } from '../../components/CountryFlag';
import type { ChartTokens } from '../tokens.ts';

export interface TimelineSpan {
  id: string;
  firstHour: number;
  lastHour: number;
  label: string;
  icon?: string;
  icons?: readonly (string | undefined)[];
  countryFlag?: CountryFlagLook;
  shade?: number;
  emphasis?: boolean;
  marker?: boolean;
  value?: number;
  positiveCount?: number;
  negativeCount?: number;
  dot?: boolean;
  stack?: number;
  positive?: boolean;
  endLabel?: string;
  color?: string;
  glyph?: string;
  glyphHour?: number;
  glyphHours?: readonly number[];
  glyphShape?: 'cross';
  openEnd?: boolean;
  subrow?: number;
  barHeight?: number;
  band?: readonly BandStep[];
  stops?: readonly number[];
}

export interface BandStep { firstHour: number; lastHour: number; value: number }

interface LaneGeometry { top: number; height: number; left: number; right: number; hours: readonly [number, number]; toX: (hour: number) => number; playhead: number | null }

export interface TimelineLane {
  id: string;
  label: string;
  spans: readonly TimelineSpan[];
  icons?: boolean;
  names?: boolean;
  identity?: { label: string; icon?: string; countryFlag?: CountryFlagLook; tile?: 'blueprint' };
  muted?: boolean;
  stepMax?: number;
  barMax?: number;
  compact?: boolean;
  depth?: number;
  heading?: boolean;
  expanded?: boolean;
  opens?: boolean;
  singleLine?: boolean;
  rowHeight?: number;
  subrows?: number;
  barHeight?: number;
  labelStyle?: 'side';
  separator?: false;
  separatorAfter?: boolean;
  pill?: { text: string; color: string };
  paint?: (ctx: CanvasRenderingContext2D, lane: LaneGeometry, tokens: ChartTokens) => void;
}

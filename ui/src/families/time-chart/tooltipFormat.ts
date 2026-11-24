import { badgeRows, type EventBadge } from '../../charts/badges.ts';
import { dayLabel } from '../../charts/draw.ts';
import type { MarkCluster } from '../../charts/markSummary.ts';
import { runAt } from '../../charts/series.ts';
import { fitTooltip, type TooltipContent, type TooltipRow } from '../../charts/tooltip.ts';
import { isMember } from '../../dashboard/groups.ts';
import type { HourSpan, SeriesDrop } from '../../generated/dashboard';
import { formatBalance, netSpend } from '../../widgets/country-measure/measureData.ts';
import type { ReachedLine } from '../../widgets/country-measure/measureSeriesData.ts';
import type { TimeChartLegendReading } from './laneView.ts';

export interface TooltipSeriesItem {
  readonly tag: string;
  readonly hex: string;
  readonly membership?: readonly HourSpan[];
}

export interface HoverLabel {
  readonly name: string;
  readonly flagUrl?: string | null;
  readonly flagPending?: boolean;
  readonly flagDetail?: string;
  readonly flagReason?: string;
}

export function formatLegendReading(
  reading: TimeChartLegendReading | undefined,
  decimals: number | null,
  digits: number,
  loaded: boolean,
): string {
  if (!reading) return loaded ? 'Not recorded' : 'Loading…';
  if (reading.notApplicable === true) return 'Not applicable';
  if (reading.valueRaw !== null && decimals !== null) {
    return formatBalance(reading.valueRaw, decimals, digits);
  }
  return 'Not recorded';
}

export function buildMeasureTooltip(input: {
  readonly hour: number;
  readonly series: readonly TooltipSeriesItem[];
  readonly lineByTag: ReadonlyMap<string, ReachedLine>;
  readonly hourLoaded: (tag: string, hour: number) => boolean;
  readonly labelAt: (hour: number) => (tag: string) => HoverLabel;
  readonly decimals: number | null;
  readonly digits: number;
  readonly unit: string;
  readonly badges: readonly EventBadge[];
  readonly viewportHeight: number;
}): TooltipContent {
  const { hour, series, lineByTag, hourLoaded, labelAt, decimals, digits, unit, badges, viewportHeight } = input;
  const whole = Math.floor(hour);
  const labelFor = labelAt(hour);
  const rows: TooltipRow[] = series.map(item => {
    const label = labelFor(item.tag);
    const line = lineByTag.get(item.tag);
    const member = isMember(item.membership, hour);
    const run = member ? runAt(line?.runs ?? [], hour) : null;
    const raw = run?.valueRaw ?? null;
    const isNotApplicable = member && (line?.notApplicable.has(whole) ?? false);
    const missing = line?.missing.get(whole);
    const loaded = !member || hourLoaded(item.tag, whole);
    if (isNotApplicable) {
      return {
        label: label.name,
        countryFlag: label,
        swatch: item.hex,
        value: 'Not applicable',
        detail: missing,
        tone: 'muted',
      };
    }
    if (raw === null || decimals === null) {
      return {
        label: label.name,
        countryFlag: label,
        swatch: item.hex,
        value: !loaded ? 'Loading balance…' : 'Not recorded',
        detail: loaded ? missing : undefined,
        tone: 'muted',
      };
    }
    return {
      label: label.name,
      countryFlag: label,
      swatch: item.hex,
      value: `${formatBalance(raw, decimals, digits)} ${unit}`,
    };
  });

  if (decimals !== null) {
    for (const item of series) {
      const drop: SeriesDrop | undefined = lineByTag.get(item.tag)?.drops.find(entry => entry.hour === whole);
      if (drop) {
        rows.push({
          label: series.length > 1 ? `${labelFor(item.tag).name}: balance fell` : 'Balance fell',
          value: netSpend(drop, decimals, digits),
          divider: true,
        });
      }
    }
  }

  rows.push(...badgeRows(badges));
  return fitTooltip([{ title: dayLabel(hour), rows }], viewportHeight);
}

export function buildMarkTooltip(input: {
  readonly cluster: MarkCluster;
  readonly lineByTag: ReadonlyMap<string, ReachedLine>;
  readonly labelAt: (hour: number) => (tag: string) => HoverLabel;
  readonly decimals: number | null;
  readonly digits: number;
}): TooltipContent {
  const { cluster, lineByTag, labelAt, decimals, digits } = input;
  const label = cluster.tag ? labelAt(cluster.firstHour)(cluster.tag) : null;
  const drop = cluster.tag ? lineByTag.get(cluster.tag)?.drops.find(entry => entry.hour === cluster.firstHour) : undefined;
  return {
    title: cluster.firstHour === cluster.lastHour ? dayLabel(cluster.firstHour) : `${dayLabel(cluster.firstHour)}–${dayLabel(cluster.lastHour)}`,
    countryFlag: label ?? undefined,
    subtitle: label?.name,
    rows: [
      { label: 'Recorded balance changes', value: cluster.count },
      ...(drop && decimals !== null ? [{ label: 'Balance fell', value: netSpend(drop, decimals, digits) }] : []),
    ],
    note: 'Open Balance changes from the chart menu to reach each event.',
  };
}

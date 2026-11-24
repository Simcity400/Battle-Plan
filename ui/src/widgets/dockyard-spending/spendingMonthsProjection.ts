import { auditRow } from '../../families/auditRow.ts';
import { categoricalPalette, reservedGrey } from '../../charts/palette.ts';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import { icDecimals } from '../../dashboard/icDecimals.ts';
import type { OptionValue } from '../../dashboard/options';
import type { BoundedReadView, FamilyPlanContext, SourcePlan } from '../../families/contract.ts';
import { registerFamilyProjection } from '../../families/registry.ts';
import type {
  TimeChartProjection,
  TimeChartWidgetDeclaration,
} from '../../families/time-chart/declaration.ts';
import type {
  ChartColumnSegment,
  ChartLegendEntry,
  ChartMonthSlot,
  TimeChartModel,
} from '../../families/time-chart/model.ts';
import { formatScalar } from '../../families/valueFormat.ts';
import type {
  Domain,
  ProductionSpendingMonthsReply,
  SpendingMonthBandSpec,
} from '../../generated/dashboard';

export interface SpendingMonthsSpec {
  readonly kind: string;
  readonly name: string;
  readonly measure: string;
  readonly subject: string;
  readonly repairs: boolean;
  readonly domain: (options: Readonly<Record<string, OptionValue>>) => Domain;
  readonly measureTitle: (options: Readonly<Record<string, OptionValue>>) => string;
}

export interface SpendingMonthsOptions {
  readonly domain: Domain;
  readonly measureTitle: string;
  readonly digits: number;
  readonly surface: string;
  readonly extent: readonly [number, number];
}

const RAW_INT = /^-?\d{1,39}$/;

export function validateProductionSpendingMonths(raw: unknown): ProductionSpendingMonthsReply {
  const reply = raw as ProductionSpendingMonthsReply;
  if (
    !reply
    || typeof reply !== 'object'
    || typeof reply.branchId !== 'string'
    || typeof reply.tag !== 'string'
    || typeof reply.decimals !== 'number'
    || typeof reply.recorded !== 'boolean'
    || !Array.isArray(reply.bands)
    || !Array.isArray(reply.months)
    || !Array.isArray(reply.warnings)
  ) {
    throw new Error('Invalid production spending months reply');
  }
  for (const month of reply.months) {
    if (!month || !Array.isArray(month.parts) || month.parts.length !== reply.bands.length) {
      throw new Error('Invalid production spending months bar');
    }
    for (const part of month.parts) {
      if (part.raw !== null && (typeof part.raw !== 'string' || !RAW_INT.test(part.raw))) {
        throw new Error('Invalid production spending months part amount');
      }
    }
  }
  return reply;
}

export function formatExactIcText(raw: string, denominator: number, decimals: number, digits: number): string {
  const shown = Math.max(0, Math.min(digits, decimals));
  const num = BigInt(raw) * (10n ** BigInt(shown));
  const den = BigInt(denominator) * (10n ** BigInt(decimals));
  const half = den / 2n;
  const rounded = num < 0n ? -((-num + half) / den) : (num + half) / den;
  return formatScalar(rounded, { decimals: shown, unit: 'IC' });
}

function bandColour(band: SpendingMonthBandSpec, index: number, surface: string): string {
  if (band.id === 'purpose-unrecorded') return reservedGrey(surface);
  const palette = categoricalPalette(surface);
  if (band.id === 'other-purpose') return palette[4 % palette.length]!;
  return palette[index % palette.length]!;
}

export function createSpendingMonthsView(spec: SpendingMonthsSpec): {
  readonly projection: TimeChartProjection<SpendingMonthsOptions>;
  readonly declaration: TimeChartWidgetDeclaration<SpendingMonthsOptions>;
} {
  const projection: TimeChartProjection<SpendingMonthsOptions> = registerFamilyProjection({
    kind: spec.kind,
    family: 'time-chart',
    variant: 'columns',
    joinMode: 'join',
    measureTitle: spec.name,
    project(readView: BoundedReadView, options: SpendingMonthsOptions): TimeChartModel {
      const hour = readView.hour;
      const reply = readView.panel<ProductionSpendingMonthsReply>('main');
      const refused = readView.refused('main');
      if (refused) {
        return {
          kind: spec.kind,
          measureTitle: options.measureTitle,
          mode: 'columns',
          hour,
          extent: options.extent,
          yAxis: { format: 'compact', min: 0 },
          series: [],
          empty: { message: refused, kind: 'unavailable' },
        };
      }
      if (!reply) {
        return {
          kind: spec.kind,
          measureTitle: options.measureTitle,
          mode: 'columns',
          hour,
          extent: options.extent,
          yAxis: { format: 'compact', min: 0 },
          series: [],
        };
      }
      if (!reply.recorded) {
        return {
          kind: spec.kind,
          measureTitle: options.measureTitle,
          mode: 'columns',
          hour,
          extent: options.extent,
          yAxis: { format: 'compact', min: 0 },
          series: [],
          warnings: reply.warnings,
          empty: { message: `${spec.name} wasn\u2019t recorded.`, kind: 'unavailable' },
        };
      }

      const visibleMonths = reply.months.filter(m => m.firstHour <= hour);
      const bandColours = reply.bands.map((band, index) => bandColour(band, index, options.surface));
      const shownBandIndexes = reply.bands.flatMap((band, index) => {
        if (!band.remainder) return [index];
        const active = visibleMonths.some(m => {
          const part = m.parts[index];
          return part && (part.uncertain || (part.raw !== null && part.raw !== '0'));
        });
        return active ? [index] : [];
      });
      const shownBandSet = new Set(shownBandIndexes);

      const slots: ChartMonthSlot[] = [];
      let recordedCount = 0;
      let anyNonZero = false;
      let anyMissing = false;

      for (const bar of visibleMonths) {
        const segments: ChartColumnSegment[] = [];
        const missingParts: { readonly label: string; readonly color: string; readonly reason: string }[] = [];
        let anyKnown = false;
        let knownSum = 0;
        for (let i = 0; i < reply.bands.length; i++) {
          const band = reply.bands[i]!;
          const part = bar.parts[i]!;
          const color = bandColours[i]!;
          if (part.raw !== null && part.value !== null) {
            anyKnown = true;
            knownSum += part.value;
            if (part.raw !== '0') anyNonZero = true;
            if (shownBandSet.has(i)) {
              segments.push({
                id: band.id,
                label: band.label,
                color,
                value: part.value,
                exactText: formatExactIcText(part.raw, band.denominator, reply.decimals, options.digits),
              });
            }
          } else if (!band.remainder) {
            missingParts.push({
              label: band.label,
              color,
              reason: part.reason ?? 'Spending not recorded.',
            });
          }
        }
        const partial = bar.incomplete || (hour >= bar.monthFirstHour && hour < bar.monthLastHour);
        if (!anyKnown) {
          anyMissing = true;
          slots.push({
            year: bar.year,
            month: bar.month,
            firstHour: bar.monthFirstHour,
            lastHour: bar.monthLastHour,
            status: 'not-recorded',
            reason: bar.parts.find(p => p.reason)?.reason ?? 'Spending not recorded.',
            segments: [],
            totalValue: 0,
            totalText: '0 IC',
          });
          continue;
        }
        recordedCount++;
        if (missingParts.length > 0) anyMissing = true;
        const totalValue = bar.totalValue ?? knownSum;
        const totalText = bar.totalRaw !== null
          ? formatExactIcText(bar.totalRaw, bar.totalDenominator, reply.decimals, options.digits)
          : '';
        slots.push({
          year: bar.year,
          month: bar.month,
          firstHour: bar.monthFirstHour,
          lastHour: bar.monthLastHour,
          status: 'recorded',
          ...(partial ? { partial: true, partialToHour: Math.min(hour, bar.lastHour) } : {}),
          segments,
          ...(missingParts.length > 0 ? { missingParts } : {}),
          totalValue,
          totalText,
          ...(bar.totalRaw === null ? { totalMissingReason: bar.totalReason ?? 'Spending not recorded.' } : {}),
          ...(bar.note ? { note: bar.note } : {}),
        });
      }

      if (recordedCount === 0) {
        const reason = slots.find(s => s.reason)?.reason ?? `${spec.name} wasn\u2019t recorded.`;
        return {
          kind: spec.kind,
          measureTitle: options.measureTitle,
          mode: 'columns',
          hour,
          extent: options.extent,
          yAxis: { format: 'compact', min: 0 },
          series: [],
          warnings: reply.warnings,
          empty: { message: reason, kind: 'unavailable' },
        };
      }

      if (!anyNonZero && !anyMissing) {
        return {
          kind: spec.kind,
          measureTitle: options.measureTitle,
          mode: 'columns',
          hour,
          extent: options.extent,
          yAxis: { format: 'compact', min: 0 },
          series: [],
          warnings: reply.warnings,
          empty: { message: `No ${spec.measure} spent at this date.`, kind: 'empty' },
        };
      }

      const legend: ChartLegendEntry[] = [
        ...shownBandIndexes.map(index => ({
          id: reply.bands[index]!.id,
          label: reply.bands[index]!.label,
          color: bandColours[index]!,
        })),
        ...(anyMissing ? [{ id: '__unrecorded__', label: 'Not recorded', hatch: true }] : []),
      ];

      const auditRows = slots
        .filter(s => s.status === 'recorded' && !s.totalMissingReason)
        .map(s => auditRow(`${s.year}-${String(s.month).padStart(2, '0')}`, s.totalText));

      return {
        kind: spec.kind,
        measureTitle: options.measureTitle,
        mode: 'columns',
        hour,
        extent: options.extent,
        yAxis: { format: 'compact', min: 0 },
        series: [],
        columns: {
          foldUnrecorded: false,
          months: slots,
          unit: 'IC',
        },
        ...(legend.length > 0 ? { legend } : {}),
        warnings: reply.warnings,
        auditRows,
      };
    },
  }, 'months');

  const declaration: TimeChartWidgetDeclaration<SpendingMonthsOptions> = {
    family: 'time-chart',
    kind: spec.kind,
    label: `${spec.name} by month`,
    subject: spec.subject,
    emptyCampaign: `Load a campaign to see ${spec.name.toLowerCase()}.`,
    icon: 'sunburst',
    countries: { mode: 'single', choose: `Choose a country to see its ${spec.name.toLowerCase()}.` },
    sources(ctx: FamilyPlanContext): SourcePlan {
      const tag = ctx.countryTag;
      const domain = spec.domain(ctx.options);
      if (!tag) {
        return { owner: `${spec.kind}:months:${ctx.track.source.id}:${ctx.branch.id}`, entries: [] };
      }
      return {
        owner: `${spec.kind}:months:${ctx.track.source.id}:${ctx.track.source.revision}:${ctx.branch.id}:${tag}:${domain}`,
        entries: [{
          name: 'main',
          source: ctx.track.source,
          branch: ctx.branch.id,
          offset: ctx.track.axisOffset,
          read: {
            shape: 'branch-to-hour',
            coverage: { mode: 'keyed', keys: [tag] },
            product: 'production-spending-months',
            followQuery: null,
            params: {
              branchId: ctx.branch.id,
              country: countryFromKey(tag),
              domain,
              repairs: spec.repairs,
            },
            dynamicKeys: ['firstHour', 'lastHour'],
            validate: validateProductionSpendingMonths,
            provenInterval: (data, _requested, hour) => {
              const reply = data as ProductionSpendingMonthsReply;
              return reply.nextChangeHour !== null ? [hour, reply.nextChangeHour - 1] : [hour, Infinity];
            },
          },
        }],
      };
    },
    projection,
    projectionOptions(ctx, { extent, surface }) {
      const domain = spec.domain(ctx.options);
      return {
        domain,
        measureTitle: spec.measureTitle(ctx.options),
        digits: icDecimals(ctx.options, 5),
        surface,
        extent,
      };
    },
  };

  return { projection, declaration };
}

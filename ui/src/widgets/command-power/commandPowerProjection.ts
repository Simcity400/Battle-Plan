import { measureColour } from '../../charts/palette.ts';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import type { BoundedReadView, FamilyPlanContext, SourcePlan } from '../../families/contract.ts';
import type { HolderRun, HourRanges } from '../../families/holders.ts';
import { measureArtHandle } from '../../families/time-chart/measureArt.ts';
import { chartAxis, chartWindow, DEFAULT_POWER_SETTINGS, powerChartSettings, type PowerChartSettings } from '../../families/time-chart/settings.ts';
import { type TimeChartProjection, type TimeChartWidgetDeclaration } from '../../families/time-chart/declaration.ts';
import { registerFamilyProjection } from '../../families/registry.ts';
import type { ChartArtSlot, ChartGap, TimeChartSeriesModel } from '../../families/time-chart/model.ts';
import {
  branchGapsOf,
  countrySeriesLooks,
  exactHoverByCountry,
  memberAt,
  missingSpansOf,
  rankEntry,
  rankedOrder, unnamedLooks,
  spendMarkerEntry,
  spendMarkersOf,
  withinMapOf,
  auditReading,
  formatPowerAmount,
  headReading,
  headValueOf,
  type HeadReading,
  type KnownHeadReading,
} from '../../families/time-chart/seriesHelpers.ts';

export interface CommandPowerOptions {
  readonly tags: readonly string[];
  readonly holders?: Readonly<Record<string, readonly HolderRun[]>> | null;
  readonly membership?: Readonly<Record<string, HourRanges>> | null;
  readonly extent: readonly [number, number];
  readonly gaps: readonly ChartGap[];
  readonly surface: string;
  readonly settings?: PowerChartSettings;
  readonly showCeiling: boolean;
}



export const commandPowerProjection: TimeChartProjection<CommandPowerOptions> = registerFamilyProjection({
  kind: 'command-power',
  family: 'time-chart',
  variant: 'rate-with-ceiling',
  joinMode: 'join',
  measureTitle: 'Command power',
  project(readView: BoundedReadView, options: CommandPowerOptions) {
    const hour = readView.hour;
    const mainMap = readView.series('main');
    const ceilingMap = options.showCeiling ? readView.series('ceiling') : undefined;
    const tags = options.tags;
    const single = tags.length === 1;
    const settings = options.settings ?? DEFAULT_POWER_SETTINGS;
    const format = (reading: KnownHeadReading) => formatPowerAmount(reading, settings.decimals);
    const view = chartWindow(settings, hour, options.extent[0]);

    const headByTag = new Map<string, HeadReading | null>();
    const ceilingByTag = new Map<string, HeadReading | null>();
    const readings: [string, HeadReading | null][] = [];
    for (const tag of tags) {
      if (!memberAt(options.membership, tag, hour)) {
        headByTag.set(tag, null);
        continue;
      }
      const main = headReading(readView, 'main', { tag }, 5);
      readings.push([tag, main]);
      headByTag.set(tag, main);
      if (options.showCeiling) {
        const ceiling = headReading(readView, 'ceiling', { tag }, 5);
        readings.push([`${tag}:ceiling`, ceiling]);
        ceilingByTag.set(tag, ceiling);
      }
    }

    const exactByCountry = exactHoverByCountry(readView, 'hover', mainMap?.get(tags[0] ?? '')?.decimals ?? 5);
    const present = tags.filter(tag => memberAt(options.membership, tag, hour));
    const presentSet = new Set(present);
    const ranking = rankedOrder(readView, present);
    const ordered = [...ranking.order, ...tags.filter(tag => !presentSet.has(tag))];
    const looks = unnamedLooks(countrySeriesLooks(ordered, readView, {
      holders: options.holders,
      from: options.extent[0],
      surface: options.surface,
      withFlag: !single,
      singleColor: measureColour('commandPower', options.surface),
    }), ranking.unnamed, hour);

    const measureIcon: ChartArtSlot | undefined = single
      ? { art: readView.art(measureArtHandle('commandPower')), label: 'Command power' }
      : undefined;

    const series: TimeChartSeriesModel[] = [];
    for (let rank = 0; rank < ordered.length; rank++) {
      const tag = ordered[rank]!;
      const isTail = !single && rank >= 8;
      const bounded = mainMap?.get(tag) ?? null;
      const { name, flag, color, identities } = looks.get(tag)!;
      const mainReading = headByTag.get(tag) ?? null;
      const headValue = headValueOf(mainReading);
      const ceilingBounded = options.showCeiling && !isTail ? (ceilingMap?.get(tag) ?? null) : null;
      const ceilingReading = options.showCeiling && !isTail ? (ceilingByTag.get(tag) ?? null) : null;
      const ceilingHead = headValueOf(ceilingReading);
      const exact = exactByCountry?.get(tag);

      series.push({
        id: tag,
        key: tag,
        name: single ? 'Command power' : name,
        color,
        role: isTail ? 'tail' : 'primary',
        step: false,
        area: single && settings.area,
        bounded,
        headValue,
        headHour: hour,
        exact: exact && memberAt(options.membership, tag, exact.hour) ? exact : null,
        flag,
        ...(single ? {} : { identities }),
        icon: single ? measureIcon : undefined,
        missingSpans: missingSpansOf(bounded),
        endLabel: isTail || !settings.endLabels || !mainReading || !('value' in mainReading)
          ? null
          : {
              ...(single ? { icon: measureIcon } : { flag }),
              name: single ? (options.showCeiling ? 'Command power' : null) : name,
              fullName: single ? 'Command power' : name,
              value: formatPowerAmount(mainReading, settings.decimals),
            },
      });

      if (ceilingBounded || ceilingHead !== null) {
        series.push({
          id: `${tag}:ceiling`,
          key: `${tag}:ceiling`,
          name: single ? 'Maximum' : `${name} max`,
          color,
          ...(single ? {} : { identities: identities.map(run => ({ ...run, name: `${run.name} max` })) }),
          role: 'ceiling',
          step: true,
          dash: [5, 4],
          opacity: 0.55,
          width: 1.4,
          bounded: ceilingBounded,
          headValue: ceilingHead,
          headHour: hour,
          endLabel: !settings.endLabels || !ceilingReading || !('value' in ceilingReading)
            ? null
            : {
                name: single ? 'Maximum' : `${name} max`,
                fullName: single ? 'Maximum command power' : `${name} maximum`,
                value: formatPowerAmount(ceilingReading, settings.decimals),
                valueMuted: true,
              },
        });
      }
    }

    const spent = spendMarkersOf(readView, 'Command power spent', options.extent[0]);

    return {
      kind: 'command-power',
      measureTitle: 'Command power',
      mode: 'lines',
      hour,
      extent: options.extent,
      ...(view ? { view } : {}),
      yAxis: { format: 'power', decimals: settings.decimals, ...chartAxis(settings) },
      series,
      markers: spent.markers.length > 0 ? spent.markers : undefined,
      ...(ranking.warnings.length + spent.warnings.length > 0 ? { warnings: [...ranking.warnings, ...spent.warnings] } : {}),
      gaps: options.gaps.length > 0 ? options.gaps : undefined,
      auditRows: readings.map(([row, reading]) => auditReading(row, reading, format)),
    };
  },
});

export const commandPowerDeclaration: TimeChartWidgetDeclaration<CommandPowerOptions> = {
  family: 'time-chart',
  kind: 'command-power',
  label: 'Command power',
  subject: 'command power',
  emptyCampaign: 'Load a campaign to see command power.',
  icon: 'chart-line',
  countries: { mode: 'multi', content: 'history', allowSlots: true },
  sources(ctx: FamilyPlanContext): SourcePlan {
    const tags = ctx.countries.tags;
    const within = withinMapOf(ctx.countries.membership);
    const countries = tags.map(countryFromKey);
    const showCeiling = ctx.options.ceiling !== false;
    const mainFamily = { kind: 'country-measure', measure: 'commandPower' };
    const ceilingFamily = { kind: 'country-stability', measure: 'maxCommandPower' };
    const entries: SourcePlan['entries'][number][] = [
      {
        name: 'main',
        source: ctx.track.source,
        branch: ctx.branch.id,
        offset: ctx.track.axisOffset,
        read: {
          shape: 'pixel-binned',
          coverage: { mode: 'keyed', keys: tags },
          family: mainFamily,
          tags,
          ...(within ? { within } : {}),
          headQuery: tags.length
            ? {
                product: 'series-state',
                coverage: 'row',
                params: { branchId: ctx.branch.id, family: mainFamily, reads: countries.map(country => ({ country })) },
              }
            : null,
          exactProduct: 'series-state',
        },
      },
    ];
    if (showCeiling) {
      entries.push({
        name: 'ceiling',
        source: ctx.track.source,
        branch: ctx.branch.id,
        offset: ctx.track.axisOffset,
        read: {
          shape: 'pixel-binned',
          coverage: { mode: 'keyed', keys: tags },
          family: ceilingFamily,
          tags,
          ...(within ? { within } : {}),
          headQuery: tags.length
            ? {
                product: 'series-state',
                coverage: 'row',
                params: { branchId: ctx.branch.id, family: ceilingFamily, reads: countries.map(country => ({ country })) },
              }
            : null,
          exactProduct: 'series-state',
        },
      });
    }
    if (tags.length) {
      entries.push({
        name: 'hover',
        source: ctx.track.source,
        branch: ctx.branch.id,
        offset: ctx.track.axisOffset,
        read: {
          shape: 'exact-hover',
          product: 'series-state',
          ...(ctx.countries.membership ? { membership: ctx.countries.membership } : {}),
          params: { branchId: ctx.branch.id, family: mainFamily, reads: countries.map(country => ({ country })) },
        },
      });
    }
    const rank = rankEntry(ctx, mainFamily, tags);
    if (rank) entries.push(rank);
    const spending = spendMarkerEntry(ctx, 'Command');
    if (spending) entries.push(spending);
    return {
      owner: `command-power:${ctx.track.source.id}:${ctx.branch.id}:${tags.join(',')}:${showCeiling ? 1 : 0}:${ctx.options.spendTicks !== false ? 1 : 0}`,
      entries,
    };
  },
  projection: commandPowerProjection,
  projectionOptions(ctx, { extent, surface }) {
    return {
      tags: ctx.countries.tags,
      holders: ctx.countries.holders,
      membership: ctx.countries.membership,
      extent,
      gaps: branchGapsOf(ctx.branch),
      surface,
      settings: powerChartSettings(ctx.options, ctx.following),
      showCeiling: ctx.options.ceiling !== false,
    };
  },
};

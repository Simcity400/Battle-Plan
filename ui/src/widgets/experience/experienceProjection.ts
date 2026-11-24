import { CHART_MEASURE_COLOURS, lineDashes } from '../../charts/palette.ts';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import type { ArtHandle, BoundedReadView, FamilyPlanContext, SourcePlan } from '../../families/contract.ts';
import type { HolderRun } from '../../families/holders.ts';
import {
  registerTimeChart,
  type TimeChartProjection,
  type TimeChartWidgetDeclaration,
} from '../../families/time-chart/declaration.ts';
import type { ChartArtSlot, ChartGap, TimeChartSeriesModel } from '../../families/time-chart/model.ts';
import {
  assignCountrySeriesColours,
  branchGapsOf,
  countrySeriesIdentity,
  missingSpansOf,
  seriesHeadValue,
  withinMapOf,
} from '../../families/time-chart/seriesHelpers.ts';
import { OPTIONS } from './definition.ts';
import {
  BRANCHES,
  BRANCH_LABELS,
  SCALE,
  experienceBranches,
  experienceFamily,
  experienceMeasure,
  type ExperienceBranch,
  type ExperienceMeasure,
} from './experienceData.ts';

export interface ExperienceOptions {
  readonly tags: readonly string[];
  readonly holders?: Readonly<Record<string, readonly HolderRun[]>> | null;
  readonly extent: readonly [number, number];
  readonly gaps: readonly ChartGap[];
  readonly surface: string;
  readonly measure: ExperienceMeasure;
  readonly branches: readonly ExperienceBranch[];
  readonly yMin: number | null;
  readonly yMax: number | null;
  readonly log: boolean;
}

const BRANCH_COLOURS: Readonly<Record<ExperienceBranch, string>> = {
  army: CHART_MEASURE_COLOURS.army,
  navy: CHART_MEASURE_COLOURS.navy,
  air: CHART_MEASURE_COLOURS.air,
};

const BRANCH_ICON_HANDLES: Readonly<Record<ExperienceBranch, ArtHandle>> = {
  army: { kind: 'sprite', key: 'GFX_army_experience', frame: 0 },
  navy: { kind: 'sprite', key: 'GFX_navy_experience', frame: 0 },
  air: { kind: 'sprite', key: 'GFX_air_experience', frame: 0 },
};

const DISPLAY_ORDER: readonly ExperienceBranch[] = ['air', 'navy', 'army'];

function formatXpValue(points: number, signed: boolean): string {
  const abs = Math.abs(points);
  const rounded = Math.round(abs * 100) / 100;
  const text = rounded.toLocaleString('en-US', { maximumFractionDigits: 2 });
  if (points < 0) return `\u2212${text}`;
  return signed && points > 0 ? `+${text}` : text;
}

export const experienceProjection: TimeChartProjection<ExperienceOptions> = registerTimeChart({
  kind: 'experience',
  family: 'series',
  variant: 'line',
  joinMode: 'join',
  measureTitle: 'Experience',
  project(readView: BoundedReadView, options: ExperienceOptions) {
    const hour = readView.hour;
    const tags = options.tags;
    const single = tags.length === 1;
    const signed = options.measure !== 'banked';
    const scale = 1 / SCALE;

    const orderedBranches = DISPLAY_ORDER.filter(b => options.branches.includes(b));
    const headByBranchTag = new Map<string, number | null>();
    for (const branch of orderedBranches) {
      const seriesMap = readView.series(branch);
      const answer = readView.answer(branch);
      for (let i = 0; i < tags.length; i++) {
        const tag = tags[i]!;
        headByBranchTag.set(`${branch}:${tag}`, seriesHeadValue(seriesMap?.get(tag), answer, i, 0, scale));
      }
    }

    const totalByTag = new Map<string, number>();
    for (const tag of tags) {
      let sum = 0;
      for (const b of orderedBranches) sum += headByBranchTag.get(`${b}:${tag}`) ?? 0;
      totalByTag.set(tag, sum);
    }
    const orderedTags = [...tags].sort((a, b) => (totalByTag.get(b) ?? -Infinity) - (totalByTag.get(a) ?? -Infinity));
    const colourMap = single
      ? new Map<string, string>()
      : assignCountrySeriesColours(orderedTags, options.holders, readView, hour, options.surface);

    const series: TimeChartSeriesModel[] = [];
    for (let rank = 0; rank < orderedTags.length; rank++) {
      const tag = orderedTags[rank]!;
      const isTail = !single && rank >= 8;
      const { name, flag } = countrySeriesIdentity(tag, options.holders, readView, hour, !single);
      for (const branch of orderedBranches) {
        const bounded = readView.series(branch)?.get(tag) ?? null;
        const headValue = headByBranchTag.get(`${branch}:${tag}`) ?? null;
        const icon: ChartArtSlot | undefined = single
          ? { art: readView.art(BRANCH_ICON_HANDLES[branch]), label: BRANCH_LABELS[branch] }
          : undefined;
        const color = isTail
          ? 'rgba(245, 245, 245, 0.22)'
          : single
            ? BRANCH_COLOURS[branch]
            : (colourMap.get(tag) ?? BRANCH_COLOURS[branch]);
        const branchIdx = BRANCHES.indexOf(branch);
        const dash = !single && branchIdx > 0 ? lineDashes(branchIdx).primary : undefined;
        const lineName = single ? BRANCH_LABELS[branch] : `${name} · ${BRANCH_LABELS[branch]}`;
        series.push({
          id: single ? branch : `${tag}:${branch}`,
          key: single ? branch : `${tag}:${branch}`,
          name: lineName,
          color,
          role: isTail ? 'tail' : 'primary',
          step: false,
          dash,
          area: false,
          negRed: signed,
          bounded,
          valueScale: scale,
          headValue,
          headHour: hour,
          flag,
          icon,
          missingSpans: missingSpansOf(bounded),
          rank,
          endLabel: isTail || headValue === null
            ? null
            : {
                ...(single ? { icon } : { flag }),
                name: lineName,
                fullName: lineName,
                value: formatXpValue(headValue, signed),
              },
        });
      }
    }

    return {
      kind: 'experience',
      measureTitle: 'Experience',
      mode: 'lines',
      hour,
      extent: options.extent,
      yAxis: {
        format: 'number',
        min: options.yMin,
        max: options.yMax,
        log: options.log,
      },
      series,
      gaps: options.gaps.length > 0 ? options.gaps : undefined,
      empty: orderedBranches.length === 0
        ? { message: 'Turn on an experience branch in the widget settings.' }
        : null,
      auditRows: series.slice(0, 8).flatMap(s =>
        s.endLabel ? [{ row: s.id, key: s.id, value: s.endLabel.value }] : [],
      ),
    };
  },
});

export const experienceDeclaration: TimeChartWidgetDeclaration<ExperienceOptions> = {
  family: 'series',
  kind: 'experience',
  label: 'Experience',
  subject: 'experience',
  emptyCampaign: 'Load a campaign to see experience.',
  icon: 'chart-line',
  optionFields: OPTIONS,
  countries: { mode: 'multi', labels: true, colours: true, allowSlots: true },
  sources(ctx: FamilyPlanContext): SourcePlan {
    const tags = ctx.countries.tags;
    const within = withinMapOf(ctx.countries.membership);
    const countries = tags.map(countryFromKey);
    const measure = experienceMeasure(ctx.options);
    const branches = experienceBranches(ctx.options);
    const entries: SourcePlan['entries'][number][] = branches.map(branch => {
      const family = experienceFamily(measure, branch);
      return {
        name: branch,
        source: ctx.track.source,
        branch: ctx.branch.id,
        offset: ctx.track.axisOffset,
        read: {
          shape: 'pixel-binned',
          coverage: { mode: 'keyed', keys: tags },
          family,
          tags,
          ...(within ? { within } : {}),
          marks: false,
          headQuery: tags.length
            ? {
                product: 'series-state',
                params: { branchId: ctx.branch.id, family, reads: countries.map(country => ({ country })) },
              }
            : null,
          exactProduct: 'series-state',
        },
      };
    });
    if (tags.length === 1) {
      entries.push({
        name: 'hover',
        source: ctx.track.source,
        branch: ctx.branch.id,
        offset: ctx.track.axisOffset,
        read: {
          shape: 'exact-hover',
          product: 'country-experience',
          params: { branchId: ctx.branch.id, country: countries[0]! },
        },
      });
    }
    return {
      owner: `experience:${ctx.track.source.id}:${ctx.branch.id}:${tags.join(',')}:${measure}:${branches.join(',')}`,
      entries,
    };
  },
  projection: experienceProjection,
  projectionOptions(ctx, { extent, surface }) {
    return {
      tags: ctx.countries.tags,
      holders: ctx.countries.holders,
      extent,
      gaps: branchGapsOf(ctx.branch),
      surface,
      measure: experienceMeasure(ctx.options),
      branches: experienceBranches(ctx.options),
      yMin: typeof ctx.options.yMin === 'number' ? ctx.options.yMin : null,
      yMax: typeof ctx.options.yMax === 'number' ? ctx.options.yMax : null,
      log: ctx.options.log === true,
    };
  },
};

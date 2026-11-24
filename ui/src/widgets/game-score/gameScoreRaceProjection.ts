import { liveKey } from '../../charts/live.ts';
import { colour, DEFAULT_CARD_SURFACE } from '../../charts/palette.ts';
import { compareTags } from '../../dashboard/tagOrder.ts';
import type { BarsWidgetDeclaration } from '../../families/bars/declaration.ts';
import { countryRowPresentation, type BarsDataRow, type BarsModel, type BarsProjection, type BarsSegment } from '../../families/bars/model.ts';
import { registerBarsProjection } from '../../families/bars/registry.ts';
import type { BoundedReadView, FamilyPlanContext, SourcePlan } from '../../families/contract.ts';
import { holderTagAt, inHourRanges, nextHoldersChange, nextMembershipChange, nextRangesChange, type HolderRun, type HourRanges } from '../../families/holders.ts';
import type { TooltipListItem } from '../../families/host/tooltips.ts';
import { knownValue, notRecorded, valueFromLive, zeroValue, type Value } from '../../families/value.ts';
import { formatValueState } from '../../families/valueFormat.ts';
import type { GameScoreRow } from '../../generated/dashboard';
import { OPTIONS } from './definition.ts';
import { SCORE_LABELS, scorePart, validateGameScore, type ScorePart } from './gameScoreData.ts';

const PART_SURFACE = DEFAULT_CARD_SURFACE;
const SPLIT_PARTS = ['army', 'navy', 'air'] as const;

export interface GameScoreRaceOptions {
  readonly all: boolean;
  readonly chosenTags: readonly string[];
  readonly holders: Readonly<Record<string, readonly HolderRun[]>> | null;
  readonly membership: Readonly<Record<string, HourRanges>> | null;
  readonly unobserved: HourRanges;
  readonly top: number | null;
  readonly by: ScorePart;
  readonly split: boolean;
}

function numericOf(value: Value): number | null {
  if (value.kind === 'known' || value.kind === 'partial') return Number(value.value);
  if (value.kind === 'zero') return 0;
  return null;
}

function partValue(
  row: GameScoreRow,
  part: ScorePart,
  live: ReturnType<BoundedReadView['live']>,
): Value {
  return row[part].missing !== null ? notRecorded(row[part].missing) : valueFromLive(live, liveKey(row.tag, part));
}

function buildSplitSegments(
  row: GameScoreRow,
  live: ReturnType<BoundedReadView['live']>,
  options: GameScoreRaceOptions,
): { readonly segments?: readonly BarsSegment[]; readonly tooltipItems: TooltipListItem[] } {
  if (!options.split || options.by !== 'total') return { tooltipItems: [] };
  const tooltipItems: TooltipListItem[] = [];
  const rawParts: { readonly part: typeof SPLIT_PARTS[number]; readonly label: string; readonly color: string; readonly value: Value; readonly amount: number }[] = [];
  let sum = 0;
  for (let i = 0; i < SPLIT_PARTS.length; i++) {
    const part = SPLIT_PARTS[i]!;
    const label = SCORE_LABELS[part];
    const color = colour(PART_SURFACE, i);
    const val = partValue(row, part, live);
    const amount = numericOf(val);
    if (amount === null) {
      tooltipItems.push({ label, swatch: color, missingReason: val.kind === 'notRecorded' ? val.reason : 'Not recorded' });
      continue;
    }
    const clamped = Math.max(0, amount);
    sum += clamped;
    const fmt = formatValueState(val, { decimals: 0 });
    tooltipItems.push({ label, swatch: color, value: fmt.kind === 'known' || fmt.kind === 'zero' || fmt.kind === 'partial' ? fmt.text : fmt.label });
    rawParts.push({ part, label, color, value: clamped === 0 ? zeroValue() : knownValue(clamped), amount: clamped });
  }
  if (sum <= 0 || !rawParts.length) return { tooltipItems };
  const segments: BarsSegment[] = [];
  for (const item of rawParts) {
    if (item.amount <= 0) continue;
    segments.push({
      key: item.part,
      label: item.label,
      share: item.amount / sum,
      value: item.value,
      format: { decimals: 0 },
      color: item.color,
    });
  }
  return { ...(segments.length ? { segments } : {}), tooltipItems };
}

function makeRow(
  readView: BoundedReadView,
  row: GameScoreRow,
  live: ReturnType<BoundedReadView['live']>,
  options: GameScoreRaceOptions,
): BarsDataRow {
  const pres = countryRowPresentation(readView, row.tag);
  const value = partValue(row, options.by, live);
  const numeric = numericOf(value);
  const parts = numeric !== null ? buildSplitSegments(row, live, options) : { tooltipItems: [] };
  const fmt = formatValueState(value, { decimals: 0 });
  const label = SCORE_LABELS[options.by];
  const primaryItem: TooltipListItem = fmt.kind === 'notRecorded'
    ? { label, missingReason: fmt.reason }
    : fmt.kind === 'notApplicable'
      ? { label, value: '\u2014' }
      : { label, value: fmt.exact ?? fmt.text };
  return {
    kind: 'data',
    key: row.tag,
    subject: row.tag,
    label: pres.label,
    labelMissing: pres.labelMissing,
    media: pres.media,
    ...(pres.color ? { color: pres.color } : {}),
    value: {
      value,
      format: { decimals: 0 },
      label,
    },
    numeric,
    ...(parts.segments ? { segments: parts.segments } : {}),
    tooltip: {
      title: pres.labelMissing ? 'Name not recorded' : pres.label,
      hour: readView.hour,
      items: [primaryItem, ...parts.tooltipItems],
    },
  };
}

function compareUnrankedRows(a: BarsDataRow, b: BarsDataRow): number {
  const aMissing = a.numeric === null || a.numeric === undefined;
  const bMissing = b.numeric === null || b.numeric === undefined;
  if (aMissing !== bMissing) return aMissing ? 1 : -1;
  if (!aMissing && !bMissing && a.numeric !== b.numeric) return b.numeric! - a.numeric!;
  return compareTags(a.subject ?? a.key, b.subject ?? b.key);
}

export const gameScoreRaceProjection: BarsProjection<GameScoreRaceOptions> = registerBarsProjection({
  kind: 'game-score',
  family: 'bars',
  nextChange(readView, options) {
    const hour = readView.hour;
    return Math.min(
      nextHoldersChange(options.holders, hour),
      nextRangesChange(options.unobserved, hour),
      nextMembershipChange(options.membership, hour),
    );
  },
  project(readView: BoundedReadView, options: GameScoreRaceOptions): BarsModel {
    const hour = readView.hour;
    const baseModel = {
      form: 'ordered' as const,
      hour,
      animateOrder: true,
      showAxis: true,
      scaleUnit: SCORE_LABELS[options.by],
      scaleFormat: { decimals: 0 },
    };

    if (options.top == null && options.unobserved.some(([first, last]) => first <= hour && hour <= last)) {
      return {
        ...baseModel,
        rows: [],
        empty: 'Group membership is unavailable at this time.',
        emptyKind: 'unavailable',
      };
    }

    const refused = readView.refused('main');
    if (refused) {
      return { ...baseModel, rows: [], empty: refused, emptyKind: 'unavailable' };
    }

    const panel = readView.panel('main');
    if (panel === undefined) {
      return { ...baseModel, rows: [] };
    }

    const reply = validateGameScore(panel);
    if (reply.missing !== null) {
      return { ...baseModel, rows: [], empty: reply.missing, emptyKind: 'unavailable', warnings: reply.warnings };
    }

    const live = readView.live('main');
    let selected: GameScoreRow[];
    if (options.top != null) {
      if (!reply.top) {
        return { ...baseModel, rows: [], empty: 'The top countries were not answered for this reading.', emptyKind: 'unavailable', warnings: reply.warnings };
      }
      if (reply.top.missing !== null) {
        return { ...baseModel, rows: [], empty: reply.top.missing, emptyKind: 'unavailable', warnings: reply.warnings };
      }
      const byTag = new Map(reply.rows.map(row => [row.tag, row]));
      selected = reply.top.tags.flatMap(tag => byTag.get(tag) ?? []);
    } else if (options.all) {
      selected = reply.rows;
    } else if (options.membership) {
      selected = reply.rows.filter(row => inHourRanges(options.membership![row.tag], hour));
    } else if (options.holders) {
      const activeTags = new Set<string>();
      for (const runs of Object.values(options.holders)) {
        const tag = holderTagAt(runs, hour);
        if (tag !== null) activeTags.add(tag);
      }
      selected = reply.rows.filter(row => activeTags.has(row.tag));
    } else {
      const chosen = new Set(options.chosenTags);
      selected = reply.rows.filter(row => chosen.has(row.tag));
    }

    const rows = selected.map(row => makeRow(readView, row, live, options));
    if (options.top == null) rows.sort(compareUnrankedRows);

    return {
      ...baseModel,
      rows,
      warnings: reply.warnings,
      ...(!rows.length ? { empty: 'No country’s scores recorded at this time.', emptyKind: 'unavailable' as const } : {}),
    };
  },
}, 'race');

function gameScoreRaceSources(ctx: FamilyPlanContext): SourcePlan {
  const top = ctx.countries.top;
  const by = scorePart(ctx.options.topMeasure);
  const asked = top != null ? { branchId: ctx.branch.id, top: { count: top, by } } : { branchId: ctx.branch.id };
  return {
    owner: `game-score:race:${ctx.track.source.id}:${ctx.track.source.revision}:${ctx.branch.id}:${by}`,
    entries: [{
      name: 'main',
      source: ctx.track.source,
      branch: ctx.branch.id,
      offset: ctx.track.axisOffset,
      read: {
        shape: 'point',
        channel: 'panel',
        coverage: top != null ? { mode: 'population', form: 'top', limit: top } : { mode: 'population', form: 'all' },
        product: 'game-score',
        followQuery: { panel: 'game-score', ...asked },
        params: asked,
        validate: validateGameScore,
      },
    }],
  };
}

export const gameScoreRaceDeclaration: BarsWidgetDeclaration<GameScoreRaceOptions> = {
  family: 'bars',
  kind: 'game-score',
  label: 'Game score',
  subject: 'game scores',
  emptyCampaign: 'Load a campaign to see its game scores.',
  icon: 'grid',
  optionFields: OPTIONS,
  countries: {
    mode: 'multi',
    labels: true,
    colours: true,
    allowSlots: true,
    allTagsForLabels: true,
    requireGroupsWhen: widget => widget.countries.kind === 'group' && widget.countries.group.type !== 'top',
  },
  sources: gameScoreRaceSources,
  projection: gameScoreRaceProjection,
  projectionOptions: ctx => ({
    all: ctx.countries.all,
    chosenTags: ctx.countries.tags,
    holders: ctx.countries.top == null ? ctx.countries.holders : null,
    membership: ctx.countries.top == null ? ctx.countries.membership : null,
    unobserved: ctx.countries.top == null ? ctx.countries.unobserved : [],
    top: ctx.countries.top,
    by: scorePart(ctx.options.topMeasure),
    split: ctx.options.split !== false,
  }),
  jumpToDate: true,
  keyByBranch: true,
};

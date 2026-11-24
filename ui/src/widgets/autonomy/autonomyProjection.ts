import { formatDay } from '../../dashboard/calendar.ts';
import { clipSpansToHour, paletteAt, resolveCountryChip } from '../../families/timeline/helpers.ts';
import type { TimelineMeterRow, TimelineModel, TimelineProjection, TimelineSeriesLine } from '../../families/timeline/model.ts';
import { registerTimelineProjection } from '../../families/timeline/registry.ts';
import type { SubjectReply } from '../../generated/dashboard';
import type { MeasureStates } from '../country-measure/measureSeriesData';
import { autonomyLanes, formatScore, levelAt, levelName, levelOrder, runAt } from './autonomyData.ts';

export type AutonomyViewMode = 'meters' | 'history';

export interface AutonomySeriesSample {
  readonly tag: string;
  readonly points: readonly (readonly [number, number | null])[];
  readonly current: number | null;
}

export interface AutonomyProjectionOptions {
  readonly view: AutonomyViewMode;
  readonly bySubject: boolean;
  readonly selectedTags: readonly string[] | null;
  readonly branchFirst: number;
  readonly surface: string;
  readonly seriesSamples?: Readonly<Record<string, AutonomySeriesSample>>;
}

function progressAtHour(sample: AutonomySeriesSample | undefined, hour: number): number | null {
  if (!sample) return null;
  let last: number | null = null;
  for (const [h, v] of sample.points) {
    if (h > hour) break;
    last = v;
  }
  return last ?? sample.current;
}

function sparklineBefore(sample: AutonomySeriesSample | undefined, hour: number, maxPoints = 18): (number | null)[] {
  if (!sample || sample.points.length < 2) return [];
  const clipped = sample.points.filter(([h]) => h <= hour);
  if (clipped.length < 2) return [];
  if (clipped.length <= maxPoints) return clipped.map(([, v]) => v);
  const out: (number | null)[] = [];
  const step = (clipped.length - 1) / (maxPoints - 1);
  for (let i = 0; i < maxPoints; i++) {
    out.push(clipped[Math.round(i * step)]![1]);
  }
  return out;
}

export const autonomyProjection: TimelineProjection<AutonomyProjectionOptions> = registerTimelineProjection({
  kind: 'autonomy',

  project(readView, opt): TimelineModel {
    const hour = readView.hour;
    const reply = readView.panel<SubjectReply>('main');
    const extent: [number, number] = [opt.branchFirst, Math.max(opt.branchFirst + 1, hour)];
    if (!reply) {
      return {
        mode: opt.view === 'history' ? 'series' : 'meters',
        hour,
        extent,
        playhead: hour,
        empty: 'Loading autonomy\u2026',
      };
    }

    const chosen = opt.selectedTags ? new Set(opt.selectedTags) : null;
    const lanesData = autonomyLanes(reply, chosen, opt.bySubject);
    if (!lanesData.length) {
      return {
        mode: opt.view === 'history' ? 'series' : 'meters',
        hour,
        extent,
        playhead: hour,
        empty: reply.runs.length
          ? 'None of the chosen countries has been a subject.'
          : reply.warnings.length
            ? 'Subject relations are unavailable.'
            : 'No subjects recorded.',
        warnings: reply.warnings,
      };
    }

    const order = levelOrder(reply.levels);
    const levelColor = (token: string | null | undefined) => {
      const idx = token != null ? order.get(token)?.index : undefined;
      return idx !== undefined ? paletteAt(opt.surface, idx) : '#8c95a6';
    };

    if (opt.view === 'history') {
      const lines: TimelineSeriesLine[] = [];
      lanesData.forEach((lane, idx) => {
        if ((lane.runs[0]?.firstHour ?? Infinity) > hour) return;
        const chip = resolveCountryChip(readView, lane.tag, hour);
        const sample = opt.seriesSamples?.[lane.tag];
        const pts: [number, number | null][] = [];
        if (sample) {
          for (const [h, v] of sample.points) {
            if (h <= hour) pts.push([h, v]);
          }
        }
        const cur = progressAtHour(sample, hour);
        const endLabel = cur !== null ? `${chip.name} ${Math.round(cur * 100)}%` : chip.name;
        lines.push({
          key: lane.tag,
          label: chip.name,
          color: chip.color ?? paletteAt(opt.surface, idx),
          points: pts,
          endLabel,
          flag: chip.flag,
          flagTag: lane.tag,
          auditValue: `${chip.name}|${cur !== null ? `${Math.round(cur * 100)}%` : 'Not recorded'}`,
          meta: { tag: lane.tag },
        });
      });

      if (!lines.length) {
        return {
          mode: 'series',
          hour,
          extent,
          playhead: hour,
          empty: 'No subjects at this date.',
          warnings: reply.warnings,
        };
      }

      return {
        mode: 'series',
        hour,
        extent,
        playhead: hour,
        series: {
          lines,
          yMin: 0,
          yMax: 1,
          yFormat: 'percent',
          thresholdLines: [0, 1],
          legend: true,
        },
        unobserved: clipSpansToHour(reply.unobserved, hour),
        warnings: reply.warnings,
      };
    }

    const activeSubjects = lanesData
      .map(lane => {
        const run = runAt(lane, hour);
        if (!run) return null;
        const level = levelAt(lane, hour) ?? null;
        const chip = resolveCountryChip(readView, lane.tag, hour);
        const overlordChip = resolveCountryChip(readView, run.overlord, hour);
        const sample = opt.seriesSamples?.[lane.tag];
        const fraction = progressAtHour(sample, hour);
        const dist = fraction !== null ? Math.min(fraction, 1 - fraction) : Infinity;
        return { lane, run, level, chip, overlordChip, sample, fraction, dist };
      })
      .filter((x): x is NonNullable<typeof x> => x !== null);

    if (!activeSubjects.length) {
      return {
        mode: 'meters',
        hour,
        extent,
        playhead: hour,
        meters: [],
        empty: 'No subjects at this date.',
        warnings: reply.warnings,
      };
    }

    activeSubjects.sort((a, b) => a.dist - b.dist || a.chip.name.localeCompare(b.chip.name));

    const meters: TimelineMeterRow[] = activeSubjects.map(item => {
      const lvlName = levelName(reply.levels, item.level);
      const fill = levelColor(item.level);
      const pctText = item.fraction !== null ? `${Math.round(item.fraction * 100)}%` : 'Not recorded';
      return {
        key: item.lane.tag,
        label: item.chip.name,
        subtitle: lvlName,
        subtitleColor: fill,
        flag: item.chip.flag,
        flagTag: item.lane.tag,
        fraction: item.fraction,
        fill,
        valueText: pctText,
        missing: item.fraction === null,
        thresholdTicks: true,
        sparkline: sparklineBefore(item.sample, hour),
        auditValue: `${item.chip.name}|${lvlName}|${pctText}`,
        meta: {
          tag: item.lane.tag,
          level: item.level,
          levelName: lvlName,
          overlordName: item.overlordChip.name,
          fill,
          fraction: item.fraction,
        },
      };
    });

    return {
      mode: 'meters',
      hour,
      extent,
      playhead: hour,
      meters,
      warnings: reply.warnings,
    };
  },

  tooltip(model, target, hour, readView) {
    const reply = readView.panel<SubjectReply>('main');
    const tag = target.key;
    const meter = model.meters?.find(m => m.key === tag);
    const meta = meter?.meta as { level?: string | null; levelName?: string; overlordName?: string; fill?: string; fraction?: number | null } | undefined;
    const scores = readView.hover<MeasureStates>('score', hour)?.data;
    const lowers = readView.hover<MeasureStates>('lower', hour)?.data;
    const uppers = readView.hover<MeasureStates>('upper', hour)?.data;
    const readScore = (states: MeasureStates | undefined) => {
      const entry = states?.states.find(s => String(s.country) === tag);
      return entry ? formatScore(entry.run?.valueRaw, states?.decimals, entry.missing) : 'Loading\u2026';
    };
    const order = reply ? levelOrder(reply.levels) : null;
    const place = meta?.level != null && order ? order.get(meta.level) : undefined;
    const chip = resolveCountryChip(readView, tag, hour);
    return {
      title: `${chip.name} \u00b7 ${formatDay(hour)}`,
      hour,
      items: [
        ...(meta?.levelName ? [{ label: 'Subject level', value: meta.levelName, swatch: meta.fill }] : []),
        ...(meta?.overlordName ? [{ label: 'Overlord', value: meta.overlordName }] : []),
        {
          label: place?.highest ? 'Progress within level' : 'Progress to next level',
          value: meta?.fraction != null ? `${(meta.fraction * 100).toFixed(1)}%` : 'Not recorded',
        },
        ...(scores ? [{ label: 'Autonomy score', value: readScore(scores), divider: true }] : []),
        ...(lowers ? [{ label: 'Moves down below', value: `${readScore(lowers)}${place?.lowest ? ' \u00b7 no lower level' : ''}` }] : []),
        ...(uppers ? [{ label: 'Moves up at', value: `${readScore(uppers)}${place?.highest ? ' \u00b7 no higher level' : ''}` }] : []),
      ],
    };
  },
});

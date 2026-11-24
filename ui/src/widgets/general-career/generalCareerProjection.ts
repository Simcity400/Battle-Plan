import { firstHourOfMonth, formatDay, gameDate, monthIndex } from '../../dashboard/calendar.ts';
import { contentTitle } from '../../dashboard/contentTitles.ts';
import { liveKey } from '../../charts/live.ts';
import { nextBoundaryAfter, paletteAt, resolveCountryChip, urlArtHandle } from '../../families/timeline/helpers.ts';
import type {
  TimelineDivergingBin,
  TimelineHeaderCard,
  TimelineLaneModel,
  TimelineMeterRow,
  TimelineModel,
  TimelineProjection,
  TimelineSpanMarker,
  TimelineSpanModel,
} from '../../families/timeline/model.ts';
import { registerTimelineProjection } from '../../families/timeline/registry.ts';
import type { LeaderCareerReply, LeaderHourReply } from '../../generated/dashboard';
import { careerTotals, leaderName, runAt } from '../generals/generalsData.ts';

export type GeneralCareerViewMode = 'timeline' | 'dossier' | 'record';

export interface GeneralCareerProjectionOptions {
  readonly view: GeneralCareerViewMode;
  readonly reference: string;
  readonly branchFirst: number;
  readonly surface: string;
  readonly portraitUrl?: string | null;
  readonly portraitMissing?: boolean;
}

function collectBoundaries(career: LeaderCareerReply): number[] {
  const set = new Set<number>();
  for (const r of career.runs) { set.add(r.firstHour); set.add(r.lastHour + 1); }
  for (const c of career.commands) { set.add(c.firstHour); set.add(c.lastHour + 1); }
  for (const [h] of career.traitGains) set.add(h);
  for (const b of career.battles) {
    set.add(Math.max(b.start, b.read));
    if (b.resultHour !== null) set.add(b.resultHour);
  }
  return [...set].sort((a, b) => a - b);
}

function parseFixed5(raw: string | number | bigint | undefined | null): number | null {
  if (raw === undefined || raw === null) return null;
  const n = Number(raw);
  return Number.isFinite(n) ? n / 100_000 : null;
}

function formatXpPair(current: number | null, required: number | null): { readonly text: string; readonly fraction: number | null } {
  if (current === null) return { text: 'Not recorded', fraction: null };
  const curTxt = Math.round(current).toLocaleString('en-US');
  if (required === null || required <= 0) return { text: curTxt, fraction: null };
  const reqTxt = Math.round(required).toLocaleString('en-US');
  return { text: `${curTxt} / ${reqTxt}`, fraction: Math.max(0, Math.min(1, current / required)) };
}

export const generalCareerProjection: TimelineProjection<GeneralCareerProjectionOptions> = registerTimelineProjection({
  kind: 'general-career',

  nextChange(readView) {
    const career = readView.panel<LeaderCareerReply>('career');
    if (!career) return Infinity;
    return nextBoundaryAfter(collectBoundaries(career), readView.hour);
  },

  project(readView, opt): TimelineModel {
    const hour = readView.hour;
    const career = readView.panel<LeaderCareerReply>('career');
    const leaderHour = readView.panel<LeaderHourReply>('hour');
    const live = readView.live('hour');
    const extent: [number, number] = [opt.branchFirst, Math.max(opt.branchFirst + 1, hour)];

    if (!career) {
      return {
        mode: opt.view === 'dossier' ? 'meters' : opt.view === 'record' ? 'series' : 'lanes',
        hour,
        extent,
        playhead: hour,
        empty: 'Loading leader career\u2026',
      };
    }
    if (career.missing) {
      return {
        mode: opt.view === 'dossier' ? 'meters' : opt.view === 'record' ? 'series' : 'lanes',
        hour,
        extent,
        playhead: hour,
        empty: 'Career unavailable for this leader.',
        emptyDetail: career.missing,
        emptyKind: 'unavailable',
        warnings: career.warnings,
      };
    }

    const currentRun = runAt(career, hour) ?? career.runs.filter(r => r.firstHour <= hour).at(-1) ?? null;
    const role = leaderHour?.role ?? currentRun?.role ?? null;
    const ownerTag = role?.owner ?? null;
    const countryChip = resolveCountryChip(readView, ownerTag, hour);
    const skill = leaderHour?.skill ?? currentRun?.skill ?? null;
    const totals = leaderHour && !leaderHour.missing
      ? {
          fought: leaderHour.fought,
          won: leaderHour.won,
          lost: leaderHour.lost,
          unresolved: Math.max(0, leaderHour.fought - leaderHour.won - leaderHour.lost),
          winRate: leaderHour.won + leaderHour.lost > 0 ? (leaderHour.won * 100) / (leaderHour.won + leaderHour.lost) : null,
          foughtPartial: leaderHour.battlesPartialFrom !== null && leaderHour.battlesPartialFrom <= hour,
        }
      : careerTotals(career, hour);

    const rankText = role?.rank ?? (role ? `Rank ${role.category}` : 'Commander');
    const skillText = skill !== null ? `Skill ${skill}` : 'Skill not recorded';
    const headerCard: TimelineHeaderCard = {
      portrait: urlArtHandle(opt.portraitUrl),
      portraitMissing: opt.portraitMissing || !opt.portraitUrl,
      name: leaderName(leaderHour?.name ? leaderHour : career),
      subtitle: `${countryChip.name} \u00b7 ${rankText} \u00b7 ${skillText}`,
      flag: countryChip.flag,
      flagTag: ownerTag ?? undefined,
      stats: [
        { label: 'Won', value: String(totals.won), tone: 'positive' },
        { label: 'Lost', value: String(totals.lost), tone: 'negative' },
        { label: 'Unresolved', value: String(totals.unresolved), tone: 'muted' },
        { label: 'Fought', value: totals.foughtPartial ? `\u2265${totals.fought}` : String(totals.fought) },
        { label: 'Win rate', value: totals.winRate !== null ? `${Math.round(totals.winRate)}%` : '\u2014' },
      ],
    };

    const trainingMeters: TimelineMeterRow[] = [];
    const training = leaderHour?.progress?.training ?? [];
    for (const t of training) {
      const curXp = parseFixed5(live?.raw?.(liveKey(t.key, 'experience')) ?? live?.get(liveKey(t.key, 'experience')));
      const reqXp = parseFixed5(t.requiredRaw);
      const formatted = formatXpPair(curXp, reqXp);
      trainingMeters.push({
        key: `train:${t.key}`,
        label: t.name ?? t.token ?? 'Trait',
        fraction: formatted.fraction,
        fill: paletteAt(opt.surface, 0),
        valueText: formatted.text,
        missing: curXp === null,
      });
    }

    if (opt.view === 'dossier') {
      const meters: TimelineMeterRow[] = [];
      const totalFought = Math.max(1, totals.fought);
      meters.push({
        key: 'dossier:record',
        sectionHeader: 'Combat record',
        label: 'Battles',
        subtitle: `${totals.won} won \u00b7 ${totals.lost} lost \u00b7 ${totals.unresolved} unresolved`,
        fraction: totals.fought > 0 ? totals.won / totalFought : 0,
        fill: '#3fb950',
        splitSegments: totals.fought > 0
          ? [
              { fraction: totals.won / totalFought, fill: '#3fb950' },
              { fraction: totals.lost / totalFought, fill: '#e5534b' },
              { fraction: totals.unresolved / totalFought, fill: '#6e7681' },
            ]
          : undefined,
        valueText: totals.winRate !== null ? `${Math.round(totals.winRate)}%` : '\u2014',
      });

      const cmdXp = parseFixed5(live?.raw?.(liveKey(opt.reference, 'experience')) ?? live?.get(liveKey(opt.reference, 'experience')));
      const nextReq = parseFixed5(leaderHour?.progress?.nextRequiredRaw);
      const xpFmt = formatXpPair(cmdXp, nextReq);
      meters.push({
        key: 'dossier:xp',
        sectionHeader: 'Experience',
        label: 'Commander XP',
        subtitle: skillText,
        fraction: xpFmt.fraction,
        fill: paletteAt(opt.surface, 0),
        valueText: xpFmt.text,
        missing: cmdXp === null,
      });

      const heldTraits = career.traitGains
        .filter(([h]) => h <= hour)
        .map(([h, idx, cls]) => {
          const trait = career.traits[idx];
          return {
            key: `trait:${idx}:${h}`,
            label: contentTitle(trait?.name, trait?.token ?? String(idx), 'Trait name unavailable'),
            detail: `${cls === 0 ? 'Held before' : cls === 1 ? 'Gained' : 'First observed'} ${formatDay(h)}`,
            icon: urlArtHandle(trait?.icon),
            iconMissing: !trait?.icon,
          };
        });
      if (heldTraits.length > 0) {
        meters.push({
          key: 'dossier:traits',
          sectionHeader: `Traits \u00b7 ${heldTraits.length}`,
          label: 'Traits',
          fraction: null,
          fill: paletteAt(opt.surface, 0),
          valueText: String(heldTraits.length),
          plainRows: heldTraits,
        });
      }

      if (trainingMeters.length > 0) {
        trainingMeters.forEach((tm, idx) => {
          meters.push(idx === 0 ? { ...tm, sectionHeader: 'Traits in training' } : tm);
        });
      }

      return {
        mode: 'meters',
        hour,
        extent,
        playhead: hour,
        headerCard,
        meters,
        warnings: [...career.warnings, ...(leaderHour?.warnings ?? [])],
        nextChangeAt: nextBoundaryAfter(collectBoundaries(career), hour),
      };
    }

    if (opt.view === 'record') {
      const wonPts: [number, number][] = [[opt.branchFirst, 0]];
      const lostPts: [number, number][] = [[opt.branchFirst, 0]];
      const decided = career.battles
        .filter(b => b.won !== null && b.resultHour !== null && b.resultHour <= hour && b.resultRead !== null && b.resultRead <= hour)
        .sort((a, b) => (a.resultHour ?? a.start) - (b.resultHour ?? b.start));
      let cumWon = 0;
      let cumLost = 0;
      for (const b of decided) {
        const bh = b.resultHour ?? b.start;
        if (b.won) cumWon++; else cumLost++;
        wonPts.push([bh, cumWon]);
        lostPts.push([bh, cumLost]);
      }
      wonPts.push([hour, cumWon]);
      lostPts.push([hour, cumLost]);

      const yMax = Math.max(1, cumWon, cumLost);
      const markers: TimelineSpanMarker[] = career.traitGains
        .filter(([h]) => h >= opt.branchFirst && h <= hour)
        .map(([h, idx]) => ({
          id: `tg:${idx}:${h}`,
          hour: h,
          kind: 'diamond',
          color: paletteAt(opt.surface, 3),
          label: career.traits[idx]?.name ?? career.traits[idx]?.token ?? 'Trait',
        }));

      return {
        mode: 'series',
        hour,
        extent,
        playhead: hour,
        headerCard,
        series: {
          lines: [
            { key: 'won', label: 'Battles won', color: '#3fb950', points: wonPts, endLabel: `Won ${cumWon}` },
            { key: 'lost', label: 'Battles lost', color: '#e5534b', points: lostPts, endLabel: `Lost ${cumLost}` },
          ],
          yMin: 0,
          yMax,
          yFormat: 'integer',
          markers,
        },
        warnings: [...career.warnings, ...(leaderHour?.warnings ?? [])],
        nextChangeAt: nextBoundaryAfter(collectBoundaries(career), hour),
      };
    }

    const commandSpans: TimelineSpanModel[] = [];
    career.commands.forEach((cmd, idx) => {
      if (cmd.firstHour > hour) return;
      const openAtPlayhead = cmd.lastHour >= hour;
      commandSpans.push({
        id: `cmd:${cmd.formation}:${cmd.firstHour}`,
        firstHour: cmd.firstHour,
        lastHour: Math.max(cmd.firstHour, Math.min(cmd.lastHour, hour)),
        color: paletteAt(opt.surface, idx),
        label: cmd.name ?? `${cmd.kind === 'fleet' ? 'Fleet' : 'Formation'} ${cmd.formation}`,
        ended: !openAtPlayhead,
        openAtPlayhead,
      });
    });

    const months = new Map<number, TimelineDivergingBin>();
    for (const b of career.battles) {
      if (b.start > hour || b.read > hour) continue;
      const d = gameDate(b.start);
      const mIdx = d ? monthIndex(d) : b.start;
      const mStart = d ? firstHourOfMonth(mIdx) : b.start;
      const mEnd = d ? Math.min(hour, firstHourOfMonth(mIdx + 1) - 1) : b.start;
      const prev = months.get(mIdx) ?? { firstHour: mStart, lastHour: mEnd, won: 0, lost: 0 };
      const wonInc = b.won === true && b.resultHour !== null && b.resultHour <= hour && b.resultRead !== null && b.resultRead <= hour ? 1 : 0;
      const lostInc = b.won === false && b.resultHour !== null && b.resultHour <= hour && b.resultRead !== null && b.resultRead <= hour ? 1 : 0;
      months.set(mIdx, { firstHour: prev.firstHour, lastHour: prev.lastHour, won: prev.won + wonInc, lost: prev.lost + lostInc });
    }
    const bins = [...months.values()].sort((a, b) => a.firstHour - b.firstHour);
    const maxCount = bins.reduce((m, b) => Math.max(m, b.won, b.lost), 1);

    const traitMarkers: TimelineSpanMarker[] = career.traitGains
      .filter(([h]) => h <= hour)
      .map(([h, idx, cls]) => ({
        id: `trait:${idx}:${h}`,
        hour: Math.max(opt.branchFirst, h),
        kind: 'diamond',
        color: cls === 0 ? '#8c95a6' : paletteAt(opt.surface, 3),
        label: contentTitle(career.traits[idx]?.name, career.traits[idx]?.token ?? String(idx), 'Trait'),
      }));

    const lanes: TimelineLaneModel[] = [
      {
        key: 'lane:command',
        kind: 'standard',
        label: 'Command',
        subtitle: commandSpans.find(s => s.openAtPlayhead)?.label ?? (commandSpans.length ? 'No active command' : 'None recorded'),
        spans: commandSpans,
      },
      {
        key: 'lane:battles',
        kind: 'diverging-bars',
        label: 'Battles',
        subtitle: `${totals.won} won \u00b7 ${totals.lost} lost`,
        diverging: { bins, maxCount },
      },
      {
        key: 'lane:traits',
        kind: 'markers',
        label: 'Traits',
        subtitle: `${traitMarkers.length} ${traitMarkers.length === 1 ? 'trait' : 'traits'}`,
        pointMarkers: traitMarkers,
      },
    ];

    return {
      mode: 'lanes',
      hour,
      extent,
      playhead: hour,
      headerCard,
      lanes,
      bottomMetersTitle: 'Traits in training',
      bottomMeters: trainingMeters,
      iconsOnBars: false,
      labelWidth: 156,
      warnings: [...career.warnings, ...(leaderHour?.warnings ?? [])],
      nextChangeAt: nextBoundaryAfter(collectBoundaries(career), hour),
    };
  },

  tooltip(model, target, hour) {
    if (model.mode === 'lanes') {
      const lane = model.lanes?.find(l => l.key === (target.laneKey ?? target.key));
      if (lane) {
        const span = target.spanId ? lane.spans?.find(s => s.id === target.spanId) : undefined;
        return {
          title: span?.label ?? lane.label,
          subtitle: lane.subtitle,
          hour,
          items: span
            ? [
                { label: 'From', value: formatDay(span.firstHour) },
                { label: 'Until', value: span.openAtPlayhead ? 'Active' : formatDay(span.lastHour) },
              ]
            : [{ label: lane.label, value: lane.subtitle ?? 'Not recorded' }],
        };
      }
      const bm = model.bottomMeters?.find(m => m.key === target.key);
      if (bm) {
        return { title: bm.label, subtitle: 'Trait in training', hour, items: [{ label: 'Progress', value: bm.valueText }] };
      }
    }
    if (model.mode === 'meters') {
      const m = model.meters?.find(row => row.key === target.key);
      if (m) return { title: m.label, subtitle: m.subtitle, hour, items: [{ label: 'Value', value: m.valueText }] };
    }
    return null;
  },
});

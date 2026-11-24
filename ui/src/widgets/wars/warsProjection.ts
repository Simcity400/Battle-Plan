import { formatDay } from '../../dashboard/calendar.ts';
import { startUnseen } from '../../charts/timeline/runState.ts';
import { clipSpansToHour, formatDurationShort, nextBoundaryAfter, paletteAt, resolveCountryChip, urlArtHandle } from '../../families/timeline/helpers.ts';
import type {
  TimelineCountryChip,
  TimelineLaneModel,
  TimelineModel,
  TimelineProjection,
  TimelineSpanMarker,
  TimelineSpanModel,
} from '../../families/timeline/model.ts';
import { registerTimelineProjection } from '../../families/timeline/registry.ts';
import type { WarContributionsReply, WarLossesReply, WarsReply, WarTimeline } from '../../generated/dashboard';
import { labelObservationAt, lossesOf, observationAt, warContributions, type WarsColour, type WarsView } from './warsData.ts';

export interface WarsProjectionOptions {
  readonly view: WarsView;
  readonly colour: WarsColour;
  readonly branchFirst: number;
  readonly surface: string;
  readonly surrenderIconUrl?: string | null;
}

function collectBoundaries(reply: WarsReply): number[] {
  const set = new Set<number>();
  for (const w of reply.wars) {
    set.add(w.firstHour);
    set.add(w.lastHour + 1);
    for (const p of w.participants) {
      set.add(p.firstHour);
      set.add(p.lastHour + 1);
    }
    for (const c of w.capitulations) set.add(c.hour);
  }
  for (const g of reply.unobserved) {
    set.add(g.firstHour);
    set.add(g.lastHour + 1);
  }
  return [...set].sort((a, b) => a - b);
}

function splitSides(war: WarTimeline, hour: number): { readonly attackers: string[]; readonly defenders: string[] } {
  const obs = labelObservationAt(war, Math.min(hour, war.lastHour));
  if (obs) {
    const att = obs.attackersAreViewpoint === false ? obs.opposingSide : obs.viewpointSide;
    const def = obs.attackersAreViewpoint === false ? obs.viewpointSide : obs.opposingSide;
    return { attackers: [...new Set(att)], defenders: [...new Set(def)] };
  }
  const att = new Set<string>();
  const def = new Set<string>();
  for (const p of war.participants) {
    if (p.firstHour > hour) continue;
    if (p.side === 'attacker' || p.side === 'viewpoint') att.add(p.tag);
    else def.add(p.tag);
  }
  return { attackers: [...att], defenders: [...def] };
}

function formatFixed5Int(raw: string): string {
  const n = Number(raw);
  if (!Number.isFinite(n)) return '0';
  return Math.round(n / 100_000).toLocaleString('en-US');
}

export const warsProjection: TimelineProjection<WarsProjectionOptions> = registerTimelineProjection({
  kind: 'wars',

  nextChange(readView) {
    const reply = readView.panel<WarsReply>('main');
    if (!reply) return Infinity;
    return nextBoundaryAfter(collectBoundaries(reply), readView.hour);
  },

  project(readView, opt): TimelineModel {
    const hour = readView.hour;
    const reply = readView.panel<WarsReply>('main');
    const defaultExtent: [number, number] = [opt.branchFirst, Math.max(opt.branchFirst + 1, hour)];
    if (!reply) {
      return { mode: 'lanes', hour, extent: defaultExtent, playhead: hour, lanes: [], empty: 'Loading wars\u2026' };
    }
    if (!reply.available) {
      return {
        mode: 'lanes',
        hour,
        extent: defaultExtent,
        playhead: hour,
        lanes: [],
        empty: 'Wars are not in this recording.',
        emptyKind: 'unavailable',
        warnings: reply.warnings,
      };
    }

    const reachedWars = reply.wars.filter(w => w.firstHour <= hour);
    const ongoingCount = reachedWars.filter(w => w.lastHour >= hour).length;
    const headerSummary = `${reachedWars.length} ${reachedWars.length === 1 ? 'war' : 'wars'} \u00b7 ${ongoingCount} ongoing`;

    if (!reachedWars.length) {
      return {
        mode: 'lanes',
        hour,
        extent: defaultExtent,
        playhead: hour,
        headerSummary,
        lanes: [],
        empty: 'No wars yet.',
        warnings: reply.warnings,
      };
    }

    const firstWarStart = Math.min(...reachedWars.map(w => w.firstHour));
    const windowStart = Math.max(opt.branchFirst, firstWarStart);
    const extent: [number, number] = [windowStart, Math.max(windowStart + 1, hour)];
    const unobserved = clipSpansToHour(reply.unobserved, hour);
    const surrenderHandle = urlArtHandle(opt.surrenderIconUrl);

    if (opt.view === 'countries') {
      const byCountry = new Map<string, { war: WarTimeline; warIndex: number; runs: WarTimeline['participants'] }[]>();
      reachedWars.forEach((war, warIndex) => {
        const grouped = new Map<string, WarTimeline['participants']>();
        for (const p of war.participants) {
          if (p.firstHour > hour) continue;
          const list = grouped.get(p.tag);
          if (list) list.push(p);
          else grouped.set(p.tag, [p]);
        }
        for (const [tag, runs] of grouped) {
          const list = byCountry.get(tag);
          if (list) list.push({ war, warIndex, runs });
          else byCountry.set(tag, [{ war, warIndex, runs }]);
        }
      });

      const entries = [...byCountry.entries()].map(([tag, items]) => {
        const chip = resolveCountryChip(readView, tag, hour);
        const ongoing = items.some(i => i.war.lastHour >= hour);
        return { tag, chip, items, ongoing };
      });
      entries.sort((a, b) => Number(b.ongoing) - Number(a.ongoing) || a.chip.name.localeCompare(b.chip.name));

      const lanes: TimelineLaneModel[] = entries.map(({ tag, chip, items }) => {
        const spans: TimelineSpanModel[] = [];
        for (const { war, warIndex, runs } of items) {
          const obs = labelObservationAt(war, Math.min(hour, war.lastHour));
          const warTitle = obs?.name.known ? obs.name.text : 'War';
          for (const run of runs) {
            const openAtPlayhead = run.lastHour >= hour && war.lastHour >= hour;
            const color = opt.colour === 'side'
              ? (run.side === 'attacker' || run.side === 'viewpoint' ? paletteAt(opt.surface, 1) : paletteAt(opt.surface, 0))
              : paletteAt(opt.surface, warIndex);
            const caps = war.capitulations
              .filter(c => c.tag === tag && c.hour <= hour && c.hour >= run.firstHour && c.hour <= run.lastHour)
              .map<TimelineSpanMarker>(c => ({ id: `cap:${war.id}:${tag}:${c.hour}`, hour: c.hour, kind: 'surrender', icon: surrenderHandle }));
            spans.push({
              id: `${war.id}:${tag}:${run.firstHour}`,
              firstHour: run.firstHour,
              lastHour: Math.max(run.firstHour, Math.min(run.lastHour, hour)),
              color,
              label: warTitle,
              fadedStart: run.joinedBy,
              fadedEnd: run.leftBy && !openAtPlayhead,
              ended: !openAtPlayhead,
              openAtPlayhead,
              markers: caps,
              meta: { kind: 'country-war', warId: war.id, warTitle, tag },
            });
          }
        }
        return {
          key: `country:${tag}`,
          kind: 'standard',
          label: chip.name,
          subtitle: `${items.length} ${items.length === 1 ? 'war' : 'wars'}`,
          flag: chip.flag,
          flagTag: tag,
          spans,
          auditValue: `${chip.name}|${items.length}`,
          meta: { kind: 'country', tag },
        };
      });

      return {
        mode: 'lanes',
        hour,
        extent,
        playhead: hour,
        headerSummary,
        lanes,
        unobserved,
        iconsOnBars: false,
        labelWidth: 204,
        warnings: reply.warnings,
        nextChangeAt: nextBoundaryAfter(collectBoundaries(reply), hour),
      };
    }

    const orderedWars = [...reachedWars].sort((a, b) => {
      const aOn = a.lastHour >= hour;
      const bOn = b.lastHour >= hour;
      if (aOn !== bOn) return Number(bOn) - Number(aOn);
      return b.lastHour - a.lastHour || b.firstHour - a.firstHour;
    });

    const lanes: TimelineLaneModel[] = [];

    orderedWars.forEach((war, warIdx) => {
      const warKey = `war:${war.id}`;
      const ongoing = war.lastHour >= hour;
      const warEnd = ongoing ? hour : Math.min(hour, war.lastHour);
      const obs = labelObservationAt(war, warEnd);
      const sides = splitSides(war, warEnd);
      const capsBefore = war.capitulations.filter(c => c.hour <= hour);
      const capTags = new Set(capsBefore.map(c => c.tag));

      const attChips: TimelineCountryChip[] = sides.attackers.map((tag, i) => {
        const c = resolveCountryChip(readView, tag, warEnd);
        return { ...c, color: c.color ?? paletteAt(opt.surface, (warIdx * 2 + i) % 8), dimmed: !ongoing && capTags.has(tag) };
      });
      const defChips: TimelineCountryChip[] = sides.defenders.map((tag, i) => {
        const c = resolveCountryChip(readView, tag, warEnd);
        return { ...c, color: c.color ?? paletteAt(opt.surface, (warIdx * 2 + 1 + i) % 8), dimmed: !ongoing && capTags.has(tag) };
      });

      const attLead = attChips[0] ?? resolveCountryChip(readView, null, warEnd);
      const defLead = defChips[0] ?? resolveCountryChip(readView, null, warEnd);
      const attLost = !ongoing && attChips.length > 0 && attChips.every(c => capTags.has(c.tag));
      const defLost = !ongoing && defChips.length > 0 && defChips.every(c => capTags.has(c.tag));

      const lastCap = capsBefore.at(-1);
      const lastCapChip = lastCap ? resolveCountryChip(readView, lastCap.tag, Math.min(hour, lastCap.hour)) : null;

      const outcome = ongoing
        ? {
            status: 'running' as const,
            label: `Ongoing ${formatDurationShort(Math.max(0, hour - war.firstHour))}`,
          }
        : lastCap && lastCapChip
          ? {
              status: 'finished' as const,
              label: `${lastCapChip.name} capitulated`,
              detail: formatDay(lastCap.hour),
            }
          : {
              status: 'ended' as const,
              label: 'Ended',
              detail: formatDay(warEnd),
            };

      const warTitle = obs?.name.known ? obs.name.text : null;
      const fallbackLabel = `${attLead.name} \u2013 ${defLead.name}`;
      const capMarkers: TimelineSpanMarker[] = capsBefore.map(c => ({
        id: `cap:${war.id}:${c.tag}:${c.hour}`,
        hour: c.hour,
        kind: 'surrender',
        icon: surrenderHandle,
        label: `${resolveCountryChip(readView, c.tag, c.hour).name} capitulated`,
      }));

      lanes.push({
        key: warKey,
        kind: 'matchup',
        label: warTitle ?? fallbackLabel,
        subtitle: warTitle ? undefined : `${formatDay(war.firstHour)}${ongoing ? '' : ` \u2013 ${formatDay(warEnd)}`}`,
        expandable: true,
        defaultOpen: ongoing,
        matchup: {
          title: warTitle,
          attackers: { lead: attLead, allies: attChips.slice(1), dimmed: attLost },
          defenders: { lead: defLead, allies: defChips.slice(1), dimmed: defLost },
          outcome,
        },
        spans: [{
          id: `${warKey}:span`,
          firstHour: war.firstHour,
          lastHour: Math.max(war.firstHour, warEnd),
          color: attLead.color ?? paletteAt(opt.surface, 1),
          bottomColor: defLead.color ?? paletteAt(opt.surface, 0),
          openAtPlayhead: ongoing,
          ended: !ongoing,
          markers: capMarkers,
        }],
        auditValue: `${warTitle ?? fallbackLabel}|${outcome.label}`,
        meta: { kind: 'war', warId: war.id, title: warTitle ?? fallbackLabel, ongoing, firstHour: war.firstHour, endHour: warEnd, outcomeLabel: outcome.label },
      });

      const buildParticipants = (tags: readonly string[], sideTag: 'att' | 'def', sideDimmed: boolean) => {
        for (const tag of tags) {
          const runs = war.participants.filter(p => p.tag === tag && p.firstHour <= hour);
          if (!runs.length) continue;
          const firstJoin = runs[0]!.firstHour;
          const lastSeen = Math.min(hour, runs.at(-1)!.lastHour);
          const chip = resolveCountryChip(readView, tag, lastSeen);
          const capped = capsBefore.find(c => c.tag === tag);
          const joinedLate = firstJoin > war.firstHour + 24;
          const pColor = chip.color ?? paletteAt(opt.surface, sideTag === 'att' ? 1 : 0);
          const pSpans: TimelineSpanModel[] = runs.map(r => {
            const pOpen = ongoing && r.lastHour >= hour;
            const pCap = capsBefore.filter(c => c.tag === tag && c.hour >= r.firstHour && c.hour <= r.lastHour);
            return {
              id: `${war.id}:${sideTag}:${tag}:${r.firstHour}`,
              firstHour: r.firstHour,
              lastHour: Math.max(r.firstHour, Math.min(r.lastHour, hour)),
              color: pColor,
              heldBefore: startUnseen(r.firstHour, opt.branchFirst, reply.unobserved),
              fadedStart: r.joinedBy,
              fadedEnd: r.leftBy && !pOpen,
              ended: !pOpen,
              openAtPlayhead: pOpen,
              markers: pCap.map(c => ({ id: `pcap:${war.id}:${tag}:${c.hour}`, hour: c.hour, kind: 'surrender' as const, icon: surrenderHandle })),
            };
          });
          lanes.push({
            key: `war-p:${war.id}:${sideTag}:${tag}`,
            kind: 'participant',
            parentKey: warKey,
            depth: 1,
            label: chip.name,
            subtitle: joinedLate ? `Joined ${formatDay(firstJoin)}` : capped ? `Capitulated ${formatDay(capped.hour)}` : undefined,
            flag: chip.flag,
            flagTag: tag,
            flagDimmed: sideDimmed || Boolean(capped && !ongoing),
            dimmed: sideDimmed || Boolean(capped && !ongoing),
            spans: pSpans,
            auditValue: `${chip.name}|${formatDay(firstJoin)}`,
            meta: { kind: 'participant', warId: war.id, warTitle: warTitle ?? fallbackLabel, tag, firstJoin, lastSeen },
          });
        }
      };

      buildParticipants(sides.attackers, 'att', attLost);
      if (sides.defenders.length > 0) {
        lanes.push({
          key: `war-sep:${war.id}`,
          kind: 'section-header',
          parentKey: warKey,
          depth: 1,
          label: 'Against',
          height: 18,
        });
        buildParticipants(sides.defenders, 'def', defLost);
      }
    });

    return {
      mode: 'lanes',
      hour,
      extent,
      playhead: hour,
      headerSummary,
      lanes,
      unobserved,
      iconsOnBars: false,
      labelWidth: 284,
      outcomeWidth: 196,
      warnings: reply.warnings,
      nextChangeAt: nextBoundaryAfter(collectBoundaries(reply), hour),
    };
  },

  tooltip(model, target, hour, readView) {
    const reply = readView.panel<WarsReply>('main');
    const lane = model.lanes?.find(l => l.key === (target.laneKey ?? target.key));
    if (!lane) return null;
    const meta = lane.meta as { kind?: string; warId?: string; title?: string; warTitle?: string; tag?: string; ongoing?: boolean; firstHour?: number; endHour?: number; outcomeLabel?: string } | undefined;
    const warId = meta?.warId;
    const war = warId ? reply?.wars.find(w => w.id === warId) : undefined;
    const lossesReply = readView.hover<WarLossesReply>('war-losses', hour)?.data;
    const contribReply = readView.hover<WarContributionsReply>('war-contributions', hour)?.data;
    const warLoss = warId ? lossesOf(lossesReply, warId) : undefined;
    const obs = war ? observationAt(war, hour) : null;
    const contrib = war && obs ? warContributions(contribReply?.contributions, obs) : null;

    const items: { label: string; value: string; divider?: boolean; tone?: 'muted' }[] = [];
    if (meta?.firstHour !== undefined) {
      items.push({ label: 'Started', value: formatDay(meta.firstHour) });
    }
    if (meta?.outcomeLabel) {
      items.push({ label: 'Status', value: meta.outcomeLabel });
    }
    if (warLoss?.run) {
      items.push({ label: 'Attacker casualties', value: formatFixed5Int(warLoss.run[2]), divider: true });
      items.push({ label: 'Defender casualties', value: formatFixed5Int(warLoss.run[3]) });
    } else if (warLoss?.reason) {
      items.push({ label: 'Casualties', value: 'Not recorded', tone: 'muted', divider: true });
    }
    if (contrib && contrib.participants.length > 0) {
      const top = [...contrib.participants]
        .sort((a, b) => Number(b.participation) - Number(a.participation))
        .slice(0, 4);
      top.forEach((p, idx) => {
        const chip = resolveCountryChip(readView, p.tag, hour);
        const pct = Math.round(Number(p.participation) / 1000);
        items.push({
          label: chip.name,
          value: `${pct}% \u00b7 ${formatFixed5Int(p.casualties)} casualties`,
          ...(idx === 0 ? { divider: true } : {}),
        });
      });
    }

    return {
      title: meta?.title ?? meta?.warTitle ?? lane.label,
      subtitle: meta?.kind === 'participant' ? lane.label : formatDay(hour),
      hour,
      items,
    };
  },
});

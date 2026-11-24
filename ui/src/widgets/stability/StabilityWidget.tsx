import { useMemo } from 'react';
import { contentTitle } from '../../dashboard/contentTitles.ts';
import { answersKey, countryFromKey } from '../../dashboard/countryKey.ts';
import { defaultCountryKey } from '../../dashboard/countrySelection.ts';
import type { WidgetProps } from '../../dashboard/definitions';
import { resolveCountries } from '../../dashboard/groups.ts';
import { ideaName, validateIdeaHistoryReply } from '../../dashboard/ideaHistoryReply.ts';
import { resolveOptions } from '../../dashboard/options.ts';
import { useProduct } from '../../dashboard/products.ts';
import { productRequest } from '../../dashboard/request.ts';
import { useCountryGroups } from '../../dashboard/useCountryGroups.ts';
import DeclaredTimeChartWidget from '../../families/time-chart/DeclaredTimeChartWidget.tsx';
import type {
  Artwork,
  FocusArtworkParams,
  FocusArtworkReply,
  FocusReply,
  HourSpan,
  IdeaHistoryReply,
  PartyHistoryReply,
} from '../../generated/dashboard';
import { validateFocusArtworkReply, validateFocusReply } from '../national-focus/focusData.ts';
import { partyBadges, validatePartyHistoryReply } from '../ruling-party/partyData.ts';
import { STABILITY_OPTIONS } from './definition.ts';
import StabilityBreakdownView from './StabilityBreakdownView.tsx';
import { stabilityDeclaration, type StabilityRawMarker } from './stabilityProjection.ts';

function inSpans(spans: readonly HourSpan[], hour: number): boolean {
  return spans.some(s => s.firstHour <= hour && hour <= s.lastHour);
}

export default function StabilityWidget(props: WidgetProps) {
  const { widget, runtime, active } = props;
  const options = resolveOptions(STABILITY_OPTIONS, widget.options);
  const pinnedBreakdown = options.show === 'stability' || options.show === 'warSupport';

  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const lastHour = track?.branch.lastHour ?? 0;
  const fallback = defaultCountryKey(track);

  const wantWars = options.markWars !== false;
  const wantFactions = options.markFactions !== false;
  const wantGov = options.markGovernment !== false;
  const wantFocus = options.markFocus !== false;

  const groupsActive = active && !pinnedBreakdown && (widget.countries.kind === 'group' || wantWars || wantFactions);
  const groups = useCountryGroups(runtime, track, groupsActive);
  const resolved = useMemo(
    () => resolveCountries(widget.countries, fallback ? [fallback] : [], groups.data, lastHour, Number.MAX_SAFE_INTEGER),
    [widget.countries, fallback, groups.data, lastHour],
  );
  const singleTag = !pinnedBreakdown && resolved.tags.length === 1 ? resolved.tags[0]! : null;

  const ideasReq = useMemo(
    () => (track && singleTag && wantGov
      ? productRequest({ sourceId: track.source.id, revision: track.source.revision, product: 'idea-history', params: { branchId: track.branch.id, country: countryFromKey(singleTag) } })
      : null),
    [track, singleTag, wantGov],
  );
  const partiesReq = useMemo(
    () => (track && singleTag && wantGov
      ? productRequest({ sourceId: track.source.id, revision: track.source.revision, product: 'party-history', params: { branchId: track.branch.id, country: countryFromKey(singleTag) } })
      : null),
    [track, singleTag, wantGov],
  );
  const focusReq = useMemo(
    () => (track && singleTag && wantFocus
      ? productRequest({ sourceId: track.source.id, revision: track.source.revision, product: 'national-focus', params: { branchId: track.branch.id, country: countryFromKey(singleTag) } })
      : null),
    [track, singleTag, wantFocus],
  );

  const ideas = useProduct<IdeaHistoryReply>(runtime.client, ideasReq, active && !pinnedBreakdown && wantGov, validateIdeaHistoryReply, track?.source);
  const parties = useProduct<PartyHistoryReply>(runtime.client, partiesReq, active && !pinnedBreakdown && wantGov, validatePartyHistoryReply, track?.source);
  const focus = useProduct<FocusReply>(runtime.client, focusReq, active && !pinnedBreakdown && wantFocus, validateFocusReply, track?.source);

  const completedFocuses = useMemo(() => {
    if (!focus.data) return [];
    const seen = new Set<string>();
    const out: { readonly token: string; readonly hour: number }[] = [];
    for (const seg of focus.data.history.segments) {
      for (const entry of seg.order.completed) {
        if (entry.completedByHour == null) continue;
        const key = `${entry.token}#${entry.ordinal}#${entry.completedByHour}`;
        if (seen.has(key)) continue;
        seen.add(key);
        out.push({ token: entry.token, hour: entry.completedByHour });
      }
    }
    return out;
  }, [focus.data]);

  const focusArtParams = useMemo<FocusArtworkParams | null>(() => {
    if (!track || !singleTag || !wantFocus || completedFocuses.length === 0) return null;
    const tokens = [...new Set(completedFocuses.map(f => f.token))].slice(0, 256);
    if (!tokens.length) return null;
    return {
      branchId: track.branch.id,
      hour: track.branch.lastHour,
      country: countryFromKey(singleTag),
      database: 'national',
      tokens,
      textOnly: false,
      includeDescriptions: false,
      iconsOnly: true,
    };
  }, [track, singleTag, wantFocus, completedFocuses]);

  const focusArtReq = useMemo(
    () => (track && focusArtParams
      ? productRequest({ sourceId: track.source.id, revision: track.source.revision, product: 'focus-artwork', params: focusArtParams })
      : null),
    [track, focusArtParams],
  );
  const validateFocusArt = useMemo(
    () => (focusArtParams ? (raw: unknown) => validateFocusArtworkReply(raw, focusArtParams) : undefined),
    [focusArtParams],
  );
  const focusArt = useProduct<FocusArtworkReply>(runtime.client, focusArtReq, active && !pinnedBreakdown && wantFocus, validateFocusArt, track?.source);

  const rawMarkers = useMemo<readonly StabilityRawMarker[]>(() => {
    if (!track || !singleTag || pinnedBreakdown) return [];
    const branchFirst = track.branch.firstHour;
    const branchLast = track.branch.lastHour;
    const tag = singleTag;
    const out: StabilityRawMarker[] = [];

    if (wantFocus && completedFocuses.length > 0) {
      const staticNames = new Map((focus.data?.staticText ?? []).map(a => [a.token, a.name]));
      const replyIcons = new Map((focus.data?.artwork ?? []).map(a => [a.token, a]));
      const artIcons = new Map((focusArt.data?.items ?? []).map(i => [i.artwork.token, i.artwork]));
      for (const entry of completedFocuses) {
        const fetched = artIcons.get(entry.token);
        const fallbackArt = replyIcons.get(entry.token);
        const name = contentTitle(staticNames.get(entry.token) ?? fetched?.name ?? fallbackArt?.name, entry.token, 'Focus name unavailable');
        out.push({
          id: `focus:${entry.token}:${entry.hour}`,
          hour: entry.hour,
          kind: 'focus',
          title: `Focus: ${name}`,
          iconUrl: fetched?.icon ?? fallbackArt?.icon ?? null,
        });
      }
    }

    const ideaReply = ideas.data && answersKey(ideas.data.asked, tag) ? ideas.data : undefined;
    if (wantGov && ideaReply) {
      const laws = new Set(ideaReply.categories.flatMap(c => c.slots.flatMap(s => s.laws)));
      const seats = new Set(ideaReply.categories.flatMap(c => c.slots.filter(s => s.character).map(s => s.token)));
      const infos = new Map(ideaReply.ideas.map(i => [i.token, i]));
      const artwork = new Map<string, Artwork>(ideaReply.artwork.map(a => [a.token, a]));
      const holderKey = (tok: string, ch: number, script: string, nm: string) => JSON.stringify([tok, ch, script, nm]);
      const holders = new Map(ideaReply.holders.map(h => [holderKey(h.token, h.character, h.characterToken, h.name), h.artwork]));
      const holding = (span: IdeaHistoryReply['spans'][number], h: number) =>
        span.character ? `c#${span.slot}#${span.character.id}#${h}` : `i#${span.token}#${span.slot}#${h}`;
      const endings = new Set(ideaReply.spans.map(sp => holding(sp, sp.lastHour + 1)));
      const starts = new Set(ideaReply.identityStarts);
      for (const span of ideaReply.spans) {
        if (span.firstHour <= branchFirst || inSpans(ideaReply.unobserved, span.firstHour - 1) || starts.has(span.firstHour)) continue;
        if (endings.has(holding(span, span.firstHour))) continue;
        const info = infos.get(span.token);
        const slot = span.slot ?? info?.slot ?? null;
        const art = artwork.get(span.token);
        if (info?.law === true || laws.has(span.token)) {
          out.push({
            id: `law:${span.token}:${span.firstHour}`,
            hour: span.firstHour,
            kind: 'idea',
            title: `Law: ${ideaName(art, info, 'Idea name unavailable')}`,
            iconUrl: art?.icon ?? null,
          });
        } else if (slot && seats.has(slot)) {
          const holderArt = span.character
            ? holders.get(holderKey(span.token, span.character.id, span.character.token, span.character.name))
            : undefined;
          const label = span.character
            ? (holderArt?.name ?? 'holder not recorded')
            : ideaName(art, info, 'Idea name unavailable');
          out.push({
            id: `adv:${span.token}:${span.firstHour}`,
            hour: span.firstHour,
            kind: 'idea',
            title: `Advisor: ${label}`,
            iconUrl: holderArt?.icon ?? art?.icon ?? null,
          });
        }
      }
    }

    const partyReply = parties.data && answersKey(parties.data.asked, tag) ? parties.data : undefined;
    if (wantGov && partyReply) {
      for (const badge of partyBadges(partyReply, tok => contentTitle(partyReply.ideologies.find(i => i.token === tok)?.name, tok, 'Ideology name unavailable'))) {
        if (badge.glyph !== '✓') {
          out.push({
            id: `party:${badge.hour}:${badge.label}`,
            hour: badge.hour,
            kind: 'party',
            title: badge.label,
          });
        }
      }
    }

    const g = groups.data;
    if (g && wantWars) {
      for (const war of g.warStarts) {
        const other = war.attacking === tag ? war.defending : war.defending === tag ? war.attacking : null;
        if (!other) continue;
        out.push({
          id: `war:${other}:${war.hour}`,
          hour: war.hour,
          kind: 'war',
          title: `War with ${other} began`,
          otherTag: other,
          warVerb: 'began',
          warAlert: true,
        });
      }
      const enemies = new Map<string, typeof g.wars>();
      for (const war of g.wars) {
        const enemy = war.first === tag ? war.second : war.second === tag ? war.first : null;
        if (!enemy) continue;
        const list = enemies.get(enemy);
        if (list) list.push(war);
        else enemies.set(enemy, [war]);
      }
      for (const ev of g.capitulations) {
        if (ev.tag === tag) {
          out.push({ id: `cap:${tag}:${ev.hour}`, hour: ev.hour, kind: 'war', title: 'Capitulated', warAlert: true });
          continue;
        }
        const endsNow = (war: (typeof g.wars)[number]) => ev.hour === war.lastHour + 1 && !inSpans(g.unobserved.wars, ev.hour);
        const enemy = (enemies.get(ev.tag) ?? []).some(war => war.firstHour <= ev.hour && (ev.hour <= war.lastHour || endsNow(war)));
        if (enemy) {
          out.push({
            id: `cap:${ev.tag}:${ev.hour}`,
            hour: ev.hour,
            kind: 'war',
            title: `${ev.tag} capitulated`,
            otherTag: ev.tag,
            warVerb: 'capitulated',
            warAlert: true,
          });
        }
      }
    }

    if (g && wantFactions) {
      const gaps = g.unobserved.factions;
      for (const faction of g.factions) {
        const founders = faction.members.filter(m => m.firstHour === faction.firstHour);
        const unknown = [...gaps, ...faction.incomplete];
        const sole = founders.length === 1 && !inSpans(unknown, faction.firstHour);
        for (const run of faction.members.filter(m => m.tag === tag)) {
          if (run.firstHour > branchFirst && !inSpans(unknown, run.firstHour - 1)) {
            out.push({
              id: `fac-j:${faction.id}:${run.firstHour}`,
              hour: run.firstHour,
              kind: 'faction',
              title: `${run.firstHour === faction.firstHour && sole ? 'Founded' : 'Joined'} ${faction.name ?? 'Unnamed faction'}`,
              factionId: faction.id,
              factionVerb: run.firstHour === faction.firstHour && sole ? 'Founded' : 'Joined',
              fallbackFactionName: faction.name ?? undefined,
            });
          }
          if (run.lastHour < branchLast && !inSpans(unknown, run.lastHour + 1)) {
            out.push({
              id: `fac-l:${faction.id}:${run.lastHour + 1}`,
              hour: run.lastHour + 1,
              kind: 'faction',
              title: `Left ${faction.name ?? 'Unnamed faction'}`,
              factionId: faction.id,
              factionVerb: 'Left',
              fallbackFactionName: faction.name ?? undefined,
            });
          }
        }
      }
    }

    return out;
  }, [track, singleTag, pinnedBreakdown, wantFocus, wantGov, wantWars, wantFactions, completedFocuses, focus.data, focusArt.data, ideas.data, parties.data, groups.data]);

  if (pinnedBreakdown) {
    return <StabilityBreakdownView {...props} />;
  }

  const markersSig = `${rawMarkers.length}:${focusArt.data?.items.length ?? 0}:${ideas.data?.spans.length ?? 0}:${groups.data?.warStarts.length ?? 0}`;
  return (
    <DeclaredTimeChartWidget
      {...props}
      declaration={stabilityDeclaration}
      extraOptions={rawMarkers}
      extraBindingKey={markersSig}
    />
  );
}

import { useMemo } from 'react';
import { contentTitle } from '../../dashboard/contentTitles.ts';
import { answersKey, countryFromKey } from '../../dashboard/countryKey.ts';
import { defaultCountryKey } from '../../dashboard/countrySelection.ts';
import type { WidgetProps } from '../../dashboard/definitions';
import { resolveCountries } from '../../dashboard/groups.ts';
import { holdingLabel, lookOf, validateIdeaHistoryReply } from '../../dashboard/ideaHistoryReply.ts';
import { resolveOptions } from '../../dashboard/options.ts';
import { useProduct } from '../../dashboard/products.ts';
import { useCampaignGate } from '../../dashboard/regionGate.ts';
import { productRequest } from '../../dashboard/request.ts';
import { useCountryGroups } from '../../dashboard/useCountryGroups.ts';
import { DeclaredFamilyWidget } from '../../families/host/index.ts';
import { timeChartHostAdapter } from '../../families/time-chart/hostAdapter.ts';
import type {
  FocusReply,
  HourSpan,
  IdeaHistoryReply,
  IdeaHolding,
  PartyHistoryReply,
} from '../../generated/dashboard';
import { validateFocusReply } from '../national-focus/focusData.ts';
import { partyBadges, validatePartyHistoryReply } from '../ruling-party/partyData.ts';
import { STABILITY_OPTIONS } from './definition.ts';
import StabilityDetail from './StabilityDetail.tsx';
import { stabilityDeclarationWith, type StabilityRawMarker } from './stabilityProjection.ts';

function inSpans(spans: readonly HourSpan[], hour: number): boolean {
  return spans.some(s => s.firstHour <= hour && hour <= s.lastHour);
}

export default function StabilityWidget(props: WidgetProps) {
  const { widget, runtime, active } = props;
  const options = resolveOptions(STABILITY_OPTIONS, widget.options);

  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const lastHour = track?.branch.lastHour ?? 0;
  const fallback = defaultCountryKey(track);

  const wantWars = options.markWars !== false;
  const wantFactions = options.markFactions !== false;
  const wantGov = options.markGovernment !== false;
  const wantFocus = options.markFocus !== false;

  const groupsActive = active && (widget.countries.kind === 'group' || wantWars || wantFactions);
  const groups = useCountryGroups(runtime, track, groupsActive);
  const resolved = useMemo(
    () => resolveCountries(widget.countries, fallback ? [fallback] : [], groups.data, lastHour, Number.MAX_SAFE_INTEGER),
    [widget.countries, fallback, groups.data, lastHour],
  );
  const singleTag = resolved.tags.length === 1 ? resolved.tags[0]! : null;

  const ideasEnd = useCampaignGate(runtime, widget, active && wantGov).hour;
  const ideasReq = useMemo(
    () => (track && singleTag && wantGov && ideasEnd !== null
      ? productRequest({ sourceId: track.source.id, revision: track.source.revision, product: 'idea-history', params: { branchId: track.branch.id, country: countryFromKey(singleTag), hour: ideasEnd } })
      : null),
    [track, singleTag, wantGov, ideasEnd],
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

  const ideas = useProduct<IdeaHistoryReply>(runtime.client, ideasReq, active && wantGov, validateIdeaHistoryReply, track?.source);
  const parties = useProduct<PartyHistoryReply>(runtime.client, partiesReq, active && wantGov, validatePartyHistoryReply, track?.source);
  const focus = useProduct<FocusReply>(runtime.client, focusReq, active && wantFocus, validateFocusReply, track?.source);

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

  const rawMarkers = useMemo<readonly StabilityRawMarker[]>(() => {
    if (!track || !singleTag) return [];
    const branchFirst = track.branch.firstHour;
    const branchLast = track.branch.lastHour;
    const tag = singleTag;
    const out: StabilityRawMarker[] = [];

    if (wantFocus && completedFocuses.length > 0) {
      const staticNames = new Map((focus.data?.staticText ?? []).map(a => [a.token, a.name]));
      const replyArt = new Map((focus.data?.artwork ?? []).map(a => [a.token, a]));
      for (const entry of completedFocuses) {
        const art = replyArt.get(entry.token);
        const name = contentTitle(staticNames.get(entry.token) ?? art?.name, entry.token, 'Focus name unavailable');
        out.push({
          id: `focus:${entry.token}:${entry.hour}`,
          hour: entry.hour,
          kind: 'focus',
          title: `Focus: ${name}`,
          sprite: art?.sprite ?? null,
        });
      }
    }

    const ideaReply = ideas.data && answersKey(ideas.data.asked, tag) ? ideas.data : undefined;
    if (wantGov && ideaReply) {
      const laws = new Set(ideaReply.categories.flatMap(c => c.slots.flatMap(s => s.laws)));
      const seats = new Set(ideaReply.categories.flatMap(c => c.slots.filter(s => s.character).map(s => s.token)));
      const holderAt = (holding: IdeaHolding, h: number) => `${holding.slot}#${holding.character?.id}#${h}`;
      const endings = new Set(ideaReply.holdings.flatMap(holding => holding.character ? [holderAt(holding, holding.lastHour + 1)] : []));
      for (const holding of ideaReply.holdings) {
        if (holding.start !== 'seen' || holding.firstHour <= branchFirst) continue;
        if (holding.character && endings.has(holderAt(holding, holding.firstHour))) continue;
        const look = lookOf(ideaReply, holding);
        const slot = holding.slot ?? look.info.slot;
        const art = look.artwork;
        if (look.info.law || laws.has(holding.token)) {
          out.push({
            id: `law:${holding.token}:${holding.firstHour}`,
            hour: holding.firstHour,
            kind: 'idea',
            title: `Law: ${holdingLabel(look, holding)}`,
            sprite: art.sprite ?? null,
          });
        } else if (slot && seats.has(slot)) {
          out.push({
            id: `adv:${holding.token}:${holding.firstHour}`,
            hour: holding.firstHour,
            kind: 'idea',
            title: `Advisor: ${holdingLabel(look, holding)}`,
            sprite: art.sprite ?? null,
          });
        }
      }
    }

    const partyReply = parties.data && answersKey(parties.data.asked, tag) ? parties.data : undefined;
    if (wantGov && partyReply) {
      for (const badge of partyBadges(partyReply, tok => contentTitle(partyReply.ideologies.find(i => i.token === tok)?.name, tok, 'Ideology name unavailable'), Number.POSITIVE_INFINITY)) {
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
  }, [track, singleTag, wantFocus, wantGov, wantWars, wantFactions, completedFocuses, focus.data, ideas.data, parties.data, groups.data]);

  const declaration = useMemo(() => stabilityDeclarationWith(rawMarkers), [rawMarkers]);


  return (
    <StabilityDetail
      {...props}
      chart={onActivate => (
        <DeclaredFamilyWidget
          {...props}
          adapter={timeChartHostAdapter}
          declaration={declaration}
          onActivate={action => onActivate(action.subject, action.hour)}
        />
      )}
    />
  );
}

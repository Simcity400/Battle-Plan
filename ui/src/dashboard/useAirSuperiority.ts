import { useCallback, useEffect, useMemo } from 'react';
import { useChartTokens } from '../charts/useChartTokens';
import type { OptionChoices, WidgetProps, WidgetRuntime } from './definitions';
import type { WidgetSpec } from './model';
import { contentKey, contentSet, resolveOptions, setToken } from './options';
import type { OptionField, OptionValue } from './options';
import { useProduct } from './products';
import { defaultCountryTag, selectedTags } from './countrySelection';
import { useFactionColours } from './FactionColours';
import { looksRead, useCountryLooks } from './countryLooks';
import { useIdentityHour } from './atHead';
import { trackHourOf } from './settledHour';
import { countryLineColours } from './countryLineColours.ts';
import { useHeadProduct } from './headProduct';
import { airSideName, momentRows, validateAirChoices, validateAirSuperiority } from './airSuperiority';
import type { AirLooks } from './airSuperiority';
import { airChoiceKey, airChoiceToken } from './airOptions';
import type { AirChoicesReply, AirSide, AirSuperiorityReply, FactionGroup } from '../generated/dashboard';

const NO_WARNINGS: string[] = [];
const NO_SIDES: AirSide[] = [];

/** The settings as the products ask them: the comparison, its viewpoint, and the sides, regions and theatres chosen. */
export function airMode(options: Readonly<Record<string, OptionValue>>, campaign: string, currentCountry: string | null) {
  const mode = options.mode === 'viewpoint' ? 'viewpoint' as const : 'sides' as const;
  const chosen = options.viewpoint ? String(options.viewpoint) : '';
  const viewpoint = mode !== 'viewpoint' ? '' : chosen ? contentKey(chosen).startsWith('country:') ? contentKey(chosen) : airChoiceKey(campaign, chosen) ?? ''
    : currentCountry ? `country:${currentCountry}` : '';
  return { mode, viewpoint, sides: contentSet(options.sides).flatMap(value => airChoiceKey(campaign, value) ?? []),
    regions: contentSet(options.regions).map(value => Number(contentKey(value).slice(2))).filter(Number.isInteger),
    theatres: contentSet(options.theatres).flatMap(value => airChoiceKey(campaign, value) ?? []) };
}
export type AirModeSettings = ReturnType<typeof airMode>;

/** The params of `air-superiority` for `mode`; regions are chosen by the widget, not the product, unless asked. */
export function airParams(mode: AirModeSettings) {
  return { mode: mode.mode, ...(mode.viewpoint ? { viewpoint: mode.viewpoint.startsWith('country:') ? mode.viewpoint.slice(8) : mode.viewpoint,
    ...(mode.viewpoint.startsWith('country:') ? { viewpointCountry: true } : {}) } : {}),
    ...(mode.sides.length ? { sides: mode.sides } : {}), ...(mode.regions.length ? { regions: mode.regions } : {}),
    ...(mode.theatres.length ? { theatres: mode.theatres } : {}) };
}

/** The timeline's choices: one request a recording, never at the playhead. */
export function useAirChoicesReply(runtime: WidgetRuntime, widget: WidgetSpec, active: boolean) {
  const track = runtime.tracks.find(t => t.id === widget.trackIds[0]);
  const request = track ? { sourceId: track.source.id, revision: track.source.revision, product: 'air-superiority-choices', params: { branchId: track.branch.id } } : null;
  return useProduct<AirChoicesReply>(runtime.client, request, active, validateAirChoices, track?.source);
}

/** A faction's name as the groups name it at `hour` (its latest otherwise), the reader's own name first. */
export function factionNamer(factions: readonly FactionGroup[], own: ReadonlyMap<string, string>, hour: number | null) {
  const byId = new Map(factions.map(faction => [faction.id, faction]));
  return (id: string) => {
    const mine = own.get(id);
    if (mine) return mine;
    const faction = byId.get(id);
    if (!faction) return `Faction ${id}`;
    const at = hour === null ? undefined : faction.names.find(run => run.firstHour <= hour && hour <= run.lastHour)?.name;
    return at ?? faction.name ?? `Faction ${id}`;
  };
}

export function useAirChoices(widget: WidgetSpec, runtime: WidgetRuntime, fields: readonly OptionField[]): OptionChoices {
  const track = runtime.tracks.find(t => t.id === widget.trackIds[0]);
  const choices = useAirChoicesReply(runtime, widget, true);
  const factions = useFactionColours(runtime.client, track?.source);
  const hour = track?.branch.lastHour ?? 0;
  const tags = useMemo(() => [...new Set([...(choices.data?.countries ?? []), ...(choices.data?.sides.flatMap(side => [side.tag, side.leader].flatMap(tag => tag ? [tag] : [])) ?? [])])],
    [choices.data]);
  const looks = useCountryLooks(runtime.client, track, tags, hour, true);
  const options = resolveOptions(fields, widget.options);
  return useMemo(() => {
    const campaign = track?.source.metadata.campaignKey ?? '';
    const faction = factionNamer(factions.factions, factions.displayNames, null);
    const country = (tag: string) => looks.name(tag, hour);
    const sides = (choices.data?.sides ?? []).map(side => ({ value: airChoiceToken(campaign, side.key), label: airSideName(side, country, faction) }));
    const chosenRegions = contentSet(options.regions).map(contentKey);
    const regions = (choices.data?.regions ?? []).map(region => ({ value: setToken(`r:${region.id}`), label: region.name }));
    for (const key of chosenRegions) if (!regions.some(region => contentKey(region.value) === key)) regions.push({ value: setToken(key), label: `Region ${key.slice(2)} · not recorded` });
    return {
      viewpoint: [...sides, ...(choices.data?.countries ?? []).map(tag => ({ value: setToken(`country:${tag}`), label: country(tag) }))], sides, regions,
      theatres: (choices.data?.theatres ?? []).map(theatre => ({ value: airChoiceToken(campaign, theatre.id), label: theatre.name })),
    };
  }, [choices.data, looks, hour, factions.factions, factions.displayNames, options.regions, track?.source.metadata.campaignKey]);
}

/** The moment at the widget's hour (followed at the head as a panel, asked for a pinned or hovered hour), with the
 * sides' names and colours. Every score is a live cell: nothing here moves with the playhead. A push renders only when
 * its rows change (`momentRows`); `current` is the newest reply, read where it is painted (`live` tells when one lands). */
export function useAirMoment(props: WidgetProps, options: Readonly<Record<string, OptionValue>>, hoverHour: number | null = null) {
  const { widget, runtime, active, onDiagnostics } = props;
  const track = runtime.tracks.find(t => t.id === widget.trackIds[0]);
  const campaign = track?.source.metadata.campaignKey ?? '';
  const currentCountry = selectedTags(widget.countries, track)[0] ?? defaultCountryTag(track);
  const mode = useMemo(() => airMode(options, campaign, currentCountry),
    [options.mode, options.viewpoint, options.sides, options.regions, options.theatres, campaign, currentCountry]); // eslint-disable-line react-hooks/exhaustive-deps
  const needsViewpoint = mode.mode === 'viewpoint' && !mode.viewpoint;
  const params = useMemo(() => track && !needsViewpoint ? airParams(mode) : null, [track, needsViewpoint, mode]);
  const product = useHeadProduct<AirSuperiorityReply>(runtime, widget, 'air-superiority', params, { active, validate: validateAirSuperiority, structure: momentRows, hoverHour });
  const looks = useAirLooks(props, product.data, product.at, mode, product.read === 'followed');
  useEffect(() => {
    const warnings = [...(product.data?.warnings ?? []), ...looks.warnings];
    onDiagnostics(warnings.length ? warnings : NO_WARNINGS);
    return () => onDiagnostics(NO_WARNINGS);
  }, [product.data, looks.warnings, onDiagnostics]);
  return { ...product, track, mode, needsViewpoint, ...looks };
}

/** Names and colours of a reply's sides at the hour its structure is of, and its viewpoint's. `pushed` (a panel at the
 * head): at the last hour at or before the head at which one of them changed, so a push alone asks and renders nothing. */
export function useAirLooks(props: Pick<WidgetProps, 'runtime' | 'widget' | 'active'>, data: { sides: readonly AirSide[]; viewpoint: number | null } | undefined, at: number | null,
  mode: AirModeSettings, pushed = false) {
  const { runtime, widget, active } = props;
  const track = runtime.tracks.find(t => t.id === widget.trackIds[0]);
  const tokens = useChartTokens();
  const sides = data?.sides ?? NO_SIDES;
  const viewpointTag = mode.viewpoint.startsWith('country:') ? mode.viewpoint.slice(8) : null;
  const tags = useMemo(() => [...new Set([...sides.flatMap(side => [side.tag, side.leader].flatMap(tag => tag ? [tag] : [])), ...(viewpointTag ? [viewpointTag] : [])])], [sides, viewpointTag]);
  const identityHour = useIdentityHour(runtime.client, track, tags, runtime.clock, active && pushed,
    clockHour => track ? trackHourOf(widget, track, runtime.tracks, clockHour) : null);
  const read = looksRead(active && pushed, identityHour, Math.floor(at ?? track?.branch.lastHour ?? 0));
  const hour = read.hour;
  const names = useCountryLooks(runtime.client, track, tags, hour, active, undefined, undefined, read.exact);
  const anchors = useMemo(() => new Map(sides.flatMap(side => side.anchor ? [[side.anchor.tag, side.anchor.hour] as const] : [])), [sides]);
  const anchorTags = useMemo(() => [...anchors.keys()], [anchors]);
  const anchorLooks = useCountryLooks(runtime.client, track, anchorTags, hour, active, undefined, anchors, true);
  const factions = useFactionColours(runtime.client, track?.source);
  const faction = useMemo(() => factionNamer(factions.factions, factions.displayNames, hour), [factions.factions, factions.displayNames, hour]);
  const country = useCallback((tag: string) => ({ name: names.name(tag, hour), flag: names.flag(tag, hour) }), [names, hour]);
  const sideName = useCallback((side: AirSide) => airSideName(side, tag => names.name(tag, hour), faction), [names, hour, faction]);
  const colours = useMemo(() => {
    const entries = new Map<string, { color: [number, number, number] }>();
    for (const side of sides) {
      const anchor = side.anchor ? anchorLooks.color(side.anchor.tag, side.anchor.hour) : null;
      const tag = side.tag ?? side.leader;
      const color = anchor ?? (tag ? names.color(tag, hour) : null);
      if (color) entries.set(side.key, { color });
    }
    if (viewpointTag) { const color = names.color(viewpointTag, hour); if (color) entries.set(`country:${viewpointTag}`, { color }); }
    return countryLineColours([...entries.keys()].sort(), entries, tokens.surface, tokens.countryColours === 'fixed');
  }, [sides, anchorLooks, names, hour, viewpointTag, tokens.surface, tokens.countryColours]);
  const color = useCallback((side: AirSide) => side.colourFaction ? factions.colours.get(side.colourFaction) ?? tokens.muted : colours.get(side.key) ?? tokens.muted,
    [factions.colours, colours, tokens.muted]);
  const looks: AirLooks = useMemo(() => {
    const side = data?.viewpoint == null ? undefined : sides[data.viewpoint];
    const viewpoint = side ? { name: sideName(side), color: color(side) }
      : viewpointTag ? { name: names.name(viewpointTag, hour), tag: viewpointTag, color: colours.get(`country:${viewpointTag}`) ?? tokens.muted }
        : { name: 'No country selected', color: tokens.muted };
    return { name: sideName, color, viewpoint, enemy: tokens.error, scale: 100_000, badBelow: 40_000, goodAbove: 60_000, mode: mode.mode };
  }, [data?.viewpoint, sides, sideName, color, viewpointTag, names, hour, colours, tokens.muted, tokens.error, mode.mode]);
  const legend = useMemo(() => mode.mode === 'viewpoint' ? [
    { id: 'friendly', label: `${looks.viewpoint.name} (friendly)`, color: looks.viewpoint.color },
    { id: 'enemy', label: `Enemies of ${looks.viewpoint.name}`, color: tokens.error },
  ] : [...sides].sort((a, b) => (a.faction == null) === (b.faction == null) ? 0 : a.faction == null ? 1 : -1).map(side => ({ id: side.key, label: sideName(side), color: color(side),
    onColor: side.colourFaction ? (hex: string) => factions.setColour(side.colourFaction!, hex, 'campaign') : undefined })),
  [mode.mode, looks.viewpoint, tokens.error, sides, sideName, color, factions.setColour]); // eslint-disable-line react-hooks/exhaustive-deps
  const warnings = useMemo(() => [...factions.warnings, ...(factions.error ? [String(factions.error)] : []), ...(names.warnings ?? [])], [factions.warnings, factions.error, names.warnings]);
  return { tokens, looks, legend, country, faction, warnings };
}

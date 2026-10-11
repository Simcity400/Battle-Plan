import { useMemo, useState } from 'react';
import WidgetState from '../../components/WidgetState';
import { useContextMenu } from '../../components/ContextMenu';
import ChordChart from '../../charts/chord/ChordChart';
import type { ChordFrame } from '../../charts/chord/ChordChart';
import { jumpItem } from '../../charts/jump';
import { useChartTokens } from '../../charts/useChartTokens';
import { useCountryColours } from '../../charts/race/countryColours';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import { useCountryFactsAt, nameKey } from '../../dashboard/countryNamesAt';
import type { FactSpec } from '../../dashboard/countryFacts';
import type { WidgetProps } from '../../dashboard/definitions';
import { defaultCountryTag } from '../../dashboard/countrySelection';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { usePlaybackHour } from '../../dashboard/playback';
import { useProduct } from '../../dashboard/products';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import { compareTags } from '../../dashboard/tagOrder';
import type { CountryStyleReply, OperativePlacementReply } from '../../generated/dashboard';
import { OPTIONS } from './definition';
import { placementChordMemo, placementTooltip, placementsAt, selectPlacements, validatePlacementsReply } from './placementsData';
import type { PlacementChord } from './placementsData';
import './placements.css';

const COLORS: FactSpec<[number, number, number]> = { product: 'country-styles', perRequest: 64,
  params: (branchId, hour, tags) => ({ branchId, hour, tags }),
  extract: value => new Map((value as CountryStyleReply).styles.flatMap(style => style.color ? [[style.tag, style.color] as const] : [])),
};
const NO_WARNINGS: string[] = [];

export default function WhoSpiesOnWhomWidget({ widget, runtime, active, onDiagnostics }: WidgetProps) {
  usePlaybackHour(runtime.clock, active && widget.time.mode === 'follow');
  const tokens = useChartTokens(), options = resolveOptions(OPTIONS, widget.options);
  const track = runtime.tracks.find(t => t.id === widget.trackIds[0]);
  const request = track ? { sourceId: track.source.id, revision: track.source.revision, product: 'operative-placements', params: { branchId: track.branch.id } } : null;
  const { data, error, retry } = useProduct<OperativePlacementReply>(runtime.client, request, active, validatePlacementsReply);
  const groups = useCountryGroups(runtime, track, active);
  const window = track && resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour());
  const hour = window?.lastHour ?? 0;
  const current = useMemo(() => data ? placementsAt(data, hour) : null, [data, hour]);
  const selectedTag = defaultCountryTag(track);
  const picked = useMemo(() => current ? selectPlacements(current.placements, widget.countries, groups.data, hour, track?.branch.lastHour ?? hour, selectedTag) : null,
    [current, widget.countries, groups.data, hour, track?.branch.lastHour, selectedTag]);
  const wanted = useMemo(() => [...new Set(picked?.placements.flatMap(n => [n.owner, n.target]) ?? [])].map(tag => ({ hour, tag })), [picked, hour]);
  const { labelAt } = useCountryLabelsAt(runtime.client, track, wanted, active);
  const colourTags = useMemo(() => [...new Set(groups.data?.countryIdentities?.map(country => country.tag) ?? [])].sort(compareTags), [groups.data]);
  const colourWanted = useMemo(() => colourTags.map(tag => ({ hour, tag })), [colourTags, hour]);
  const colors = useCountryFactsAt(runtime.client, track, colourWanted, active, COLORS);
  const colourRevision = useMemo(() => ({}), [colors]);
  const plottedColor = useCountryColours(colourTags, tag => colors.get(nameKey(hour, tag)), tokens, colourRevision);
  const menu = useContextMenu('Operative placement actions');
  const [chartError, setChartError] = useState<string | null>(null);
  const chordAt = useMemo(() => placementChordMemo(), []);
  const diagnostics = useMemo(() => {
    const missing = wanted.filter(w => !colors.has(nameKey(w.hour, w.tag))).length;
    return [...(data?.warnings ?? NO_WARNINGS), ...(missing ? [`${missing} country map colours are unavailable at the shown hour; their arcs use the muted theme colour.`] : [])];
  }, [data, wanted, colors]);
  useDiagnostics(onDiagnostics, diagnostics);

  // Resolve each hour, but fold only when exact data or presentation changes.
  // Weak keys connect an exact frame to its tooltip, without retaining old hours.
  const frames = useMemo(() => {
    const exact = new WeakMap<object, PlacementChord>();
    let lastHour = NaN, lastSeek = -1, frame: ChordFrame = { data: null };
    const at = (now: number): ChordFrame => {
      const snapshot = runtime.clock.getSnapshot();
      const seek = widget.time.mode === 'follow' ? snapshot.seekSequence : 0;
      const shown = track ? resolveWindow(widget, track, runtime.tracks, widget.time.mode === 'follow' ? runtime.clock.positionAt(now) : snapshot.position)?.lastHour ?? NaN : NaN;
      if (shown === lastHour && lastSeek === seek) return { ...frame, playing: widget.time.mode === 'follow' && snapshot.playing };
      const resolved = data && placementsAt(data, shown);
      const picked = resolved && selectPlacements(resolved.placements, widget.countries, groups.data, shown, track?.branch.lastHour ?? shown, selectedTag);
      const segment = JSON.stringify([track?.source.id, track?.source.revision, track?.branch.id, resolved?.period?.firstHour, seek]);
      if (!resolved || resolved.period?.availability !== 'recorded' || resolved.read == null || !picked || picked.reason) {
        frame = { data: null, segment, playing: widget.time.mode === 'follow' && snapshot.playing };
      } else {
        const chord = chordAt(picked.placements, Number(options.mostCountries), resolved.read, labelAt(shown),
          tag => colors.has(nameKey(hour, tag)) ? plottedColor(tag) : tokens.muted, tokens.muted);
        exact.set(chord.data, chord);
        frame = { data: chord.data, segment, playing: widget.time.mode === 'follow' && snapshot.playing };
      }
      lastHour = shown; lastSeek = seek;
      return frame;
    };
    return { at, exact };
  }, [data, track, widget, runtime.clock, runtime.tracks, groups.data, options.mostCountries, labelAt, plottedColor, colors, hour, tokens.muted, selectedTag, chordAt]);

  if (!track) return <WidgetState kind="empty" icon="target" message="Load a campaign to see its operative placements." />;
  if (error || groups.error) return <WidgetState kind="error" message={error ?? groups.error!} onRetry={error ? retry : groups.retry} />;
  if (!data || !current) return <WidgetState kind="loading" label="Loading operative placements…" />;
  if (current.period?.availability !== 'recorded') return <WidgetState kind="unavailable" message={current.period?.availability === 'featureAbsent'
    ? 'Not in this recording. The intelligence agency feature was disabled.' : 'Not in this recording. Operative placements were not recorded at this time.'} />;
  if (picked?.reason) return <WidgetState kind="unavailable" message={picked.reason} />;
  return <div className={`who-spies-on-whom${chartError ? ' has-chart-error' : ''}`}>
    <ChordChart label="Who spies on whom" caption="Ribbon colour = the country spying" frame={{ data: null }} frameAt={frames.at} onError={setChartError}
      hoursPerSecond={runtime.clock.getSnapshot().hoursPerSecond} active={active}
      tooltip={(hit, data) => { const chord = frames.exact.get(data); return chord ? placementTooltip(hit, chord) : null; }}
      onContextMenu={(hit, data, x, y) => {
        const chord = frames.exact.get(data);
        if (chord && (hit.kind === 'flow' ? chord.flows.has(hit.id) : chord.nodes.has(hit.id))) menu.open(x, y, [jumpItem(runtime, track.id, chord.read)]);
      }} />
    {menu.menu}
    {chartError && <WidgetState kind="error" message={chartError} />}
    {!picked?.placements.some(n => n.count > 0) && <p className="placements-empty">No operatives placed yet.</p>}
    {wanted.some(w => !colors.has(nameKey(w.hour, w.tag))) && <p className="sr-only">Some country colours were not recorded; their arcs use the muted theme colour.</p>}
  </div>;
}

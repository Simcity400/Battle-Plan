import { useEffect, useMemo } from 'react';
import { useContextMenu } from '../../components/ContextMenu';
import WidgetState from '../../components/WidgetState';
import { dayLabel } from '../../charts/draw';
import { jumpItem } from '../../charts/jump';
import Legend from '../../charts/Legend';
import { countryColourMaps } from '../../charts/race/countryColours';
import { countryRaceItems, partColours } from '../../charts/race/countryRace';
import RaceChart from '../../charts/race/RaceChart';
import type { LiveRace } from '../../charts/race/RaceChart';
import type { RaceItem } from '../../charts/race/race';
import type { CountryFlagLook } from '../../components/CountryFlag';
import { fitTooltip } from '../../charts/tooltip';
import { useChartTokens, useViewportHeight } from '../../charts/useChartTokens';
import { formatAxisValue } from '../../charts/valueAxis';
import { useContentRevision } from '../../dashboard/contentRevision';
import type { WidgetProps } from '../../dashboard/definitions';
import { resolveOptions } from '../../dashboard/options';
import { useProduct } from '../../dashboard/products';
import { usePlaybackMotion } from '../../dashboard/playback';
import { LiveLabels } from '../../live/liveLabels';
import { useLiveAnswers } from '../../live/useLiveAnswers';
import '../../charts/race/countryRace.css';
import { OPTIONS } from './definition';
import { BAR_PARTS, FALLBACK_LABELS, SCORE_PARTS, gameScoreCountryKnown, gameScoreLabel, gameScoreRace, gameScoreTooltips, validateGameScoreReply } from './gameScoreData';
import type { ScoreRow } from './gameScoreData';
import type { GameScoreReply } from '../../generated/dashboard';
import { liveScoreHour } from './liveScoreHour';

interface RaceMoment { reply: GameScoreReply; rows: ReadonlyMap<string, ScoreRow>; entries: ReturnType<typeof gameScoreRace> }
const NO_ITEMS: readonly RaceItem[] = [];
const EMPTY = 'Total scores unavailable at this time.';
const NO_FLAG: CountryFlagLook = { flagReason: 'The country flag is unavailable at this date.' };

/** The Game score race of every country, its bars drawn from hours read ahead of the playhead:
 * the chart asks for the moment the clock shows each frame, and nothing renders as playback moves. */
export default function CanvasGameScoreRace({ widget, runtime, active }: WidgetProps) {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const options = resolveOptions(OPTIONS, widget.options);
  const split = options.split === true;
  const content = useContentRevision(runtime.client);
  const tokens = useChartTokens();
  const viewport = useViewportHeight();
  const hoursPerSecond = usePlaybackMotion(runtime.clock);
  const menu = useContextMenu('Game score actions');
  const lane = track ? JSON.stringify([track.source.id, track.source.revision, track.branch.id, content, split]) : null;
  const { store, phase } = useLiveAnswers<RaceMoment>({ active, lane, runtime, widget, track,
    phaseOf: moment => moment.entries.length ? { state: 'ready' } : { state: 'empty' },
    read: async (hour, signal, restart) => {
      const { prepared, rows, through, partial } = await liveScoreHour(runtime, track!, content, hour, signal, restart);
      return { through, partial, value: { reply: prepared.reply, rows: new Map(rows.map(row => [row.key, row])), entries: gameScoreRace(rows, split) } };
    } });
  const labelsRequest = track ? { sourceId: track.source.id, revision: track.source.revision, product: 'game_score', params: { branchId: track.branch.id } } : null;
  const names = useProduct<GameScoreReply>(runtime.client, labelsRequest, active, validateGameScoreReply).data?.labels ?? FALLBACK_LABELS;
  const parts = useMemo(() => BAR_PARTS.map(id => ({ id, label: gameScoreLabel(names, id) })), [names]);
  const colours = useMemo(() => partColours(parts, tokens.categories), [parts, tokens.categories]);
  const live = useMemo<{ race: LiveRace; row: (item?: RaceItem) => { row: ScoreRow; reply: GameScoreReply } | null; flag: (item?: RaceItem) => CountryFlagLook; start: () => void; stop: () => void } | null>(() => {
    if (!store || !track) return null;
    const wakers = new Set<() => void>();
    // Subscribes only once started (an effect): a render that is thrown away leaves nothing behind.
    const labels = new LiveLabels(runtime.client, track.source, track.branch.id, content, 'panel', () => { labelsChanged++; for (const wake of wakers) wake(); });
    let labelsChanged = 0, built: unknown = null, builtLabels = -1, items: readonly RaceItem[] = NO_ITEMS, moment: RaceMoment | null = null, hour: number | null = null;
    let flags = new Map<string, CountryFlagLook>();
    let colourKey = '', country = new Map<string, string>(), overlayHour: number | null = null, overlay = '';
    const current = () => {
      const answer = store.answer();
      if (!answer) return items;
      if (answer.value !== built || labelsChanged !== builtLabels) {
        built = answer.value; builtLabels = labelsChanged; moment = answer.value; hour = answer.hour;
        labels.ask(answer.hour);
        // Each country's recorded colour at the hour, else the palette's, kept while they stay the same.
        const tags = moment.entries.map(entry => entry.tag);
        // Rows are keyed by the native identity an entry names; colours are asked for by tag.
        const recorded = new Map(moment.entries.map(entry => [entry.tag, moment!.rows.get(entry.entityKey ?? entry.tag)?.identity.color ?? null]));
        const colourOf = (tag: string) => recorded.get(tag) ?? null;
        const key = tags.map(tag => `${tag}:${colourOf(tag)?.join(',') ?? ''}`).join(';');
        if (key !== colourKey) { colourKey = key; country = countryColourMaps(tags, colourOf, tokens).colors; }
        items = countryRaceItems(moment.entries, colours, tag => country.get(tag) ?? tokens.muted, tokens.muted, false);
        // Flags looked up once a moment, not each frame.
        flags = new Map(items.map(item => {
          const row = moment!.rows.get(item.key);
          if (!row || !gameScoreCountryKnown(row.identity)) return [item.key, NO_FLAG];
          const label = labels.label(row.hour, row.tag);
          return [item.key, { flag: label.flag, flagPending: label.flagPending, flagReason: label.flagReason }];
        }));
        labels.draw();
      }
      return items;
    };
    const rowOf = (item?: RaceItem) => item && moment ? moment.rows.get(item.key) ?? null : null;
    return {
      race: {
        items: current,
        label: () => `Game score, ${dayLabel(hour ?? runtime.clock.getSnapshot().hour)}`,
        overlay: () => {
          if (options.dateOverlay !== true || hour === null) return null;
          if (hour !== overlayHour) { overlayHour = hour; overlay = dayLabel(hour); }
          return overlay;
        },
        bind: wake => { wakers.add(wake); const stop = store.bind(wake); return () => { wakers.delete(wake); stop(); }; },
        clock: runtime.clock,
      },
      row: item => { const row = rowOf(item); return row && moment ? { row, reply: moment.reply } : null; },
      flag: item => (item && flags.get(item.key)) ?? NO_FLAG,
      start: () => { labels.start(); for (const wake of wakers) wake(); },
      stop: () => { labels.stop(); },
    };
  }, [store, track, runtime, content, tokens, colours, options.dateOverlay]);
  useEffect(() => { if (!live) return; live.start(); return () => live.stop(); }, [live]);

  if (!track) return <WidgetState kind="empty" icon="grid" message="Load a campaign to see its game scores." />;
  return <>
    {phase.state === 'error' && <WidgetState kind="error" failure={phase.failure} onRetry={store?.retry} />}
    {(phase.state === 'loading' || !live) && <WidgetState kind="loading" label="Loading game scores…" />}
    {phase.state === 'empty' && <WidgetState kind="empty" message={EMPTY} />}
    {phase.state === 'unavailable' && <WidgetState kind="unavailable" message={phase.message} />}
    {live && <div className="country-race" hidden={phase.state !== 'ready'}>
      <Legend label="Score parts" items={(split ? parts : []).map(part => ({ id: part.id, label: part.label, color: colours.get(part.id) ?? tokens.muted }))} />
      <div className="country-race-chart">
        <RaceChart label="Game score" items={NO_ITEMS} live={live.race} slots={0}
          name={(_key, item) => (item && live.row(item)?.row.identity.name) ?? 'Country name unavailable'}
          countryFlag={(_key, item) => live.flag(item)}
          tooltip={(_key, item) => { const found = live.row(item); return found ? fitTooltip([...gameScoreTooltips(found.reply, found.row)], viewport, { keep: SCORE_PARTS.length }) : null; }}
          onContextMenu={(_key, x, y, item) => { const found = live.row(item); if (found) menu.open(x, y, [jumpItem(runtime, track.id, found.row.hour)]); }}
          formatTick={formatAxisValue} empty={EMPTY} hoursPerSecond={hoursPerSecond} active={active} resetKey={`${track.source.id}:${track.branch.id}`} />
      </div>
      {menu.menu}
    </div>}
  </>;
}

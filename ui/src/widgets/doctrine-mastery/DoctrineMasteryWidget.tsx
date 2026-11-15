import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import WidgetState from '../../components/WidgetState';
import { ChipSet } from '../../components/ChipSet';
import { useContextMenu } from '../../components/ContextMenu';
import ProgressMeter from '../../charts/meter/ProgressMeter';
import type { MeterMark } from '../../charts/meter/ProgressMeter';
import { meterValue } from '../../charts/meter/meterValue';
import { dayLabel } from '../../charts/draw';
import { jumpItem } from '../../charts/jump';
import { useRowTooltips } from '../../charts/useRowTooltips';
import { tooltipText } from '../../charts/tooltip';
import type { TooltipContent } from '../../charts/tooltip';
import type { LiveValues } from '../../charts/live';
import type { WidgetProps } from '../../dashboard/definitions';
import { contentKey, contentSet, resolveOptions, setToken } from '../../dashboard/options';
import { selectedTags } from '../../dashboard/countrySelection';
import { useHeadProduct } from '../../dashboard/headProduct';
import { useProduct } from '../../dashboard/products';
import { reporting } from '../../dashboard/follow/liveCells';
import { useShownHour } from '../../dashboard/widgetPerf';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import { useSpriteArtwork } from '../../dashboard/useSpriteArtwork';
import { nameKey, useCountryNamesAt } from '../../dashboard/countryNamesAt';
import { nameReadText } from '../../dashboard/nameReads';
import { activeTracks, choiceName, doctrineIcons, doctrineSprite, folderName, masteryStructure, masteryText, rateText, requirementMet,
  rewardDescription, rewardName, rewardUnlocked, trackKey, trackMarks, trackName, trackNumbers, trackRatio, trackScale, trackTexts, unlockMastery,
  unlockedRewards, validateDoctrineMastery, validateDoctrineRewards } from './doctrineMasteryData';
import type { TrackNumbers } from './doctrineMasteryData';
import { OPTIONS } from './definition';
import './doctrineMastery.css';
import type { DoctrineMasteryReply, DoctrineRewardsReply, MasteryFolder, MasteryTrack } from '../../generated/dashboard';

/** Each doctrine track's mastery against its reward thresholds, filling downward as the game's track does. Following
 * playback it is a pushed panel that renders only when folders, tracks, names or unlocks change; mastery, bank, gains
 * and the meters' fill are live cells painted each frame. */
export default function DoctrineMasteryWidget({ widget, runtime, active, onDiagnostics, setOption, headerSlot }: WidgetProps) {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const options = resolveOptions(OPTIONS, widget.options);
  const tag = selectedTags(widget.countries, track)[0] ?? null;
  const root = useRef<HTMLDivElement | null>(null);
  const head = useHeadProduct<DoctrineMasteryReply>(runtime, widget, 'doctrine-mastery', tag ? { tag } : null,
    { active, validate: validateDoctrineMastery, structure: masteryStructure });
  const { data, error, retry } = head;
  const attach = useCallback((element: HTMLDivElement | null) => { root.current = element; head.root(element); }, [head.root]);
  const report = useShownHour();
  const shownAt = useRef(head.shownAt);
  shownAt.current = head.shownAt;
  const live = useMemo(() => reporting(head.live, () => report(shownAt.current())), [head.live, report]);
  const image = useSpriteArtwork(runtime, track, useMemo(() => doctrineIcons(data).map(doctrineSprite), [data]), active);
  const art = (key: string | null | undefined) => image(doctrineSprite(key));
  const chosen = contentSet(options.folders).map(contentKey);
  const shown = useMemo(() => data?.folders.filter(folder => !chosen.length || chosen.includes(folder.id)) ?? [],
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [data, JSON.stringify(chosen)]);
  // Every number is written into the page in the frame its cells move: no render.
  useLayoutEffect(() => {
    if (!data) return undefined;
    const paint = () => paintTracks(root.current, shown, live.read());
    paint();
    return live.subscribe(paint);
  }, [data, shown, live]);
  // The hour a tooltip speaks of: the one asked, else the head's when followed.
  const hourShown = () => head.hour ?? Math.floor(runtime.clock.getHour());
  const [describing, setDescribing] = useState<{ folder: number; track: number; hour: number } | null>(null);
  const described = useProduct<DoctrineRewardsReply>(runtime.client, track && tag && describing ? { sourceId: track.source.id, revision: track.source.revision,
    product: 'doctrine-rewards', params: { branchId: track.branch.id, tag, folder: describing.folder, track: describing.track, hour: describing.hour } } : null,
  active && describing !== null, validateDoctrineRewards, track?.source);
  const menu = useContextMenu('Doctrine mastery');
  const tracks = useMemo(() => new Map<string, { folder: MasteryFolder; item: MasteryTrack }>(shown.flatMap(folder =>
    folder.tracks.map(item => [trackKey(folder, item), { folder, item }] as const))), [shown]);
  const content = useCallback((key: string): TooltipContent | null => {
    const [kind, ...rest] = key.split('#');
    const found = tracks.get(rest[0]);
    if (!found) return null;
    const { folder, item } = found;
    const numbers = trackNumbers(live.read(), rest[0]);
    if (kind === 'mark') {
      const index = Number(rest[1]);
      const next = item.rewards[index];
      if (!next) return null;
      const description = rewardDescription(described.data, item, index);
      const needed = unlockMastery(item)[index];
      const met = requirementMet(item, index, numbers.mastery);
      const missing = needed == null || numbers.mastery === null ? null : needed - numbers.mastery;
      return { title: rewardName(next), rows: [{ label: 'Mastery required', value: needed == null ? 'Not recorded' : masteryText(needed) },
        { label: 'Unlocked', value: rewardUnlocked(item, index) ? 'Yes' : 'No' },
        { label: 'Mastery', value: met === null || missing === null ? 'Not recorded' : met ? 'Requirement met' : `${masteryText(missing)} to go` }],
      ...description ? { description } : {} };
    }
    return trackTooltip(folder, item, numbers, dayLabel(hourShown()));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [tracks, described.data, head.hour, live]);
  const tips = useRowTooltips<string>({ owner: `doctrine-mastery:${widget.id}`, active, content, revision: `${head.hour}:${described.data ? 'described' : ''}`,
    onMenu: (_, x, y) => { if (track) menu.open(x, y, [jumpItem(runtime, track.id, hourShown())]); } });
  // A hovered reward's descriptions, with the country's values, load for its track at the hour then shown.
  useEffect(() => {
    const [kind, key] = tips.hovered?.split('#') ?? [];
    const [folder, item] = (key ?? '').split('|').map(Number);
    setDescribing(kind === 'mark' && Number.isSafeInteger(folder) && Number.isSafeInteger(item) ? { folder: folder!, track: item!, hour: hourShown() } : null);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [tips.hovered]);
  useDiagnostics(onDiagnostics, data ? [...data.warnings, ...(described.data?.warnings ?? [])] : null);
  const nameHour = head.at ?? head.hour;
  const country = useCountryNamesAt(runtime.client, track, useMemo(() => tag && nameHour !== null && data && !data.folders.length ? [{ hour: nameHour, tag }] : [],
    [tag, nameHour, data]), active);
  const countryName = tag && nameHour !== null ? nameReadText(country.get(nameKey(nameHour, tag))) : undefined;
  const pill = headerSlot && data && createPortal(<span className="pill">{activeTracks(data.folders)} tracks active</span>, headerSlot);
  if (!track) return <WidgetState kind="empty" icon="flask" message="Load a campaign to see doctrines." />;
  if (!tag) return <WidgetState kind="empty" icon="flask" message="Choose a country in the widget's header." />;
  if (head.read === 'off' && !data) return <WidgetState kind="unavailable" message="Choose another time in widget settings." />;
  if (error) return <WidgetState kind="error" failure={error} onRetry={retry} />;
  if (!data) return <div ref={attach} className="doctrine-mastery"><WidgetState kind="loading" label="Loading doctrines…" /></div>;
  const chips = data.folders.map(folder => ({ value: folder.id, label: folderName(folder), icon: art(folder.icon) }));
  const toggle = (id: string) => {
    const next = chosen.includes(id) ? chosen.filter(item => item !== id) : [...chosen, id];
    setOption?.('folders', next.length === chips.length ? '' : next.map(setToken).join(' '));
  };
  const values = live.read();
  return <div ref={attach} className="doctrine-mastery">
    {pill}
    {chips.length > 1 && setOption && <div className="log-chips doctrine-mastery-chips"><ChipSet label="Doctrine folders" chips={chips}
      selected={chosen.length ? chosen : chips.map(chip => chip.value)} max={chips.length} onToggle={toggle} /></div>}
    {!data.folders.length ? <WidgetState kind="empty" message={`${countryName ?? 'This country'} has no doctrines at this date.`} />
      : <div className="doctrine-mastery-folders themed-scrollbar">{shown.map(folder => <section key={folder.id} className="doctrine-mastery-folder" aria-label={folderName(folder)}>
        <h3 className="doctrine-mastery-heading">{art(folder.grand?.icon) && <img src={art(folder.grand?.icon)!} alt="" />}
          <span>{folderName(folder)}</span>
          <small>{choiceName(folder.grand, 'No grand doctrine chosen')}</small></h3>
        <div className="doctrine-mastery-tracks">{folder.tracks.map(item => {
          const key = trackKey(folder, item);
          const marks = trackMarks(item, true);
          const numbers = trackNumbers(values, key);
          const texts = trackTexts(numbers);
          // A known scale draws the fill element; its height is painted from the mastery cell.
          const ratio = trackScale(item) === null ? null : trackRatio(item, numbers.mastery) ?? 0;
          return <div key={key} data-track={key} className={`doctrine-mastery-track${item.sub === null ? ' is-empty' : ''}`} {...tips.rowProps(`track#${key}`)}>
            {tips.hovered === `track#${key}` && <span id={tips.descriptionId(`track#${key}`)} className="sr-only">{tooltipText(trackTooltip(folder, item, numbers, dayLabel(hourShown())))}</span>}
            <div className="doctrine-mastery-sub">{art(item.sub?.icon) && <img src={art(item.sub?.icon)!} alt="" />}
              <span>{choiceName(item.sub, 'No subdoctrine chosen')}</span></div>
            <div className="doctrine-mastery-meter">
              <ProgressMeter orientation="vertical" value={ratio} state={item.sub === null ? 'idle' : marks.every(mark => mark.reached) && marks.length ? 'done' : 'live'}
                label={`${trackName(item)} mastery`} marks={marks}
                markProps={(mark: MeterMark) => tips.rowProps(`mark#${key}#${mark.id.split(':').pop()}`)} />
            </div>
            <dl className="doctrine-mastery-values">
              <div><dt>Mastery</dt><dd data-paint="mastery">{texts.mastery}</dd></div>
              <div><dt>Daily</dt><dd data-paint="daily">{texts.daily}</dd></div>
              <div><dt>Banked</dt><dd data-paint="bank">{texts.bank}</dd></div>
            </dl>
          </div>;
        })}</div>
      </section>)}</div>}
    {menu.menu}
  </div>;
}

/** Each shown track's texts and meter fill from its live cells. */
function paintTracks(root: Element | null, folders: readonly MasteryFolder[], values: LiveValues<number>): void {
  if (!root) return;
  for (const folder of folders) for (const item of folder.tracks) {
    const key = trackKey(folder, item);
    const element = root.querySelector<HTMLElement>(`[data-track="${key}"]`);
    if (!element) continue;
    const numbers = trackNumbers(values, key);
    const texts = trackTexts(numbers);
    for (const cell of element.querySelectorAll<HTMLElement>('[data-paint]')) {
      const next = texts[cell.dataset.paint as keyof typeof texts];
      if (next !== undefined && cell.textContent !== next) cell.textContent = next;
    }
    const meter = element.querySelector<HTMLElement>('.progress-meter');
    const fill = meter?.querySelector<HTMLElement>(':scope > i');
    if (!meter || !fill) continue;
    const ratio = meterValue(trackRatio(item, numbers.mastery));
    fill.style.height = `${(ratio ?? 0) * 100}%`;
    if (ratio === null) { meter.removeAttribute('aria-valuenow'); meter.setAttribute('aria-valuetext', numbers.missing || 'Not recorded'); }
    else { meter.setAttribute('aria-valuenow', String(ratio)); meter.removeAttribute('aria-valuetext'); }
  }
}

function trackTooltip(folder: MasteryFolder, item: MasteryTrack, numbers: TrackNumbers, date: string): TooltipContent {
  const scale = trackScale(item);
  const recorded = (raw: bigint | null, text: (raw: bigint) => string) => raw === null ? 'Not recorded' : text(raw);
  return {
    title: item.sub?.name ?? trackName(item),
    rows: [
      { label: 'Track', value: trackName(item) },
      { label: 'Mastery', value: recorded(numbers.mastery, raw => `${masteryText(raw)}${scale === null ? '' : ` of ${masteryText(scale)}`}`) },
      { label: 'Banked mastery', value: recorded(numbers.bank, masteryText) },
      { label: 'Daily gain', value: recorded(numbers.effective, rateText) },
      { label: 'Units’ daily gain', value: recorded(numbers.daily, rateText) },
      { label: 'Rewards unlocked', value: String(item.unlocked) },
      ...unlockedRewards(item).map(next => ({ label: rewardName(next), indent: true })),
      ...folder.completed.length ? [{ label: 'Completed subdoctrines' }, ...folder.completed.map(sub => ({ label: choiceName(sub, ''), indent: true }))] : [],
      { label: 'As of', value: date },
    ],
  };
}

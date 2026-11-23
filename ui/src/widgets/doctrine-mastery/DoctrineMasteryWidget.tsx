import { countryFromKey } from '../../dashboard/countryKey.ts';
import { useCallback, useEffect, useId, useMemo, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import WidgetState from '../../components/WidgetState';
import { ChipSet } from '../../components/ChipSet';
import { useContextMenu } from '../../components/ContextMenu';
import ProgressMeter from '../../charts/meter/ProgressMeter';
import type { MeterMark } from '../../charts/meter/ProgressMeter';
import { dayLabel } from '../../charts/draw';
import { jumpItem } from '../../charts/jump';
import { useRowTooltips } from '../../charts/useRowTooltips';
import { tooltipText } from '../../charts/tooltip';
import type { TooltipContent } from '../../charts/tooltip';
import type { LiveValues } from '../../charts/live';
import type { WidgetProps } from '../../dashboard/definitions';
import { contentKey, contentSet, resolveOptions, setToken } from '../../dashboard/options';
import { selectedCountryKeys } from '../../dashboard/countrySelection';
import { useDrawnPanel } from '../../dashboard/DrawnPanel';
import { panelFeed } from '../../dashboard/panelFeed';
import { useHeadProduct } from '../../dashboard/headProduct';
import { headState } from '../../dashboard/HeadState';
import { useProduct } from '../../dashboard/products';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import { useSpriteArtwork } from '../../dashboard/useSpriteArtwork';
import { spriteId } from '../../dashboard/spriteArtwork';
import { nameKey, useCountryNamesAt } from '../../dashboard/countryNamesAt';
import { nameReadText } from '../../dashboard/nameReads';
import { activeTracks, choiceName, doctrineIcons, doctrineSprite, folderName, masteryStructure, masteryText, rateText, requirementMet,
  rewardDescription, rewardName, rewardUnlocked, trackKey, trackMarks, trackName, trackNumbers, trackRatio, trackScale, trackTexts, unlockMastery,
  unlockedRewards, validateDoctrineMastery, validateDoctrineRewards } from './doctrineMasteryData';
import type { TrackNumbers } from './doctrineMasteryData';
import { OPTIONS } from './definition';
import './doctrineMastery.css';
import type { DoctrineMasteryReply, DoctrineRewardsReply, MasteryFolder, MasteryTrack } from '../../generated/dashboard';

export default function DoctrineMasteryWidget({ widget, runtime, active, onDiagnostics, setOption, headerSlot }: WidgetProps) {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const options = resolveOptions(OPTIONS, widget.options);
  const tag = selectedCountryKeys(widget.countries, track)[0] ?? null;
  const valueId = useId();
  const root = useRef<HTMLDivElement | null>(null);
  const head = useHeadProduct<DoctrineMasteryReply>(runtime, widget, 'doctrine-mastery', tag ? { country: countryFromKey(tag) } : null,
    { active, validate: validateDoctrineMastery, structure: masteryStructure });
  const painted = useDrawnPanel(root, runtime.clock, panelFeed('doctrine-mastery', head, masteryStructure, widget, runtime), active);
  const offset = track?.axisOffset ?? 0;
  const drawn = head;
  const { data, error, retry } = drawn;
  const attach = useCallback((element: HTMLDivElement | null) => { root.current = element; head.root(element); }, [head.root]);
  const live = drawn.live;
  const paintArt = useCallback((icons: ReadonlyMap<string, string | null>) => {
    for (const img of root.current?.querySelectorAll<HTMLImageElement>('img[data-doctrine-art]') ?? []) {
      const src = icons.get(img.dataset.doctrineArt!);
      if (src && img.getAttribute('src') !== src) img.src = src;
      img.hidden = !src;
    }
  }, []);
  const image = useSpriteArtwork(runtime, track, useMemo(() => doctrineIcons(data).map(doctrineSprite), [data]), active, paintArt);
  const folderImage = useSpriteArtwork(runtime, track, useMemo(() => data?.folders.map(f => doctrineSprite(f.icon)) ?? [], [data]), active);
  const art = (key: string | null | undefined) => image(doctrineSprite(key));
  const artNode = (key: string | null | undefined) => key && <img data-doctrine-art={spriteId(doctrineSprite(key)!)} src={art(key) ?? undefined} hidden={!art(key)} alt="" />;
  const chosen = contentSet(options.folders).map(contentKey);
  const shown = useMemo(() => data?.folders.filter(folder => !chosen.length || chosen.includes(folder.id)) ?? [],
    [data, JSON.stringify(chosen)]);
  const hourShown = () => drawn.shownAt() ?? Math.floor(runtime.clock.getHour());
  const [describing, setDescribing] = useState<{ folder: number; track: number; hour: number } | null>(null);
  const described = useProduct<DoctrineRewardsReply>(runtime.client, track && tag && describing ? { sourceId: track.source.id, revision: track.source.revision,
    product: 'doctrine-rewards', params: { branchId: track.branch.id, country: countryFromKey(tag), folder: describing.folder, track: describing.track, hour: describing.hour } } : null,
  active && describing !== null, validateDoctrineRewards, track?.source);
  const menu = useContextMenu('Doctrine mastery');
  const tracks = useMemo(() => new Map<string, { folder: MasteryFolder; item: MasteryTrack }>(shown.flatMap(folder =>
    folder.tracks.map(item => [trackKey(folder, item), { folder, item }] as const))), [shown]);
  const content = useCallback((key: string): TooltipContent | null => {
    const [kind, ...rest] = key.split('#');
    const found = tracks.get(rest[0]);
    if (!found) return null;
    const { folder, item } = found;
    const inspected = painted.inspected();
    if (inspected === null) return null;
    const numbers = trackNumbers(inspected ? paintedValues(inspected.live) : live.read(), rest[0]);
    const date = dayLabel(inspected ? inspected.date.through : hourShown());
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
    return trackTooltip(folder, item, numbers, date);
  }, [tracks, described.data, head.hour, live, painted, offset]);
  const spoken = (key: string) => { const shown = content(key); return shown ? tooltipText(shown) : ''; };
  const refresh = useMemo(() => {
    const pipeline = head.pipeline, pipe = head.followedWidget;
    return pipeline && pipe !== undefined ? (changed: () => void) => { const installed = pipeline.onInstall(pipe, changed), dated = pipeline.onDate(pipe, changed); return () => { installed(); dated(); }; } : undefined;
  }, [head.pipeline, head.followedWidget]);
  const tips = useRowTooltips<string>({ owner: `doctrine-mastery:${widget.id}`, active, content, refresh, revision: `${head.hour}:${described.data ? 'described' : ''}`,
    onMenu: (_, x, y) => { if (track) menu.open(x, y, [jumpItem(runtime, track.id, hourShown())]); } });
  useEffect(() => {
    const [kind, key] = tips.hovered?.split('#') ?? [];
    const [folder, item] = (key ?? '').split('|').map(Number);
    setDescribing(kind === 'mark' && Number.isSafeInteger(folder) && Number.isSafeInteger(item) ? { folder: folder!, track: item!, hour: hourShown() } : null);
  }, [tips.hovered]);
  useDiagnostics(onDiagnostics, head.data ? [...head.data.warnings, ...(described.data?.warnings ?? [])] : null);
  const nameHour = drawn.at ?? drawn.hour;
  const country = useCountryNamesAt(runtime.client, track, useMemo(() => tag && nameHour !== null && data && !data.folders.length ? [{ hour: nameHour, tag }] : [],
    [tag, nameHour, data]), active);
  const countryName = tag && nameHour !== null ? nameReadText(country.get(nameKey(nameHour, tag))) : undefined;
  const pill = headerSlot && head.data && createPortal(<span className="pill">{activeTracks(head.data.folders)} tracks active</span>, headerSlot);
  if (!track) return <WidgetState kind="empty" icon="flask" message="Load a campaign to see doctrines." />;
  if (!tag) return <WidgetState kind="empty" icon="flask" message="Choose a country in the widget's header." />;
  const shared = headState(drawn);
  if (shared) return <div ref={attach} className="doctrine-mastery">{shared}</div>;
  if (head.read === 'off' && !data) return <WidgetState kind="unavailable" message="Choose another time in widget settings." />;
  if (error) return <WidgetState kind="error" failure={error} onRetry={retry} />;
  if (!data) return <div ref={attach} className="doctrine-mastery"><WidgetState kind="loading" label="Loading doctrines…" /></div>;
  const chips = data.folders.map(folder => ({ value: folder.id, label: folderName(folder), icon: folderImage(doctrineSprite(folder.icon)) }));
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
        <h3 className="doctrine-mastery-heading">{artNode(folder.grand?.icon)}
          <span>{folderName(folder)}</span>
          <small>{choiceName(folder.grand, 'No grand doctrine chosen')}</small></h3>
        <div className="doctrine-mastery-tracks">{folder.tracks.map(item => {
          const key = trackKey(folder, item);
          const marks = trackMarks(item, true);
          const numbers = trackNumbers(values, key);
          const texts = trackTexts(numbers);
          const ratio = trackRatio(item, numbers.mastery);
          return <div key={key} data-track={key} className={`doctrine-mastery-track${item.sub === null ? ' is-empty' : ''}`} {...tips.rowProps(`track#${key}`)}>
            {tips.hovered === `track#${key}` && <span id={tips.descriptionId(`track#${key}`)} className="sr-only">{spoken(`track#${key}`)}</span>}
            <div className="doctrine-mastery-sub">{artNode(item.sub?.icon)}
              <span>{choiceName(item.sub, 'No subdoctrine chosen')}</span></div>
            <div className="doctrine-mastery-meter">
              <ProgressMeter orientation="vertical" value={ratio} state={item.sub === null ? 'idle' : marks.every(mark => mark.reached) && marks.length ? 'done' : 'live'}
                label={`${trackName(item)} mastery`} marks={marks}
                markProps={(mark: MeterMark) => tips.rowProps(`mark#${key}#${mark.id.split(':').pop()}`)} />
            </div>
            <dl className="doctrine-mastery-values">
              <div><dt>Mastery</dt><dd id={`${valueId}-${key}-mastery`} data-paint="mastery">{texts.mastery}</dd></div>
              <div><dt>Daily</dt><dd id={`${valueId}-${key}-daily`} data-paint="daily">{texts.daily}</dd></div>
              <div><dt>Banked</dt><dd id={`${valueId}-${key}-bank`} data-paint="bank">{texts.bank}</dd></div>
            </dl>
          </div>;
        })}</div>
      </section>)}</div>}
    {shown.length > 0 && <table className="sr-only"><caption>Doctrine mastery</caption>
      <thead><tr><th scope="col">Folder</th><th scope="col">Track</th><th scope="col">Mastery</th><th scope="col">Daily</th><th scope="col">Banked</th></tr></thead>
      <tbody>{shown.flatMap(folder => folder.tracks.map(item => {
        const key = trackKey(folder, item);
        return <tr key={key}><td>{folderName(folder)}</td><th scope="row">{trackName(item)}</th>
          {['mastery', 'daily', 'bank'].map(field => <td key={field} aria-labelledby={`${valueId}-${key}-${field}`} />)}
        </tr>;
      }))}</tbody></table>}
    {menu.menu}
  </div>;
}

function paintedValues(values: ReadonlyMap<string, number | string>): LiveValues<number> {
  return {
    get: key => { const value = values.get(key); return typeof value === 'number' ? value : undefined; },
    missing: key => { const value = values.get(key); return typeof value === 'string' ? value : 'Not recorded'; },
  };
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

import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import WidgetState from '../../components/WidgetState';
import { ChipSet } from '../../components/ChipSet';
import { useContextMenu } from '../../components/ContextMenu';
import ProgressMeter from '../../charts/meter/ProgressMeter';
import { PreparedKey } from '../../charts/PreparedKey';
import { dayLabel } from '../../charts/draw';
import { jumpItem } from '../../charts/jump';
import { useRowTooltips } from '../../charts/useRowTooltips';
import { tooltipText } from '../../charts/tooltip';
import type { TooltipContent } from '../../charts/tooltip';
import type { WidgetProps } from '../../dashboard/definitions';
import { resolveWindow } from '../../dashboard/model';
import { contentKey, contentSet, setToken } from '../../dashboard/options';
import { usePlaybackHour } from '../../dashboard/playback';
import { pendingReplyAt, preparedPrefix } from '../../dashboard/preparedPrefix';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import { useSpriteArtwork } from '../../dashboard/useSpriteArtwork';
import { nameKey, useCountryNamesAt } from '../../dashboard/countryNamesAt';
import { activeTracks, dailyText, doctrineIcons, doctrineSprite, doctrinesAt, masteryText, requirementMet, rewardKey, rewardName, rewardUnlocked, trackMarks, trackName, folderName, trackRatio, trackScale, unlockedRewards, unlockMastery, rateText } from './doctrineMasteryData';
import type { TrackState } from './doctrineMasteryData';
import type { MeterMark } from '../../charts/meter/ProgressMeter';
import { useDoctrineMastery, useRewardDescriptions } from './useDoctrineMastery';
import './doctrineMastery.css';

/** Each doctrine track's mastery against its reward thresholds, filling downward as the
 * game's track does, at the read behind the hour shown. */
export default function DoctrineMasteryWidget({ widget, runtime, active, onDiagnostics, setOption, headerSlot }: WidgetProps) {
  usePlaybackHour(runtime.clock, active && widget.time.mode === 'follow');
  const anchor = useRef<HTMLDivElement>(null);
  const { track, tag, options, data, error, retry } = useDoctrineMastery(widget, runtime, active);
  const image = useSpriteArtwork(runtime, track, useMemo(() => doctrineIcons(data).map(doctrineSprite), [data]), active);
  const art = (key: string | null | undefined) => image(doctrineSprite(key));
  const window = track && resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour());
  const hour = Math.floor(window?.lastHour ?? track?.branch.lastHour ?? 0);
  const at = useMemo(() => data && tag ? doctrinesAt(data, tag, hour) : null, [data, tag, hour]);
  const chosen = contentSet(options.folders).map(contentKey);
  const shown = useMemo(() => at?.folders.filter(folder => !chosen.length || chosen.includes(folder.folder.id)) ?? [],
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [at, JSON.stringify(chosen)]);
  const readHour = at?.readHour;
  const readDate = readHour === undefined ? null : dayLabel(readHour);
  const [describing, setDescribing] = useState(false);
  const described = useRewardDescriptions(runtime, track, tag, readHour, active && describing);
  const menu = useContextMenu('Doctrine mastery');
  const tracks = useMemo(() => new Map<string, TrackState>(shown.flatMap(folder => folder.tracks.map(item => [`${item.folder}|${item.track.id}`, item] as const))), [shown]);
  const content = useCallback((key: string): TooltipContent | null => {
    const [kind, ...rest] = key.split('#');
    const item = tracks.get(rest[0]);
    if (!item) return null;
    if (kind === 'mark') {
      const index = Number(rest[1]);
      const reward = item.subdoctrine?.rewards?.[index];
      if (!reward || !item.subdoctrine) return null;
      const description = described?.get(rewardKey(item.folder, item.track.id, item.subdoctrine.id, index)) ?? reward.description;
      const needed = unlockMastery(item)[index];
      const unlocked = rewardUnlocked(item, index), met = requirementMet(item, index);
      const missing = item.mastery === null || needed == null ? null : needed - BigInt(item.mastery);
      return { title: rewardName(reward), rows: [{ label: 'Mastery required', value: needed == null ? 'Not recorded' : masteryText(needed) },
        { label: 'Unlocked', value: unlocked === null ? 'Not recorded' : unlocked ? 'Yes' : 'No' },
        { label: 'Mastery', value: met === null || missing === null ? 'Not recorded' : met ? 'Requirement met' : `${masteryText(missing)} to go` }],
        ...description ? { description } : {} };
    }
    return trackTooltip(item, readDate);
  }, [tracks, readDate, described]);
  const tips = useRowTooltips<string>({ owner: `doctrine-mastery:${widget.id}`, active, content, revision: `${readHour}:${described ? 'described' : ''}`,
    onMenu: (_, x, y) => { if (track && readHour !== undefined) menu.open(x, y, [jumpItem(runtime, track.id, readHour)]); } });
  // A reward's own values at the read load while a reward is hovered.
  useEffect(() => setDescribing(tips.hovered?.startsWith('mark#') ?? false), [tips.hovered]);
  useDiagnostics(onDiagnostics, data?.warnings ?? null);
  const country = useCountryNamesAt(runtime.client, track, useMemo(() => tag && readHour !== undefined ? [{ hour: readHour, tag }] : [], [tag, readHour]), active);
  const countryName = tag && readHour !== undefined ? country.get(nameKey(readHour, tag)) : undefined;
  const pill = headerSlot && at && createPortal(<span className="pill">{activeTracks(at.folders)} tracks active</span>, headerSlot);
  if (!track) return <WidgetState kind="empty" icon="flask" message="Load a campaign to see doctrines." />;
  if (!tag) return <WidgetState kind="empty" icon="flask" message="Choose a country in the widget's header." />;
  if (!window) return <WidgetState kind="unavailable" message="Choose another time in widget settings." />;
  if (error) return <WidgetState kind="error" detail={error} onRetry={retry} />;
  if (!data) return <WidgetState kind="loading" label="Loading doctrines…" />;
  if (pendingReplyAt(data, hour)) return <WidgetState kind="loading" label="This range is still being prepared." />;
  if (!at) return <WidgetState kind="unavailable" message="Doctrines were not recorded at this time." />;
  const prefix = preparedPrefix(data);
  const chips = data.folders.map(folder => ({ value: folder.id, label: folderName(folder), icon: art(folder.icon) }));
  const toggle = (id: string) => {
    const next = chosen.includes(id) ? chosen.filter(item => item !== id) : [...chosen, id];
    setOption?.('folders', next.length === chips.length ? '' : next.map(setToken).join(' '));
  };
  return <div ref={anchor} className="doctrine-mastery">
    {pill}
    <PreparedKey pending={prefix?.pending} approximate={prefix?.approximate} />
    {chips.length > 1 && setOption && <div className="log-chips doctrine-mastery-chips"><ChipSet label="Doctrine folders" chips={chips}
      selected={chosen.length ? chosen : chips.map(chip => chip.value)} max={chips.length} onToggle={toggle} /></div>}
    {!at.folders.length ? <WidgetState kind="empty" message={`${countryName ?? 'This country'} has no doctrines in this recording.`} />
      : <div className="doctrine-mastery-folders themed-scrollbar">{shown.map(folder => <section key={folder.folder.id} className="doctrine-mastery-folder" aria-label={folderName(folder.folder)}>
        <h3 className="doctrine-mastery-heading">{art(folder.grand?.icon) && <img src={art(folder.grand?.icon)!} alt="" />}
          <span>{folderName(folder.folder)}</span>
          <small>{folder.grand?.name ?? (folder.grandToken === null ? 'No grand doctrine chosen' : 'Doctrine name unavailable')}</small></h3>
        <div className="doctrine-mastery-tracks">{folder.tracks.map(item => {
          const key = `${item.folder}|${item.track.id}`;
          const marks = trackMarks(item, true);
          return <div key={key} className={`doctrine-mastery-track${item.subToken === null ? ' is-empty' : ''}`} {...tips.rowProps(`track#${key}`)}>
            {tips.hovered === `track#${key}` && <span id={tips.descriptionId(`track#${key}`)} className="sr-only">{tooltipText(trackTooltip(item, readDate))}</span>}
            <div className="doctrine-mastery-sub">{art(item.subdoctrine?.icon) && <img src={art(item.subdoctrine?.icon)!} alt="" />}
              <span>{item.subdoctrine?.name ?? (item.subToken === null ? 'No subdoctrine chosen' : 'Subdoctrine name unavailable')}</span></div>
            <div className="doctrine-mastery-meter">
              <ProgressMeter orientation="vertical" value={trackRatio(item)} state={item.subToken === null ? 'idle' : marks.every(mark => mark.reached) && marks.length ? 'done' : 'live'}
                label={`${trackName(item.track)} mastery`} marks={marks}
                markProps={(mark: MeterMark) => tips.rowProps(`mark#${key}#${mark.id.split(':').pop()}`)} />
            </div>
            <dl className="doctrine-mastery-values">
              <div><dt>Mastery</dt><dd>{item.mastery === null ? 'Not recorded' : masteryText(item.mastery)}</dd></div>
              {dailyText(item.effective) && <div><dt>Daily</dt><dd>{dailyText(item.effective)}</dd></div>}
              {item.bank !== null && item.bank !== '0' && <div><dt>Banked</dt><dd>{masteryText(item.bank)}</dd></div>}
            </dl>
          </div>;
        })}</div>
      </section>)}</div>}
    {menu.menu}
  </div>;
}

function trackTooltip(item: TrackState, readDate: string | null): TooltipContent {
  const scale = trackScale(item);
  const unlocked = unlockedRewards(item);
  return {
    title: item.subdoctrine?.name ?? trackName(item.track),
    rows: [
      { label: 'Track', value: trackName(item.track) },
      { label: 'Mastery', value: item.mastery === null ? 'Not recorded' : `${masteryText(item.mastery)}${scale === null ? '' : ` of ${masteryText(scale)}`}` },
      { label: 'Banked mastery', value: item.bank === null ? 'Not recorded' : masteryText(item.bank) },
      { label: 'Daily gain', value: item.effective === null ? 'Not recorded' : rateText(item.effective) },
      { label: 'Units’ daily gain', value: item.daily === null ? 'Not recorded' : rateText(item.daily) },
      { label: 'Rewards unlocked', value: item.rewards ?? 'Not recorded' },
      ...unlocked.map(reward => ({ label: rewardName(reward), indent: true })),
      ...item.completed.length ? [{ label: 'Completed subdoctrines' }, ...item.completed.map(sub => ({ label: sub.name ?? 'Subdoctrine name unavailable', indent: true }))] : [],
      ...readDate ? [{ label: 'As of', value: readDate }] : [],
    ],
  };
}

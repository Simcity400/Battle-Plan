import { useCallback, useRef, useState } from 'react';
import WidgetState from '../../components/WidgetState';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import { selectedCountryKeys } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { openedOn } from '../../dashboard/gameWindows';
import { useProduct } from '../../dashboard/products';
import { useGate } from '../../dashboard/regionGate';
import { productRequest } from '../../dashboard/request.ts';
import { DetailWindowHost } from '../../families/host/index.ts';
import DeclaredMeterWidget from '../../families/meter/DeclaredMeterWidget.tsx';
import type { DoctrineRewardsReply } from '../../generated/dashboard';
import {
  masteryText,
  rewardName,
  validateDoctrineRewards,
} from './doctrineMasteryData.ts';
import { doctrineMasteryDeclaration, type DoctrineTrackMeta } from './doctrineMasteryProjection.ts';
import '../../components/Dossier.css';

export default function DoctrineMasteryView(props: WidgetProps) {
  const { widget, runtime, active } = props;
  const hostRef = useRef<HTMLDivElement>(null);
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const tag = selectedCountryKeys(widget.countries, track)[0] ?? null;
  const gate = useGate(runtime, widget);

  const [opened, setOpened] = useState<{
    readonly on: string;
    readonly key: string;
    readonly meta: DoctrineTrackMeta;
    readonly hour: number;
  } | null>(null);

  const onActivate = useCallback((key: string, rawMeta: unknown, hour?: number) => {
    if (!track || !tag || !rawMeta || typeof rawMeta !== 'object') return;
    const meta = rawMeta as DoctrineTrackMeta;
    if (!meta.rewards?.length || meta.subId === null) return;
    setOpened({ on: openedOn(track, tag), key, meta, hour: hour ?? track.branch.lastHour });
  }, [track, tag]);

  const shown = track && tag && opened?.on === openedOn(track, tag) ? opened : null;
  const recordedHour = shown ? gate.recorded(shown.hour) : null;
  const rewardsReq = track && tag && shown && recordedHour !== null
    ? productRequest({
        sourceId: track.source.id,
        revision: track.source.revision,
        product: 'doctrine-rewards',
        params: {
          branchId: track.branch.id,
          country: countryFromKey(tag),
          folder: shown.meta.folderOrdinal,
          track: shown.meta.trackOrdinal,
          hour: recordedHour,
        },
      })
    : null;
  const rewards = useProduct<DoctrineRewardsReply>(runtime.client, rewardsReq, active && shown !== null, validateDoctrineRewards, track?.source);

  let cumulative: bigint | null = 0n;
  const thresholds = shown
    ? shown.meta.rewards.map(item => (cumulative = cumulative === null || item.threshold === null ? null : cumulative + BigInt(item.threshold)))
    : [];

  return <div ref={hostRef} className="mf-frame">
    <DeclaredMeterWidget
      {...props}
      declaration={doctrineMasteryDeclaration}
      selectedKey={shown?.key ?? null}
      onActivate={onActivate}
      extraWarnings={rewards.data?.warnings}
    />
    <DetailWindowHost
      open={Boolean(shown)}
      subject={shown ? `${tag}:${shown.key}` : null}
      hour={shown?.hour ?? null}
      title={shown?.meta.title ?? 'Doctrine steps'}
      anchor={hostRef}
      onClose={() => setOpened(null)}
    >
      {shown && (
        <div className="dossier">
          <div className="dossier-body">
            <p className="dossier-missing">{shown.meta.folderTitle}</p>
            {rewards.error && <WidgetState kind="error" scale="section" failure={rewards.error} onRetry={rewards.retry} />}
            <section className="dossier-section">
              <h4>Steps ({shown.meta.unlocked} of {shown.meta.rewards.length} unlocked)</h4>
              {shown.meta.rewards.map((reward, index) => {
                const needed = thresholds[index];
                const unlocked = index < shown.meta.unlocked;
                const desc = rewards.data && rewards.data.sub === shown.meta.subId
                  ? rewards.data.rewards[index]?.description ?? null
                  : null;
                return <div key={`${reward.id}:${index}`} className="dossier-section">
                  <strong>{index + 1}. {rewardName(reward)}</strong>
                  {' · '}
                  <span>{unlocked ? 'Unlocked' : 'Locked'}</span>
                  {needed !== null && needed !== undefined && <span> · Mastery {masteryText(needed)}</span>}
                  {desc && <p>{desc}</p>}
                </div>;
              })}
            </section>
          </div>
        </div>
      )}
    </DetailWindowHost>
  </div>;
}

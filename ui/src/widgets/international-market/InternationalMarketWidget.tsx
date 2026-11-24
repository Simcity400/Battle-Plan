import { useCallback, useRef, useState } from 'react';
import { useDrill } from '../../charts/drill/useDrill';
import { dayLabel } from '../../charts/draw';
import type { WidgetProps } from '../../dashboard/definitions';
import { partyTags } from '../../dashboard/market.ts';
import MarketDrill from '../../dashboard/MarketDrill';
import { useRegion, useRegionHour } from '../../dashboard/widgetTime.ts';
import { DetailWindowHost } from '../../families/host/DetailWindowHost.tsx';
import DeclaredTableWidget from '../../families/table/DeclaredTableWidget.tsx';
import { DETAILS } from './definition';
import { internationalMarketDeclaration } from './internationalMarketProjection.ts';

interface MarketPick {
  readonly owner: string | null;
  readonly title: string;
  readonly tags: string[];
  readonly selected: string | null;
}

export default function InternationalMarketWidget(props: WidgetProps) {
  const { widget, runtime, active } = props;
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const [drillShown, setDrillShown] = useState<number | undefined>();
  const anchor = useRef<HTMLDivElement>(null);
  const drillControl = useDrill(runtime, useRegion(widget));
  const drill = drillControl.state;
  const dispatch = drillControl.dispatch;
  const pickOwner = track ? JSON.stringify([track.source.id, track.source.revision, track.branch.id]) : null;
  const [pick, setPick] = useState<MarketPick | null>(null);
  const picked = pick && pick.owner === pickOwner ? pick : null;
  const drillTime = drillControl.region.action;
  const drillRegion = useRegion(widget, drillTime);
  const drillOpen = drill.breakdown !== null && !!picked;
  const drillAt = useRegionHour(runtime, drillRegion, active && drillOpen);

  const onActivate = useCallback((_key: string, meta?: unknown, hour?: number) => {
    const info = meta as { title?: string; tags?: (string | null)[]; selected?: string | null } | undefined;
    if (!info?.tags?.length) return;
    const tagsOf = partyTags(...info.tags);
    if (!tagsOf) return;
    const atHour = hour ?? track?.branch.lastHour ?? 0;
    setPick({ owner: pickOwner, title: info.title ?? DETAILS.label, tags: tagsOf, selected: info.selected ?? null });
    dispatch({ type: 'open', breakdown: DETAILS.id, hour: atHour, follow: drillControl.followsAt(atHour) });
  }, [pickOwner, dispatch, drillControl, track?.branch.lastHour]);

  const drillDay = drillShown ?? drillAt;

  return <div ref={anchor} className="international-market">
    <DeclaredTableWidget
      declaration={internationalMarketDeclaration}
      {...props}
      onActivate={onActivate}
    />
    <DetailWindowHost
      open={drillOpen}
      subject={picked?.selected ?? null}
      hour={drillDay}
      title={picked?.title ?? DETAILS.label}
      day={drillDay == null ? '' : dayLabel(drillDay)}
      anchor={anchor}
      onClose={() => dispatch({ type: 'close' })}
    >
      {picked && <MarketDrill
        runtime={runtime}
        widget={widget}
        time={drillTime}
        tags={picked.tags}
        selected={picked.selected}
        active={active}
        onAnswered={setDrillShown}
      />}
    </DetailWindowHost>
  </div>;
}

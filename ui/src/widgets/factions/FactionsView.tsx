import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { WidgetProps } from '../../dashboard/definitions';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { useCampaignSettings } from '../../dashboard/campaignSettings';
import type { BoundTrack, WidgetSpec } from '../../dashboard/model';
import type { FactionGroup } from '../../generated/dashboard';
import type { InstalledAction } from '../../families/contract.ts';
import DeclaredFamilyWidget from '../../families/host/DeclaredFamilyWidget';
import { DetailWindowHost } from '../../families/host/DetailWindowHost';
import { timelineHostAdapter } from '../../families/timeline/hostAdapter.ts';
import { formatDateOnly } from '../../families/valueFormat.ts';
import FactionDetails from './FactionDetails';
import { factionLabel, factionViews } from './factionsData';
import { factionsDeclarationWithStyles, type FactionLaneMeta } from './factionsProjection';

interface Opened {
  readonly factionId: string;
  readonly hour: number;
}

function recordedName(faction: FactionGroup, hour: number | undefined, own: string | undefined): { name: string | null; reason: string | null } {
  if (own) return { name: own, reason: null };
  const run = hour === undefined ? faction.names.at(-1) : faction.names.find(item => item.firstHour <= hour && hour <= item.lastHour);
  return run ? { name: run.name, reason: run.reason } : { name: faction.name, reason: faction.nameReason };
}

function pinnedAt(widget: WidgetSpec, track: BoundTrack, hour: number): WidgetSpec {
  const offset = Math.max(0, hour - track.branch.firstHour);
  return { ...widget, time: { mode: 'pin', windows: { [track.id]: { firstOffset: offset, lastOffset: offset } } } };
}

export default function FactionsView(props: WidgetProps) {
  const { runtime, widget, active } = props;
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const settings = useCampaignSettings();
  const campaignKey = track?.source.metadata.campaignKey ?? '';
  const styles = settings.campaigns[campaignKey]?.factions;
  const declaration = useMemo(() => factionsDeclarationWithStyles(styles ?? {}, settings.shared), [styles, settings.shared]);
  const [opened, setOpened] = useState<Opened | null>(null);
  const anchor = useRef<HTMLDivElement>(null);
  useEffect(() => { setOpened(null); }, [track?.source.id, track?.branch.id]);
  const groups = useCountryGroups(runtime, track, active && opened !== null);
  const data = groups.data;
  const views = useMemo(() => data ? factionViews(data, (faction, hour) => recordedName(faction, hour, styles?.[faction.id]?.name), id => styles?.[id]?.colour ?? '') : [], [data, styles]);
  const view = opened ? views.find(item => item.id === opened.factionId) ?? null : null;
  const pinned = useMemo(() => (opened && track ? pinnedAt(widget, track, opened.hour) : widget), [opened, track, widget]);

  const onActivate = useCallback((action: InstalledAction) => {
    const meta = action.meta as FactionLaneMeta | undefined;
    if (meta?.factionId) setOpened({ factionId: meta.factionId, hour: action.hour });
  }, []);

  return (
    <div ref={anchor} style={{ display: 'contents' }}>
      <DeclaredFamilyWidget
        {...props}
        adapter={timelineHostAdapter}
        declaration={declaration}
        selectedKey={opened ? JSON.stringify(['faction', opened.factionId]) : null}
        onActivate={onActivate}
      />
      <DetailWindowHost
        open={opened !== null}
        subject={opened?.factionId ?? null}
        hour={opened?.hour ?? null}
        title={view ? factionLabel(view, opened!.hour) : 'Faction'}
        day={opened ? formatDateOnly(opened.hour) : undefined}
        anchor={anchor}
        onClose={() => setOpened(null)}
      >
        {view && track && opened ? (
          <FactionDetails runtime={runtime} widget={pinned} track={track} view={view} hour={opened.hour} active={active} />
        ) : null}
      </DetailWindowHost>
    </div>
  );
}

import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { WidgetProps } from '../../dashboard/definitions';
import { useFactionColours } from '../../dashboard/FactionColours';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import type { InstalledAction } from '../../families/contract.ts';
import DeclaredFamilyWidget from '../../families/host/DeclaredFamilyWidget';
import { DetailWindowHost } from '../../families/host/DetailWindowHost';
import { timelineHostAdapter } from '../../families/timeline/hostAdapter.ts';
import { formatDateOnly } from '../../families/valueFormat.ts';
import FactionDrill from './FactionDrill';
import { factionGaps, factionLabel, factionViews } from './factionsData';
import { factionsDeclarationWith, type FactionLaneMeta } from './factionsProjection';
import './factions.css';

interface Opened {
  readonly factionId: string;
  readonly hour: number;
}

export default function FactionsView(props: WidgetProps) {
  const { runtime, widget, active } = props;
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const factionColours = useFactionColours(runtime.client, track?.source);
  const colours = useMemo(() => Object.fromEntries(factionColours.colours), [factionColours.colours]);
  const names = useMemo(() => Object.fromEntries(factionColours.displayNames), [factionColours.displayNames]);
  const declaration = useMemo(() => factionsDeclarationWith(colours, names), [colours, names]);
  const [opened, setOpened] = useState<Opened | null>(null);
  const anchor = useRef<HTMLDivElement>(null);
  useEffect(() => { setOpened(null); }, [track?.source.id, track?.branch.id]);
  const groups = useCountryGroups(runtime, track, active && opened !== null);
  const data = groups.data;
  const views = useMemo(() => data ? factionViews(data, factionColours.names, id => colours[id] ?? '') : [], [data, factionColours.names, colours]);
  const view = opened ? views.find(item => item.id === opened.factionId) ?? null : null;

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
        extraWarnings={factionColours.warnings}
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
        {view && track && data && opened ? (
          <FactionDrill
            runtime={runtime}
            widget={widget}
            track={track}
            view={view}
            names={data.factions.find(faction => faction.id === view.id)?.names ?? []}
            at={opened.hour}
            active={active}
            gaps={factionGaps(data)}
            onBack={() => setOpened(null)}
          />
        ) : null}
      </DetailWindowHost>
    </div>
  );
}

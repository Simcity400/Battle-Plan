import { useMemo } from 'react';
import { useChartTokens } from '../../charts/useChartTokens';
import type { WidgetProps } from '../../dashboard/definitions';
import { useFactionColours } from '../../dashboard/FactionColours';
import { barsHostAdapter } from '../../families/bars/hostAdapter.ts';
import { DeclaredFamilyWidget } from '../../families/host/index.ts';
import { airSuperiorityRegionDeclaration } from './airSuperiorityProjection.ts';

export default function AirSuperiorityRegionWidget(props: WidgetProps) {
  const { widget, runtime } = props;
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const tokens = useChartTokens();
  const factions = useFactionColours(runtime.client, track?.source);

  const factionColours = useMemo(() => Object.fromEntries(factions.colours), [factions.colours]);
  const factionNames = useMemo(() => Object.fromEntries(factions.displayNames), [factions.displayNames]);
  const extraWarnings = useMemo(() => {
    const list = [...factions.warnings, ...(factions.error ? [String(factions.error)] : [])];
    return list.length ? list : undefined;
  }, [factions.warnings, factions.error]);

  const declaration = useMemo(() => ({
    ...airSuperiorityRegionDeclaration,
    projectionOptions: (ctx: Parameters<typeof airSuperiorityRegionDeclaration.projectionOptions>[0]) => ({
      ...airSuperiorityRegionDeclaration.projectionOptions(ctx),
      factionColours,
      factionNames,
      enemyColor: tokens.error,
      mutedColor: tokens.muted,
    }),
  }), [factionColours, factionNames, tokens.error, tokens.muted]);

  return (
    <DeclaredFamilyWidget
      {...props}
      adapter={barsHostAdapter}
      declaration={declaration}
      extraWarnings={extraWarnings}
    />
  );
}

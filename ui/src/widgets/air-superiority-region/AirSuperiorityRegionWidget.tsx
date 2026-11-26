import { useMemo } from 'react';
import { useChartTokens } from '../../charts/useChartTokens';
import type { FamilyViewProps } from '../../dashboard/definitions';
import { barsHostAdapter } from '../../families/bars/hostAdapter.ts';
import { DeclaredFamilyWidget } from '../../families/host/index.ts';
import { airSuperiorityRegionDeclaration } from './airSuperiorityProjection.ts';

export default function AirSuperiorityRegionWidget(props: FamilyViewProps) {
  const tokens = useChartTokens();

  const declaration = useMemo(() => ({
    ...airSuperiorityRegionDeclaration,
    projectionOptions: (...[ctx, extra]: Parameters<typeof airSuperiorityRegionDeclaration.projectionOptions>) => ({
      ...airSuperiorityRegionDeclaration.projectionOptions(ctx, extra),
      enemyColor: tokens.error,
      mutedColor: tokens.muted,
    }),
  }), [tokens.error, tokens.muted]);

  return (
    <DeclaredFamilyWidget
      {...props}
      adapter={barsHostAdapter}
      declaration={declaration}
    />
  );
}

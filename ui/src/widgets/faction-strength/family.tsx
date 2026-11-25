import { useMemo } from 'react';
import { useCampaignSettings } from '../../dashboard/campaignSettings.ts';
import type { FamilyViewProps, FamilyViewRenderer } from '../../dashboard/definitions';
import { FamilyWidget } from '../../families/host/registry.tsx';
import type { TimeChartWidgetDeclaration } from '../../families/time-chart/declaration.ts';
import { withFactionTotalsStyles, type FactionTotalsLinesOptions } from '../../families/time-chart/factionTotalsLines.ts';

function FactionStrengthLinesFamilyWidget(props: FamilyViewProps) {
  const { runtime, widget, manifestDeclaration } = props;
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const settings = useCampaignSettings();
  const campaignKey = track?.source.metadata.campaignKey ?? '';
  const styles = settings.campaigns[campaignKey]?.factions;
  const declaration = useMemo(() => (
    manifestDeclaration
      ? withFactionTotalsStyles(
          manifestDeclaration as TimeChartWidgetDeclaration<FactionTotalsLinesOptions>,
          styles,
          settings.shared,
        )
      : null
  ), [manifestDeclaration, styles, settings.shared]);

  return declaration ? <FamilyWidget {...props} family={declaration.family} declaration={declaration} /> : null;
}

export default { render: FactionStrengthLinesFamilyWidget } satisfies FamilyViewRenderer;

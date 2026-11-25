import { useMemo } from 'react';
import { useCampaignSettings } from '../../dashboard/campaignSettings.ts';
import { useFactionColours } from '../../dashboard/FactionColours.tsx';
import type { FamilyViewProps, FamilyViewRenderer } from '../../dashboard/definitions';
import { FamilyWidget } from '../../families/host/registry.tsx';
import { territoryLinesDeclarationWithStyles } from './territoryLinesProjection.ts';

function TerritoryLinesFamilyWidget(props: FamilyViewProps) {
  const { runtime, widget, manifestDeclaration } = props;
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const settings = useCampaignSettings();
  const campaignKey = track?.source.metadata.campaignKey ?? '';
  const styles = settings.campaigns[campaignKey]?.factions;
  const colours = useFactionColours(runtime.client, track?.source);
  const identityText = JSON.stringify(colours.factions.map(faction => [faction.id, colours.identity(faction.id)]));
  const identities = useMemo(() => new Map(JSON.parse(identityText) as [string, string | null][]), [identityText]);
  const declaration = useMemo(() => (
    manifestDeclaration
      ? territoryLinesDeclarationWithStyles(styles, settings.shared, identities)
      : null
  ), [manifestDeclaration, styles, settings.shared, identities]);

  return declaration ? <FamilyWidget {...props} family={declaration.family} declaration={declaration} /> : null;
}

export default { render: TerritoryLinesFamilyWidget } satisfies FamilyViewRenderer;

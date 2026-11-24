import { useCampaignGate } from '../../dashboard/regionGate';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import type { OptionChoices, WidgetRenderer, WidgetRuntime } from '../../dashboard/definitions';
import type { WidgetSpec } from '../../dashboard/model';
import { setToken } from '../../dashboard/options';
import { selectedCountryKeys } from '../../dashboard/countrySelection';
import { useProduct } from '../../dashboard/products';
import { useSpriteArtwork } from '../../dashboard/useSpriteArtwork';
import DoctrineMasteryView from './DoctrineMasteryView';
import { doctrineSprite, folderName, validateDoctrineMastery } from './doctrineMasteryData';
import type { DoctrineMasteryReply } from '../../generated/dashboard';
import { productRequest } from '../../dashboard/request.ts';

function useDoctrineChoices(widget: WidgetSpec, runtime: WidgetRuntime): OptionChoices {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const tag = selectedCountryKeys(widget.countries, track)[0] ?? null;
  const end = useCampaignGate(runtime, widget).hour;
  const { data } = useProduct<DoctrineMasteryReply>(runtime.client, track && tag && end !== null ? productRequest({ sourceId: track.source.id, revision: track.source.revision, product: 'doctrine-mastery',
    params: { branchId: track.branch.id, country: countryFromKey(tag), hour: end } }) : null, true, validateDoctrineMastery, track?.source);
  const image = useSpriteArtwork(runtime, track, (data?.folders ?? []).map(folder => doctrineSprite(folder.icon)), true);
  return { folders: (data?.folders ?? []).map(folder => ({ value: setToken(folder.id), label: folderName(folder), icon: image(doctrineSprite(folder.icon)) })) };
}

export default { render: DoctrineMasteryView, useOptionChoices: useDoctrineChoices } satisfies WidgetRenderer;

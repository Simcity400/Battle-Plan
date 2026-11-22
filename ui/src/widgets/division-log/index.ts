import type { WidgetRenderer } from '../../dashboard/definitions';
import { useProduct } from '../../dashboard/products';
import { templateToken, validateDivisionManifest } from './divisionLogData';
import DivisionLogWidget from './DivisionLogWidget';
import type { DivisionLogManifestReply } from '../../generated/dashboard';
import { productRequest } from '../../dashboard/request.ts';

export default { render: DivisionLogWidget, useOptionChoices: (widget, runtime) => {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const { data } = useProduct<DivisionLogManifestReply>(runtime.client, track ? productRequest({ sourceId: track.source.id, revision: track.source.revision,
    product: 'division-log-manifest', params: { branchId: track.branch.id } }) : null, true, validateDivisionManifest);
  return { templates: (data?.templates ?? []).map(template => ({ value: templateToken(track!.source.metadata.campaignKey, track!.branch.id, template.id),
    label: template.name ?? `Unnamed template (${template.id})` })) };
} } satisfies WidgetRenderer;

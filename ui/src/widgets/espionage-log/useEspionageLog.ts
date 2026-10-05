import type { OptionChoices, WidgetRuntime } from '../../dashboard/definitions';
import type { WidgetSpec } from '../../dashboard/model';
import { useProduct } from '../../dashboard/products';
import type { EspionageLogReply } from '../../generated/dashboard';
import { operationChoices, validateEspionageLogReply } from './espionageData';

export function useEspionageLog(widget: WidgetSpec, runtime: WidgetRuntime, active: boolean) {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const request = track ? { sourceId: track.source.id, revision: track.source.revision, product: 'espionage-log', params: { branchId: track.branch.id } } : null;
  return { track, ...useProduct<EspionageLogReply>(runtime.client, request, active, validateEspionageLogReply) };
}
export function useEspionageChoices(widget: WidgetSpec, runtime: WidgetRuntime): OptionChoices {
  const { data } = useEspionageLog(widget, runtime, true);
  return { types: operationChoices(data) };
}

import type { OptionChoices, WidgetRuntime } from '../../dashboard/definitions';
import type { WidgetSpec } from '../../dashboard/model';
import { usePeaceDeals } from '../../dashboard/usePeaceDeals';
import { usePlaybackHour } from '../../dashboard/playback';
import { transferTime } from './transferDrillData';
import { conferenceToken, transferConferenceLabel, visibleConferences } from './territoryData';

/** The same recorded dates and names as the Peace deals cards. Stored identity
 * is escaped so even a conference ID with spaces can be chosen. */
export function useConferenceChoices(widget: WidgetSpec, runtime: WidgetRuntime): OptionChoices {
  const playbackHour = usePlaybackHour(runtime.clock, widget.time.mode === 'follow');
  const { track, data } = usePeaceDeals(widget, runtime, true);
  const { moment: hour, from } = track ? transferTime(widget, track, runtime.tracks, playbackHour) : { moment: -Infinity, from: -Infinity };
  const choices = track ? visibleConferences(data?.deals ?? [], hour, from).map(deal => ({
    value: conferenceToken(track.source.metadata.campaignKey, track.branch.id, deal.id), label: transferConferenceLabel(deal) })) : [];
  // An ID kept from another campaign remains a visible, explicit missing choice.
  const selected = typeof widget.options?.conference === 'string' ? widget.options.conference : '';
  if (selected && !choices.some(choice => choice.value === selected)) choices.push({ value: selected, label: 'Not in this recording or not yet reached' });
  return { conference: choices };
}

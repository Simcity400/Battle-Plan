import type { OptionChoices, WidgetRuntime } from '../../dashboard/definitions';
import type { WidgetSpec } from '../../dashboard/model';
import { usePeaceDeals } from '../../dashboard/usePeaceDeals';
import { useHeadHour } from '../army-size/headHour.ts';
import { transferTime } from './transferDrillData';
import { conferenceToken, transferConferenceLabel, visibleConferences } from './territoryData';

/** The same recorded dates and names as the Peace deals cards. Stored identity
 * is escaped so even a conference ID with spaces can be chosen. */
export function useConferenceChoices(widget: WidgetSpec, runtime: WidgetRuntime): OptionChoices {
  const { track, data } = usePeaceDeals(widget, runtime, true);
  // The choices change only when the head reaches another conference.
  const playbackHour = useHeadHour(runtime.clock, widget.time.mode === 'follow', at => {
    if (!track) return '';
    const { moment, from } = transferTime(widget, track, runtime.tracks, at);
    return String(visibleConferences(data?.deals ?? [], moment, from).length);
  });
  const { moment: hour, from } = track ? transferTime(widget, track, runtime.tracks, playbackHour) : { moment: -Infinity, from: -Infinity };
  const choices = track ? visibleConferences(data?.deals ?? [], hour, from).map(deal => ({
    value: conferenceToken(track.source.metadata.campaignKey, track.branch.id, deal.id), label: transferConferenceLabel(deal) })) : [];
  // An ID kept from another campaign remains a visible, explicit missing choice.
  const selected = typeof widget.options?.conference === 'string' ? widget.options.conference : '';
  if (selected && !choices.some(choice => choice.value === selected)) choices.push({ value: selected, label: 'Not in this recording or not yet reached' });
  return { conference: choices };
}

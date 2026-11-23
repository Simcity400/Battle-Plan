import type { WidgetView } from '../../dashboard/follow/page.ts';

export function reportFamilyShownHour(
  reportShown: (hour: number | null) => void,
  visible: boolean,
  shown: WidgetView | null,
  installedHour?: number | null,
): number | null {
  if (!visible) {
    reportShown(null);
    return null;
  }
  if (installedHour !== undefined) {
    reportShown(installedHour);
    return installedHour;
  }
  if (shown?.through !== undefined) {
    reportShown(shown.through);
    return shown.painted ?? shown.through;
  }
  return null;
}

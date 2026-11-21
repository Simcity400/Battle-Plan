export function barContent(icons: boolean, names: boolean, hasIcon: boolean): { icon: boolean; name: boolean } {
  const icon = icons && hasIcon;
  return { icon, name: names || (icons && !hasIcon) };
}

export const END_MARK_ROOM = 14;

export function barNameFits(nameWidth: number, inset: number, barWidth: number, endMark: boolean): boolean {
  return nameWidth + inset + 6 + (endMark ? END_MARK_ROOM : 0) < barWidth;
}

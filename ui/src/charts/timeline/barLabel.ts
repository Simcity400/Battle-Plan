/** What a timeline bar draws inside itself: its icon when icons are on and it
 * has art, and its name when names are on. With names off, a bar that was meant
 * to show its icon but has no art (a mod's missing portrait) shows its name
 * instead, so it is never blank; a bar whose lane asks for neither (a spirit's
 * history, named in the left column) stays plain. */
export function barContent(icons: boolean, names: boolean, hasIcon: boolean): { icon: boolean; name: boolean } {
  const icon = icons && hasIcon;
  return { icon, name: names || (icons && !hasIcon) };
}

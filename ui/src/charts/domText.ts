
export function setHidden(element: HTMLElement | null | undefined, hidden: boolean): void {
  if (element && element.hidden !== hidden) element.hidden = hidden;
}

export function setStyle(element: HTMLElement | null | undefined, property: string, value: string): void {
  if (element && element.style.getPropertyValue(property) !== value) element.style.setProperty(property, value);
}

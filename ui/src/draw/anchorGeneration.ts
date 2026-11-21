export function shownTime(shown: number | undefined): number {
  return shown === undefined ? performance.now() : shown - performance.timeOrigin;
}

export function noticeMatchesAnchor(notice: number | undefined, current: number): boolean {
  return notice === current;
}

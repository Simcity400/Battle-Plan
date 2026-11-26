export function rowHoverPublisher(target: () => ((subject: string | null) => void) | undefined): (subject: string | null) => void {
  let last: string | null = null;
  return subject => {
    if (subject === last) return;
    last = subject;
    target()?.(subject);
  };
}

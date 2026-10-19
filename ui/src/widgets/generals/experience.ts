/** Fixed5 display keeps signed i64 operands exact until the final text. */
export function experienceText(raw: string, required: string | null): string {
  const points = (text: string) => {
    const value = BigInt(text); const magnitude = value < 0n ? -value : value;
    return `${value < 0n ? '-' : ''}${magnitude / 100000n}.${(magnitude % 100000n).toString().padStart(5, '0').replace(/0+$/, '') || '0'}`;
  };
  return required === null ? points(raw) : `${points(raw)} / ${points(required)}`;
}

/** Native overrides add to the trait's training factor; they are percentage points. */
export function experienceFactorText(raw: string): string {
  const value = BigInt(raw);
  return `${value >= 0n ? '+' : ''}${experienceText((value * 100n).toString(), null)}%`;
}

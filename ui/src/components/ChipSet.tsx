import './ChipSet.css';

/** One chip. `unavailable` says why clicking it cannot act, pressed or not, such
 * as a name that cannot be saved or the last choice that must stay. */
export interface Chip { value: string; label: string; icon?: string | null; colour?: string; unavailable?: string }

/** Toggle chips for choosing several things, such as resources. Each chip is a
 * pressed or unpressed button; once `max` are chosen the others are disabled.
 * `full` overrides that count where chosen things are not all listed as chips. A
 * disabled chip always says why, so no click fails silently. */
export function ChipSet({ label, chips, selected, max, full: atMax, onToggle }: {
  label: string; chips: readonly Chip[]; selected: readonly string[]; max: number; full?: boolean; onToggle: (value: string) => void;
}) {
  const full = atMax ?? selected.length >= max;
  return <div className="chip-set" role="group" aria-label={label}>{chips.map(chip => {
    const pressed = selected.includes(chip.value);
    const reason = chip.unavailable ?? (!pressed && full ? `At most ${max} can be chosen` : undefined);
    return <button key={chip.value} type="button" className="chip" aria-pressed={pressed} aria-disabled={reason ? true : undefined}
      data-tooltip={reason} onClick={() => { if (!reason) onToggle(chip.value); }}>
      {chip.colour && <i className="chip-dot" style={{ background: chip.colour }} aria-hidden="true" />}
      {chip.icon && <img src={chip.icon} alt="" />}{chip.label}
    </button>;
  })}</div>;
}

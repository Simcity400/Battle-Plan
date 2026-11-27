import { createRoot } from 'react-dom/client';
import EventDetailWindow, { type EventDetailSubject } from './EventDetailWindow.tsx';

const anchor = document.body.appendChild(document.createElement('button'));
const host = document.body.appendChild(document.createElement('div'));
const root = createRoot(host);
Object.assign(window, { eventDetail: {
  async show(firedHour: number | null, chosenOrdinal: number | null = 0) {
    const subject: EventDetailSubject = { key: 'orphan', hour: 137, firedHour, asOfHour: 150, title: 'A synthetic event',
      countryTag: null, countryName: null, item: null, chosenOrdinal, awaiting: false, chosenHour: 137, noChoiceReason: 'Chosen option not recorded' };
    root.render(<EventDetailWindow subject={subject} anchor={{ current: anchor }} onClose={() => {}} />);
    await new Promise<void>(resolve => requestAnimationFrame(() => resolve()));
    return document.body.textContent ?? '';
  },
} });

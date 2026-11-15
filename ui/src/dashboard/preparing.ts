import { createContext } from 'react';

/** Inside the prepare lane's hidden copy of a page: its widgets are not shown, and ask at the clock's hour what a shown one
 * is pushed, so the page holds it when shown. */
export const PreparingPage = createContext(false);

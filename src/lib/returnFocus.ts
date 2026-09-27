/**
 * Radix sends focus back only to a `Dialog.Trigger`; these dialogs open from
 * state, so without this a keyboard user lands on the page itself when one
 * closes. Spread into each `Dialog.Content`. A stack, since one dialog can
 * open another.
 */
const openers: (HTMLElement | null)[] = [];

export const returnFocus = {
  onOpenAutoFocus: (): void => {
    openers.push(document.activeElement instanceof HTMLElement ? document.activeElement : null);
  },
  onCloseAutoFocus: (event: Event): void => {
    const opener = openers.pop();
    if (opener?.isConnected === true) {
      event.preventDefault();
      opener.focus();
    }
  }
};

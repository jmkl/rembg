/**
 * Small declarative window-shortcut helper. Add new shortcuts to the array in
 * App.svelte; modifier flags default to "must not be held".
 */
export type Hotkey = {
  /** KeyboardEvent.key; letters are matched case-insensitively. */
  key: string;
  shift?: boolean;
  ctrl?: boolean;
  alt?: boolean;
  meta?: boolean;
  /** Only run when this returns true. */
  when?: () => boolean;
  /** Return false to allow the browser default behaviour. */
  run: (e: KeyboardEvent) => void | boolean;
  /** Also run while a text field / select has focus. */
  inInputs?: boolean;
  /** Allow keyboard auto-repeat (e.g. holding an arrow key). */
  repeat?: boolean;
};

export const isEditable = (target: EventTarget | null) =>
  target instanceof HTMLElement &&
  (["INPUT", "TEXTAREA", "SELECT"].includes(target.tagName) ||
    target.isContentEditable);

/** Run the first matching shortcut and prevent its browser default. */
export function handleHotkeys(e: KeyboardEvent, hotkeys: Hotkey[]) {
  for (const hotkey of hotkeys) {
    if (!hotkey.inInputs && isEditable(e.target)) continue;
    if (e.repeat && !hotkey.repeat) continue;
    if (e.key.toLowerCase() !== hotkey.key.toLowerCase()) continue;
    if (!!hotkey.shift !== e.shiftKey) continue;
    if (!!hotkey.ctrl !== e.ctrlKey) continue;
    if (!!hotkey.alt !== e.altKey) continue;
    if (!!hotkey.meta !== e.metaKey) continue;
    if (hotkey.when && !hotkey.when()) continue;

    if (hotkey.run(e) !== false) e.preventDefault();
    return true;
  }
  return false;
}

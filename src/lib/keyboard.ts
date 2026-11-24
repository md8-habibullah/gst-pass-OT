import { useEffect, useState } from "react";

const MIN_KEYBOARD_PX = 120;

const typing = (): boolean => {
  const el = document.activeElement as HTMLElement | null;
  return !!el && (el.tagName === "INPUT" || el.tagName === "TEXTAREA" || el.tagName === "SELECT" || el.isContentEditable);
};

/**
 * How many pixels of the page the on-screen keyboard covers that the page has NOT already given back.
 *
 * On Android the native activity pads the window by the keyboard's height (scripts/android/MainActivity.kt.tmpl), which
 * shrinks the page, so the layout viewport and the visual viewport shrink together and this returns 0. Where that does not
 * happen (a different web view, a browser, an older Android) only the visual viewport shrinks, a sheet fixed to the bottom
 * stays behind the keyboard, and this returns the missing height so the sheet can move up by it. A pinch-zoom also shrinks
 * the visual viewport, so the value only counts while a text field has focus and only above a small threshold.
 */
export function useKeyboardInset(): number {
  const [inset, setInset] = useState(0);
  useEffect(() => {
    const vv = window.visualViewport;
    if (!vv) return;
    const read = () => {
      const hidden = Math.round(window.innerHeight - vv.height - vv.offsetTop);
      setInset(typing() && hidden >= MIN_KEYBOARD_PX ? hidden : 0);
    };
    read();
    vv.addEventListener("resize", read);
    vv.addEventListener("scroll", read);
    window.addEventListener("focusin", read);
    window.addEventListener("focusout", read);
    return () => {
      vv.removeEventListener("resize", read);
      vv.removeEventListener("scroll", read);
      window.removeEventListener("focusin", read);
      window.removeEventListener("focusout", read);
    };
  }, []);
  return inset;
}

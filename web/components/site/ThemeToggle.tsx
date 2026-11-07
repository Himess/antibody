"use client";

import { useSyncExternalStore } from "react";
import { Icon } from "../ui/Icon";

type Theme = "system" | "light" | "dark";

const EVENT = "antibody-theme";

function read(): Theme {
  try {
    const t = localStorage.getItem("theme");
    return t === "light" || t === "dark" ? t : "system";
  } catch {
    return "system";
  }
}

function subscribe(cb: () => void) {
  window.addEventListener(EVENT, cb);
  window.addEventListener("storage", cb);
  return () => {
    window.removeEventListener(EVENT, cb);
    window.removeEventListener("storage", cb);
  };
}

function apply(t: Theme) {
  const el = document.documentElement;
  if (t === "system") el.removeAttribute("data-theme");
  else el.setAttribute("data-theme", t);
  try {
    if (t === "system") localStorage.removeItem("theme");
    else localStorage.setItem("theme", t);
  } catch {
    /* storage may be unavailable: the choice lasts for this page */
  }
  window.dispatchEvent(new Event(EVENT));
}

/** System by default; a click cycles light, dark, system, remembered per browser. */
export function ThemeToggle() {
  const theme = useSyncExternalStore(subscribe, read, (): Theme => "system");
  const next: Theme = theme === "system" ? "light" : theme === "light" ? "dark" : "system";
  return (
    <button
      type="button"
      className="btn btn-ghost btn-sm icon-btn"
      onClick={() => apply(next)}
      aria-label={`Theme: ${theme}. Switch to ${next}.`}
      title={`Theme: ${theme}`}
    >
      <Icon name={theme === "dark" ? "moon" : theme === "light" ? "sun" : "layers"} size={16} />
    </button>
  );
}

/** Runs before paint (an inline script carrying the request's nonce), so a remembered theme never flashes. */
export const themeScript =
  'try{var t=localStorage.getItem("theme");if(t==="light"||t==="dark")document.documentElement.setAttribute("data-theme",t)}catch(e){}';

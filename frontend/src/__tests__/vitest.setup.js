import { afterEach, vi } from "vitest";

afterEach(() => {
  document.body.innerHTML = "";
  document.documentElement.removeAttribute("data-color-scheme");
  document.documentElement.style.colorScheme = "";
  window.localStorage.clear();
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

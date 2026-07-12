import { describe, expect, it } from "vitest";
import { APP_THEMES, contrastRatio } from "./theme-contract.js";

const TEXT_PAIRS = [
  ["base-content", "base-100"],
  ["primary-content", "primary"],
  ["secondary-content", "secondary"],
  ["accent-content", "accent"],
  ["neutral-content", "neutral"],
  ["info-content", "info"],
  ["success-content", "success"],
  ["warning-content", "warning"],
  ["error-content", "error"],
  ["--app-muted-foreground", "--app-background"],
  ["--app-toast-success-content", "--app-toast-success-bg"],
  ["--app-toast-error-content", "--app-toast-error-bg"],
];

describe("theme contract", () => {
  it.each(Object.entries(APP_THEMES))(
    "%s keeps text and status pairs at WCAG AA contrast",
    (_name, theme) => {
      for (const [foreground, background] of TEXT_PAIRS) {
        expect(
          contrastRatio(theme[foreground], theme[background]),
          `${foreground} on ${background}`,
        ).toBeGreaterThanOrEqual(4.5);
      }
    },
  );

  it.each(Object.entries(APP_THEMES))(
    "%s keeps the focus ring distinguishable from both surfaces",
    (_name, theme) => {
      expect(contrastRatio(theme.accent, theme["base-100"])).toBeGreaterThanOrEqual(3);
      expect(contrastRatio(theme.accent, theme["base-200"])).toBeGreaterThanOrEqual(3);
    },
  );
});

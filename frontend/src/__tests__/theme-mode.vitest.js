import { describe, expect, it } from "vitest";
import {
  nextThemeMode,
  normalizeThemeMode,
  resolveThemeScheme,
  themeModeLabel,
} from "../shared/theme/theme-mode.js";

describe("theme mode", () => {
  it("normalizes supported values", () => {
    expect(normalizeThemeMode("system")).toBe("system");
    expect(normalizeThemeMode("light")).toBe("light");
    expect(normalizeThemeMode("dark")).toBe("dark");
    expect(normalizeThemeMode("bad")).toBe("system");
    expect(normalizeThemeMode(null)).toBe("system");
  });

  it("resolves the effective theme", () => {
    expect(resolveThemeScheme("system", true)).toBe("dark");
    expect(resolveThemeScheme("system", false)).toBe("light");
    expect(resolveThemeScheme("light", true)).toBe("light");
    expect(resolveThemeScheme("dark", false)).toBe("dark");
    expect(resolveThemeScheme("bad", true)).toBe("dark");
  });

  it("cycles modes and exposes accessible labels", () => {
    expect(nextThemeMode("system")).toBe("light");
    expect(nextThemeMode("light")).toBe("dark");
    expect(nextThemeMode("dark")).toBe("system");
    expect(nextThemeMode("bad")).toBe("light");
    expect(themeModeLabel("system")).toBe("主题：跟随系统");
    expect(themeModeLabel("light")).toBe("主题：浅色");
    expect(themeModeLabel("dark")).toBe("主题：深色");
  });
});

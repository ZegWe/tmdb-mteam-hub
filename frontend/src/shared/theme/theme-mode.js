export const THEME_STORAGE_KEY = "tmdb-mteam-theme-mode";
const THEME_MODES = ["system", "light", "dark"];

const THEME_MODE_LABELS = {
  system: "主题：跟随系统",
  light: "主题：浅色",
  dark: "主题：深色",
};

export function normalizeThemeMode(value) {
  return THEME_MODES.includes(value) ? value : "system";
}

export function resolveThemeScheme(mode, prefersDark) {
  const normalized = normalizeThemeMode(mode);
  if (normalized === "dark") return "dark";
  if (normalized === "light") return "light";
  return prefersDark ? "dark" : "light";
}

export function nextThemeMode(mode) {
  const normalized = normalizeThemeMode(mode);
  const index = THEME_MODES.indexOf(normalized);
  return THEME_MODES[(index + 1) % THEME_MODES.length];
}

export function themeModeLabel(mode) {
  return THEME_MODE_LABELS[normalizeThemeMode(mode)];
}

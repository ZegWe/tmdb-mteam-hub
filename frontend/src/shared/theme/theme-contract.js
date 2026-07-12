function appTheme(colors, app) {
  return Object.freeze({
    ...colors,
    "--rounded-box": "0.5rem",
    "--rounded-btn": "0.5rem",
    "--rounded-badge": "0.5rem",
    "--tab-radius": "0.5rem",
    "--app-background": colors["base-200"],
    "--app-foreground": colors["base-content"],
    "--app-card": colors["base-100"],
    "--app-card-foreground": colors["base-content"],
    "--app-popover": colors.neutral,
    "--app-popover-foreground": colors["neutral-content"],
    "--app-primary": colors.primary,
    "--app-primary-foreground": colors["primary-content"],
    "--app-secondary": colors.secondary,
    "--app-secondary-foreground": colors["secondary-content"],
    "--app-accent-bg": colors.secondary,
    "--app-accent-foreground": colors["secondary-content"],
    "--app-destructive": colors.error,
    "--app-destructive-foreground": colors["error-content"],
    "--app-border": colors["base-300"],
    "--app-input": app.input,
    "--app-ring": colors.accent,
    "--app-success": colors.success,
    "--app-warning": colors.warning,
    "--app-info": colors.info,
    "--app-muted-bg": app.mutedBackground,
    "--app-muted-foreground": app.mutedForeground,
    "--app-accent-dim": app.accentDim,
    "--app-toast-success-bg": app.toastSuccessBackground,
    "--app-toast-success-content": app.toastSuccessContent,
    "--app-toast-error-bg": app.toastErrorBackground,
    "--app-toast-error-content": app.toastErrorContent,
    "--app-shadow-sm": app.shadowSmall,
    "--app-shadow-md": app.shadowMedium,
  });
}

const lightTheme = appTheme(
  {
    primary: "#18181b",
    "primary-content": "#fafafa",
    secondary: "#e7f6f1",
    "secondary-content": "#0f5f54",
    accent: "#0f766e",
    "accent-content": "#f0fdfa",
    neutral: "#27272a",
    "neutral-content": "#fafafa",
    "base-100": "#ffffff",
    "base-200": "#f7f8f7",
    "base-300": "#d9dedc",
    "base-content": "#18181b",
    info: "#2563eb",
    "info-content": "#ffffff",
    success: "#047857",
    "success-content": "#ffffff",
    warning: "#b45309",
    "warning-content": "#ffffff",
    error: "#b42318",
    "error-content": "#ffffff",
  },
  {
    input: "#cfd6d3",
    mutedBackground: "#f1f3f3",
    mutedForeground: "#6b7075",
    accentDim: "#0d9488",
    toastSuccessBackground: "#d1fae5",
    toastSuccessContent: "#065f46",
    toastErrorBackground: "#fee2e2",
    toastErrorContent: "#991b1b",
    shadowSmall: "0 1px 2px rgba(24, 24, 27, 0.06)",
    shadowMedium: "0 12px 32px rgba(24, 24, 27, 0.12)",
  },
);

const darkTheme = appTheme(
  {
    primary: "#e7f6f1",
    "primary-content": "#0b1110",
    secondary: "#22302d",
    "secondary-content": "#d7f5ee",
    accent: "#5eead4",
    "accent-content": "#06211d",
    neutral: "#1d2221",
    "neutral-content": "#eef2f1",
    "base-100": "#191d1c",
    "base-200": "#111313",
    "base-300": "#303a37",
    "base-content": "#eef2f1",
    info: "#60a5fa",
    "info-content": "#0b1110",
    success: "#34d399",
    "success-content": "#0b1110",
    warning: "#f59e0b",
    "warning-content": "#0b1110",
    error: "#f87171",
    "error-content": "#190707",
  },
  {
    input: "#3b4642",
    mutedBackground: "#202625",
    mutedForeground: "#9aa5a2",
    accentDim: "#2dd4bf",
    toastSuccessBackground: "#064e3b",
    toastSuccessContent: "#d1fae5",
    toastErrorBackground: "#7f1d1d",
    toastErrorContent: "#fee2e2",
    shadowSmall: "0 1px 2px rgba(0, 0, 0, 0.35)",
    shadowMedium: "0 18px 42px rgba(0, 0, 0, 0.42)",
  },
);

export const APP_THEMES = Object.freeze({
  mediahub: lightTheme,
  "mediahub-dark": darkTheme,
});

export const DAISYUI_THEMES = Object.freeze([
  Object.freeze({ mediahub: lightTheme }),
  Object.freeze({ "mediahub-dark": darkTheme }),
]);

function channelToLinear(value) {
  const normalized = value / 255;
  return normalized <= 0.04045 ? normalized / 12.92 : ((normalized + 0.055) / 1.055) ** 2.4;
}

export function contrastRatio(foreground, background) {
  const parse = (value) => {
    const match = /^#([\da-f]{2})([\da-f]{2})([\da-f]{2})$/i.exec(String(value));
    if (!match) throw new TypeError(`Expected a six-digit hex color, received ${value}`);
    const red = channelToLinear(Number.parseInt(match[1], 16));
    const green = channelToLinear(Number.parseInt(match[2], 16));
    const blue = channelToLinear(Number.parseInt(match[3], 16));
    return 0.2126 * red + 0.7152 * green + 0.0722 * blue;
  };
  const first = parse(foreground);
  const second = parse(background);
  return (Math.max(first, second) + 0.05) / (Math.min(first, second) + 0.05);
}

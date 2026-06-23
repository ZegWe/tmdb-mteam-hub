/** @type {import('tailwindcss').Config} */
export default {
  content: ["./frontend/index.html", "./frontend/src/**/*.{vue,js,ts}"],
  theme: {
    extend: {},
  },
  plugins: [require("daisyui")],
  daisyui: {
    logs: false,
    themes: [
      {
        mediahub: {
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
          success: "#047857",
          warning: "#b45309",
          error: "#b42318",
          "--rounded-box": "0.5rem",
          "--rounded-btn": "0.5rem",
          "--rounded-badge": "0.5rem",
          "--tab-radius": "0.5rem",
        },
      },
    ],
  },
};

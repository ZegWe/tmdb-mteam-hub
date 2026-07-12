import daisyui from "daisyui";
import { DAISYUI_THEMES } from "./frontend/src/shared/theme/theme-contract.js";

/** @type {import('tailwindcss').Config} */
export default {
  content: ["./frontend/index.html", "./frontend/src/**/*.{vue,js,ts}"],
  theme: {
    extend: {},
  },
  plugins: [daisyui],
  daisyui: {
    logs: false,
    themes: DAISYUI_THEMES,
  },
};

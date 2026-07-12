# Browser acceptance evidence

`app.spec.js` covers user journeys. `visual.spec.js` renders the six real routes with the deterministic
fixture in both light and dark themes. Every visual case checks that the primary heading is visible,
the document has no horizontal overflow, and the generated PNG is non-empty.

Build and capture Chromium desktop/mobile evidence:

```bash
npm run build
E2E_PORT=4189 npx playwright test e2e/visual.spec.js \
  --project chromium-desktop --project chromium-mobile
```

When using a system Chrome instead of the Playwright-managed runtime:

```bash
E2E_PORT=4189 PLAYWRIGHT_CHROMIUM_EXECUTABLE=/usr/bin/google-chrome-stable \
  npx playwright test e2e/visual.spec.js \
  --project chromium-desktop --project chromium-mobile
```

Screenshots are written to:

```text
test-results/visual-evidence/<project>/<theme>/<route>.png
```

The directory is intentionally ignored by Git. Review all generated PNGs before declaring the manual
visual gate complete; generation and the automated layout checks are not a substitute for human review.

// vp test setup — no vitest imports allowed.
// Run standard DOM/Web API cleanup after each test.
if (typeof globalThis.afterEach === 'function') {
  globalThis.afterEach(() => {
    document.body.innerHTML = "";
    document.documentElement.removeAttribute("data-color-scheme");
    document.documentElement.style.colorScheme = "";
  });
}

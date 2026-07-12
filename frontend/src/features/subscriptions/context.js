import { readonly, ref } from "vue";
import { createSubscriptionStore } from "./store.js";

export const SUBSCRIPTION_CONTEXT_KEY = Symbol("subscription-context");

export function createSubscriptionContext({
  store = createSubscriptionStore(),
  scheduleMicrotask = (callback) => queueMicrotask(callback),
} = {}) {
  const activeRouteCount = ref(0);
  let lifecycleRevision = 0;
  let pollingStarted = false;
  let startPromise = null;

  function enterRoute() {
    activeRouteCount.value += 1;
    lifecycleRevision += 1;
    if (pollingStarted) return startPromise || Promise.resolve(store.state?.value ?? null);
    pollingStarted = true;
    try {
      const request = Promise.resolve(store.start());
      const tracked = request.finally(() => {
        if (startPromise === tracked) startPromise = null;
      });
      startPromise = tracked;
      return tracked;
    } catch (error) {
      pollingStarted = false;
      throw error;
    }
  }

  function leaveRoute() {
    activeRouteCount.value = Math.max(0, activeRouteCount.value - 1);
    if (activeRouteCount.value) return;
    const revision = ++lifecycleRevision;
    scheduleMicrotask(() => {
      if (activeRouteCount.value || revision !== lifecycleRevision) return;
      pollingStarted = false;
      startPromise = null;
      store.stop();
    });
  }

  function dispose() {
    activeRouteCount.value = 0;
    lifecycleRevision += 1;
    pollingStarted = false;
    startPromise = null;
    store.dispose();
  }

  return Object.freeze({
    store,
    activeRouteCount: readonly(activeRouteCount),
    enterRoute,
    leaveRoute,
    dispose,
  });
}

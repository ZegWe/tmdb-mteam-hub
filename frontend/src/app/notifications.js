export const APP_NOTIFICATIONS_KEY = Symbol("app-notifications");

export const NOOP_APP_NOTIFICATIONS = Object.freeze({
  clearError() {},
  showError() {},
  showToast() {},
});

export const AUTH_SESSION_CHANGED_EVENT = "tmdb-mteam-auth-session-changed";

/**
 * @param {{authenticated?: boolean, token_configured?: boolean, bootstrap_allowed?: boolean}} [status]
 */
export function notifyAuthSessionChanged(status = {}) {
  if (
    typeof globalThis.dispatchEvent !== "function" ||
    typeof globalThis.CustomEvent !== "function"
  ) {
    return;
  }
  globalThis.dispatchEvent(
    new CustomEvent(AUTH_SESSION_CHANGED_EVENT, {
      detail: {
        authenticated: status?.authenticated === true,
        token_configured: status?.token_configured === true,
        bootstrap_allowed: status?.bootstrap_allowed === true,
      },
    }),
  );
}

export function notifyAuthenticationRequired() {
  notifyAuthSessionChanged({ authenticated: false });
}

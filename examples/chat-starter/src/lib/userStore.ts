// localStorage-backed "currently signed-in demo user" for the multi-tenant
// dropdown. null = use the backend's fallback admin user.
//
// Values are validated against the shared USER_RE so a tampered localStorage
// entry can't smuggle SQL fragments into POST /api/auth/token.

import { USER_RE } from "./user";

const KEY = "kalamdb-chat:user";

export function loadUser(): string | null {
  try {
    const raw = window.localStorage.getItem(KEY);
    if (!raw) return null;
    return USER_RE.test(raw) ? raw : null;
  } catch {
    return null;
  }
}

export function saveUser(user: string | null): void {
  try {
    if (user && USER_RE.test(user)) {
      window.localStorage.setItem(KEY, user);
    } else {
      window.localStorage.removeItem(KEY);
    }
  } catch {
    // localStorage can throw under private-browsing or storage-quota errors.
    // The dropdown still works in-memory for the session; persistence just
    // skips this turn.
  }
}

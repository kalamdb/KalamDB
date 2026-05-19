import React from "react";
import { KalamProvider } from "@kalamdb/react";
import { createKalamClient } from "./client";
import { App } from "./components/App";
import { loadUser, saveUser } from "./lib/userStore";
import { DEMO_USER_LIST } from "./lib/demo-users";

/** First demo user, used when there's nothing in localStorage. Hardcoded
 *  rather than fetched from /api/users so the very first render already
 *  has a real user — fetching after mount would force a KalamProvider
 *  remount once the user is picked, which resets App's local state
 *  (selectedId, etc.) and breaks any in-flight interaction. */
const DEFAULT_USER = DEMO_USER_LIST[0]!;

/**
 * AppShell owns the "who is signed in" state for the multi-tenant demo. When
 * the user picks a different identity from the dropdown:
 *   1. Build a brand new KalamDB client whose authProvider sends the new
 *      `user` to /api/auth/token (so the backend mints THAT user's JWT).
 *   2. Disconnect the previous client — without this, its WebSocket and any
 *      open live subscriptions linger in the background under the old
 *      identity, wasting a connection and potentially leaking events.
 *   3. <KalamProvider> remounts via the `key` so every live subscription
 *      tears down and re-attaches under the new auth. Critical for the
 *      multi-tenant story: alice's tab must never see bob's data bleeding
 *      through a stale subscription.
 */
export function AppShell() {
  const [currentUser, setCurrentUser] = React.useState<string>(() => loadUser() ?? DEFAULT_USER);
  const [client, setClient] = React.useState(() => createKalamClient(currentUser));

  const handleUserChange = React.useCallback((next: string) => {
    saveUser(next);
    setCurrentUser(next);
    setClient((previous) => {
      void previous.disconnect().catch(() => undefined);
      return createKalamClient(next);
    });
  }, []);

  return (
    <KalamProvider key={currentUser} client={client}>
      <App currentUser={currentUser} onUserChange={handleUserChange} />
    </KalamProvider>
  );
}

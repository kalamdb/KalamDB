/// Whether [Kalam.open] should create, replace, or keep the account session.
enum KalamSessionPlan { open, replace, keep }

/// Token refresh for the same user keeps the session. A different user replaces it.
KalamSessionPlan planKalamSession({
  required String? existingSubject,
  required String userId,
}) {
  if (existingSubject == null || existingSubject.isEmpty) {
    return KalamSessionPlan.open;
  }
  if (existingSubject != userId) {
    return KalamSessionPlan.replace;
  }
  return KalamSessionPlan.keep;
}

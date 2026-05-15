import "@testing-library/jest-dom/vitest";

// jsdom doesn't implement Element.scrollTo. Conversation.tsx calls it in an
// effect; stub it so the tests don't throw.
if (typeof Element !== "undefined" && !Element.prototype.scrollTo) {
  Element.prototype.scrollTo = function () {} as Element["scrollTo"];
}

import "@testing-library/jest-dom/vitest";

// jsdom does not implement window.matchMedia
Object.defineProperty(window, "matchMedia", {
  writable: true,
  value: (query: string) => ({
    matches: false,
    media: query,
    onchange: null,
    addListener: () => {},
    removeListener: () => {},
    addEventListener: () => {},
    removeEventListener: () => {},
    dispatchEvent: () => false,
  }),
});

// jsdom does not ship ResizeObserver, but cmdk / base-ui popovers rely on
// it. Stub it with a no-op so component tests can mount floating UIs.
if (typeof globalThis.ResizeObserver === "undefined") {
  class ResizeObserverStub {
    observe() {}
    unobserve() {}
    disconnect() {}
  }
  // @ts-expect-error assigning to globalThis
  globalThis.ResizeObserver = ResizeObserverStub;
}

// cmdk's popover calls Element.scrollIntoView on the active item; jsdom
// doesn't implement it. No-op stub keeps component tests green.
if (typeof Element !== "undefined" && !Element.prototype.scrollIntoView) {
  Element.prototype.scrollIntoView = function scrollIntoView() {};
}

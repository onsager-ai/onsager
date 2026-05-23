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

// jsdom does not ship IntersectionObserver. The anchored-timeline lazy-load
// (#466) uses it as a scroll-end sentinel. Stub it so component tests can
// mount; tests that exercise the lazy-load path call the captured callback
// manually via the global mock.
if (typeof globalThis.IntersectionObserver === "undefined") {
  class IntersectionObserverStub {
    constructor(cb: IntersectionObserverCallback) {
      void cb;
    }
    observe() {}
    unobserve() {}
    disconnect() {}
    takeRecords(): IntersectionObserverEntry[] {
      return [];
    }
    readonly root = null;
    readonly rootMargin = "";
    readonly thresholds: ReadonlyArray<number> = [];
  }
  // @ts-expect-error assigning to globalThis
  globalThis.IntersectionObserver = IntersectionObserverStub;
}

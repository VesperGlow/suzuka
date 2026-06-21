(() => {
  let pagefindPromise;

  window.suzukaLoadPagefind = () => {
    if (!pagefindPromise) {
      pagefindPromise = import("/pagefind/pagefind.js").then(async (pagefind) => {
        await pagefind.init();
        return pagefind;
      }).catch((error) => {
        pagefindPromise = undefined;
        throw error;
      });
    }

    return pagefindPromise;
  };

  const warmup = () => {
    window.suzukaLoadPagefind().catch((error) => {
      console.warn("Pagefind warmup failed:", error);
    });
  };

  const isSearchInput = (element) =>
    element instanceof HTMLInputElement && element.type === "search";

  document.addEventListener("focusin", (event) => {
    if (isSearchInput(event.target)) warmup();
  });

  document.addEventListener("DOMContentLoaded", () => {
    if (isSearchInput(document.activeElement)) warmup();

    const connection = navigator.connection;
    if (
      connection?.saveData ||
      connection?.effectiveType === "slow-2g" ||
      connection?.effectiveType === "2g"
    ) {
      return;
    }

    if ("requestIdleCallback" in window) {
      window.requestIdleCallback(warmup, { timeout: 2000 });
    } else {
      window.setTimeout(warmup, 1200);
    }
  });
})();

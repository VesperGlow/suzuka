(() => {
  let pagefindPromise;

  const escapeHTML = (value) => {
    const element = document.createElement("span");
    element.textContent = value;
    return element.innerHTML;
  };

  const excerptFor = (content, query) => {
    const normalizedContent = content.toLocaleLowerCase();
    const normalizedQuery = query.toLocaleLowerCase();
    const matchIndex = normalizedContent.indexOf(normalizedQuery);
    const start = Math.max(0, matchIndex < 0 ? 0 : matchIndex - 45);
    const excerpt = content.slice(start, start + 130).trim();
    return `${start > 0 ? "…" : ""}${escapeHTML(excerpt)}${start + 130 < content.length ? "…" : ""}`;
  };

  const loadFallbackSearch = async () => {
    const languageRoot = document.documentElement.lang.toLowerCase().startsWith("en") ? "/en" : "";
    const response = await fetch(`${languageRoot}/index.json`, {
      headers: { Accept: "application/json" },
    });
    if (!response.ok) throw new Error(`Search index request failed: ${response.status}`);

    const { items = [] } = await response.json();
    return {
      preload: async () => {},
      search: async (query) => {
        const terms = query
          .toLocaleLowerCase()
          .split(/\s+/)
          .filter(Boolean);

        const matches = items
          .map((item) => {
            const title = item.title.toLocaleLowerCase();
            const tags = item.tags.join(" ").toLocaleLowerCase();
            const content = item.content.toLocaleLowerCase();
            if (!terms.every((term) => title.includes(term) || tags.includes(term) || content.includes(term))) {
              return null;
            }

            const score = terms.reduce(
              (total, term) => total + (title.includes(term) ? 4 : 0) + (tags.includes(term) ? 2 : 0) + (content.includes(term) ? 1 : 0),
              0
            );
            return { item, score };
          })
          .filter(Boolean)
          .sort((a, b) => b.score - a.score);

        return {
          results: matches.map(({ item }) => ({
            data: async () => ({
              url: item.url,
              meta: { title: item.title, date: item.date, tags: item.tags },
              excerpt: excerptFor(item.content, query),
            }),
          })),
        };
      },
    };
  };

  const loadSearch = async () => {
    try {
      const response = await fetch("/pagefind/pagefind.js", { method: "HEAD" });
      const contentType = response.headers.get("content-type") || "";
      if (!response.ok || !/javascript|ecmascript/.test(contentType)) {
        throw new Error("Pagefind module is not deployed");
      }

      const pagefind = await import("/pagefind/pagefind.js");
      await pagefind.init();
      return pagefind;
    } catch (error) {
      console.info("Using the built-in search index:", error.message);
      return loadFallbackSearch();
    }
  };

  window.suzukaLoadPagefind = () => {
    if (!pagefindPromise) {
      pagefindPromise = loadSearch().catch((error) => {
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

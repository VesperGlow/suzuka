const root = document.querySelector("[data-search-page]");

if (root) {
  const input = root.querySelector("[data-search-input]");
  const form = root.querySelector("[data-search-form]");
  const clearButton = root.querySelector("[data-search-clear]");
  const status = root.querySelector("[data-search-status]");
  const results = root.querySelector("[data-search-results]");
  const messages = root.dataset;

  if (input && form && clearButton && status && results) {
    let requestId = 0;
    let debounceTimer;

    const loadPagefind = () => {
      if (typeof window.suzukaLoadPagefind !== "function") {
        return Promise.reject(new Error("Pagefind loader is unavailable"));
      }
      return window.suzukaLoadPagefind();
    };

    function setQueryInUrl(query) {
      const url = new URL(window.location.href);
      query ? url.searchParams.set("q", query) : url.searchParams.delete("q");
      window.history.replaceState(null, "", url);
    }

    function resetSearch() {
      window.clearTimeout(debounceTimer);
      requestId += 1;
      results.replaceChildren();
      status.textContent = messages.i18nPrompt;
      clearButton.hidden = true;
    }

    function resultCard(data) {
      const article = document.createElement("article");
      const title = document.createElement("h2");
      const link = document.createElement("a");
      const date = document.createElement("time");
      const excerpt = document.createElement("p");

      article.className = "search-result";
      link.href = data.url;
      link.textContent = data.meta?.title || data.url;
      title.append(link);
      date.className = "search-result-date";
      date.textContent = data.meta?.date || "";
      date.hidden = !date.textContent;
      excerpt.innerHTML = data.excerpt || data.content || "";
      article.append(title, date, excerpt);
      return article;
    }

    async function runSearch(query) {
      const normalized = query.trim();
      clearButton.hidden = !input.value;

      if (!normalized) {
        resetSearch();
        return;
      }

      const currentRequest = ++requestId;
      status.textContent = messages.i18nSearching;

      try {
        const pagefind = await loadPagefind();
        const response = await pagefind.search(normalized, {
          filters: { language: document.documentElement.lang.toLowerCase().startsWith("en") ? "en" : "zh-cn" },
        });
        if (currentRequest !== requestId) return;

        const matches = await Promise.all(response.results.map((result) => result.data()));
        if (currentRequest !== requestId) return;

        results.replaceChildren(...matches.map(resultCard));
        status.textContent = matches.length
          ? messages.i18nFound.replace("{count}", matches.length)
          : messages.i18nEmpty;
      } catch (error) {
        console.warn("Search is unavailable:", error);
        if (currentRequest !== requestId) return;
        results.replaceChildren();
        status.textContent = messages.i18nUnavailable;
      }
    }

    input.addEventListener("focus", () => {
      loadPagefind().catch(() => {});
    });

    input.addEventListener("input", () => {
      const query = input.value.trim();
      clearButton.hidden = !query;
      setQueryInUrl(query);
      window.clearTimeout(debounceTimer);

      if (!query) {
        resetSearch();
        return;
      }

      loadPagefind().then((pagefind) => pagefind.preload(query)).catch(() => {});
      debounceTimer = window.setTimeout(() => runSearch(query), 160);
    });

    form.addEventListener("submit", (event) => {
      event.preventDefault();
      window.clearTimeout(debounceTimer);
      const query = input.value.trim();
      setQueryInUrl(query);
      runSearch(query);
    });

    clearButton.addEventListener("click", () => {
      input.value = "";
      setQueryInUrl("");
      resetSearch();
      input.focus();
    });

    const initialQuery = new URLSearchParams(window.location.search).get("q") || "";
    input.value = initialQuery;
    clearButton.hidden = !initialQuery;
    if (initialQuery) runSearch(initialQuery);
  }
}

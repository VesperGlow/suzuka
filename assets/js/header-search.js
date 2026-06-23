const root = document.querySelector("[data-header-search]");

if (root) {
  const tools = root.closest("[data-header-tools]");
  const toggle = tools?.querySelector("[data-header-search-toggle]");
  const form = root.querySelector(".header-search");
  const input = form?.querySelector("input[type='search']");
  const clearButton = root.querySelector(".header-search-clear");
  const popover = root.querySelector(".header-search-popover");
  const status = root.querySelector(".header-search-status");
  const results = root.querySelector(".header-search-results");
  const allResults = root.querySelector(".header-search-all");
  const messages = root.dataset;
  if (form && input && clearButton && popover && status && results && allResults) {
  const mobileQuery = window.matchMedia("(max-width: 768px)");
  const minimumLength = 2;
  let debounceTimer;
  let requestId = 0;
  let suppressFocusOpen = false;

  const loadPagefind = () => {
    if (typeof window.suzukaLoadPagefind !== "function") {
      return Promise.reject(new Error("Pagefind loader is unavailable"));
    }
    return window.suzukaLoadPagefind();
  };

  function setSearchActive(active) {
    tools?.classList.toggle("search-active", active);
    toggle?.setAttribute("aria-expanded", String(active));
  }

  function tagsFor(data) {
    const value = data.meta?.tags;
    if (Array.isArray(value)) return value;
    return typeof value === "string" ? value.split("||").filter(Boolean) : [];
  }

  function textFromExcerpt(value) {
    return new DOMParser().parseFromString(String(value || ""), "text/html").body.textContent || "";
  }

  function highlightKeyword(text, query) {
    const fragment = document.createDocumentFragment();
    const keywords = [...new Set(query.trim().split(/\s+/u).filter(Boolean))]
      .sort((left, right) => right.length - left.length);
    if (!keywords.length) {
      fragment.append(document.createTextNode(text));
      return fragment;
    }

    const pattern = keywords
      .map((keyword) => keyword.replace(/[.*+?^${}()|[\]\\]/g, "\\$&"))
      .join("|");
    const matches = text.matchAll(new RegExp(pattern, "giu"));
    let position = 0;

    for (const match of matches) {
      fragment.append(document.createTextNode(text.slice(position, match.index)));
      const mark = document.createElement("mark");
      mark.className = "search-highlight";
      mark.textContent = match[0];
      fragment.append(mark);
      position = match.index + match[0].length;
    }
    fragment.append(document.createTextNode(text.slice(position)));
    return fragment;
  }

  function openPopover() {
    popover.hidden = false;
    input.setAttribute("aria-expanded", "true");
  }

  function closePopover() {
    popover.hidden = true;
    input.setAttribute("aria-expanded", "false");
  }

  function resetResults() {
    window.clearTimeout(debounceTimer);
    requestId += 1;
    results.replaceChildren();
    status.textContent = "";
    closePopover();
  }

  function updateAllResultsLink(query) {
    const url = new URL(root.dataset.searchUrl, window.location.origin);
    url.searchParams.set("q", query);
    allResults.href = `${url.pathname}${url.search}`;
  }

  function resultItem(data, query) {
    const article = document.createElement("article");
    const link = document.createElement("a");
    const title = document.createElement("strong");
    const excerpt = document.createElement("span");
    const meta = document.createElement("small");
    const tags = document.createElement("small");

    article.className = "header-search-result";
    article.setAttribute("role", "listitem");
    link.href = data.url;
    title.append(highlightKeyword(data.meta?.title || data.url, query));
    excerpt.className = "header-search-excerpt";
    excerpt.append(highlightKeyword(textFromExcerpt(data.excerpt || data.content), query));
    meta.textContent = data.meta?.date || new URL(data.url, window.location.origin).pathname;
    tags.className = "header-search-result-tags";
    const tagText = tagsFor(data).map((tag) => `#${tag}`).join("  ");
    tags.append(highlightKeyword(tagText, query));
    tags.hidden = !tags.textContent;
    link.append(title, meta, excerpt, tags);
    article.append(link);
    return article;
  }

  async function runSearch(query) {
    const normalized = query.trim();
    if (normalized.length < minimumLength) {
      resetResults();
      return;
    }

    const currentRequest = ++requestId;
    results.replaceChildren();
    status.textContent = messages.i18nSearching;
    updateAllResultsLink(normalized);
    openPopover();

    try {
      const pagefind = await loadPagefind();
      const response = await pagefind.search(normalized, {
        filters: { language: document.documentElement.lang.toLowerCase().startsWith("en") ? "en" : "zh-cn" },
      });
      if (currentRequest !== requestId) return;

      const limit = mobileQuery.matches ? 4 : 5;
      const matches = await Promise.all(
        response.results.slice(0, limit).map((result) => result.data())
      );
      if (currentRequest !== requestId) return;

      results.replaceChildren(...matches.map((match) => resultItem(match, normalized)));
      status.textContent = matches.length ? "" : messages.i18nEmpty;
    } catch (error) {
      console.error("Header search failed:", error);
      if (currentRequest !== requestId) return;
      results.replaceChildren();
      status.textContent = messages.i18nUnavailable;
    }
  }

  input.addEventListener("focus", () => {
    loadPagefind().catch(() => {});
    if (suppressFocusOpen) return;
    const query = input.value.trim();
    if (query.length >= minimumLength && (results.children.length || status.textContent)) {
      openPopover();
    }
  });

  toggle?.addEventListener("click", (event) => {
    event.preventDefault();
    setSearchActive(true);
    input.focus();
  });

  input.addEventListener("input", () => {
    const query = input.value.trim();
    clearButton.hidden = !input.value;
    window.clearTimeout(debounceTimer);

    if (query.length < minimumLength) {
      resetResults();
      return;
    }

    debounceTimer = window.setTimeout(() => runSearch(query), 160);
  });

  form.addEventListener("submit", (event) => {
    event.preventDefault();
    window.clearTimeout(debounceTimer);
    runSearch(input.value);
  });

  clearButton.addEventListener("click", () => {
    input.value = "";
    clearButton.hidden = true;
    resetResults();
    setSearchActive(false);
    toggle?.focus();
  });

  document.addEventListener("pointerdown", (event) => {
    if (!root.contains(event.target) && !toggle?.contains(event.target)) {
      closePopover();
      setSearchActive(false);
    }
  });

  root.addEventListener("keydown", (event) => {
    if (event.key === "Escape") {
      closePopover();
      setSearchActive(false);
      suppressFocusOpen = true;
      if (toggle && !mobileQuery.matches) toggle.focus();
      else input.blur();
      suppressFocusOpen = false;
    }
  });
  }
}

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
  const loadMore = root.querySelector(".header-search-more");
  const labels = root.dataset;
  if (form && input && clearButton && popover && status && results && loadMore) {
  const mobileQuery = window.matchMedia("(max-width: 768px)");
  // 中日文没有空白分词，一个字就能构成有意义的查询（比如单字标题「蝉」），
  // 拉丁语系维持 2 个字符的门槛，避免每敲一个字母就发起一次搜索。
  const CJK_CHAR = /[぀-ヿ㐀-䶿一-鿿豈-﫿]/u;
  const minimumLength = 2;
  const pageSize = 10;
  let debounceTimer;
  let requestId = 0;
  let activeSearch = null;

  function meetsMinimumLength(query) {
    return query.length >= minimumLength || (query.length === 1 && CJK_CHAR.test(query));
  }

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
    activeSearch = null;
    results.replaceChildren();
    loadMore.hidden = true;
    status.textContent = "";
    closePopover();
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

  async function renderNextBatch(search) {
    const batch = search.queue.splice(0, pageSize);
    if (!batch.length) {
      loadMore.hidden = true;
      return;
    }

    loadMore.disabled = true;
    const matches = await Promise.all(batch.map((result) => result.data()));
    if (search.request !== requestId) return;

    results.append(...matches.map((match) => resultItem(match, search.query)));
    loadMore.disabled = false;
    loadMore.hidden = search.queue.length === 0;
  }

  async function runSearch(query) {
    const normalized = query.trim();
    if (!meetsMinimumLength(normalized)) {
      resetResults();
      return;
    }

    const currentRequest = ++requestId;
    results.replaceChildren();
    loadMore.hidden = true;
    status.textContent = labels.i18nSearching;
    openPopover();

    try {
      // 不传 language filter：pagefind 本身就按页面 <html lang> 加载对应语言
      // 的独立索引，语言隔离已经成立。此前传的 filter 因为挂在 data-pagefind-body
      // 区域之外的 <body> 上而从未进入索引，反而让所有搜索恒为空。
      const pagefind = await loadPagefind();
      const response = await pagefind.search(normalized);
      if (currentRequest !== requestId) return;

      if (!response.results.length) {
        activeSearch = null;
        status.textContent = labels.i18nEmpty;
        return;
      }

      const search = { request: currentRequest, query: normalized, queue: response.results.slice() };
      activeSearch = search;
      status.textContent = labels.i18nCount.replace("{count}", response.results.length);
      await renderNextBatch(search);
    } catch (error) {
      console.error("Header search failed:", error);
      if (currentRequest !== requestId) return;
      activeSearch = null;
      results.replaceChildren();
      loadMore.hidden = true;
      status.textContent = labels.i18nUnavailable;
    }
  }

  input.addEventListener("focus", () => {
    loadPagefind().catch(() => {});
    const query = input.value.trim();
    if (meetsMinimumLength(query) && (results.children.length || status.textContent)) {
      openPopover();
    }
  });

  toggle?.addEventListener("click", (event) => {
    event.preventDefault();
    setSearchActive(true);
    input.focus();
  });

  loadMore.addEventListener("click", () => {
    if (activeSearch) renderNextBatch(activeSearch);
  });

  input.addEventListener("input", () => {
    const query = input.value.trim();
    clearButton.hidden = !input.value;
    window.clearTimeout(debounceTimer);

    if (!meetsMinimumLength(query)) {
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
      if (toggle && !mobileQuery.matches) toggle.focus();
      else input.blur();
    }
  });
  }
}

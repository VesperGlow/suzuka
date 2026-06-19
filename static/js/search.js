import * as pagefind from "/pagefind/pagefind.js";

const input = document.querySelector("#search-input");
const form = document.querySelector(".search-form");
const clearButton = document.querySelector("#search-clear");
const status = document.querySelector("#search-status");
const results = document.querySelector("#search-results");

if (input && form && clearButton && status && results) {
  let requestId = 0;

  function setQueryInUrl(query) {
    const url = new URL(window.location.href);
    query ? url.searchParams.set("q", query) : url.searchParams.delete("q");
    window.history.replaceState(null, "", url);
  }

  function resetSearch() {
    requestId += 1;
    results.replaceChildren();
    status.textContent = "输入关键词开始搜索。";
    clearButton.hidden = true;
  }

  function resultCard(data) {
    const article = document.createElement("article");
    const title = document.createElement("h2");
    const link = document.createElement("a");
    const excerpt = document.createElement("p");

    article.className = "search-result";
    link.href = data.url;
    link.textContent = data.meta.title;
    title.append(link);
    excerpt.innerHTML = data.excerpt;
    article.append(title, excerpt);
    return article;
  }

  async function runSearch(query, immediate = false) {
    const normalized = query.trim();
    clearButton.hidden = !input.value;

    if (!normalized) {
      resetSearch();
      return;
    }

    const currentRequest = ++requestId;
    status.textContent = "正在搜索……";

    try {
      const response = immediate
        ? await pagefind.search(normalized)
        : await pagefind.debouncedSearch(normalized);
      if (currentRequest !== requestId || response === null) return;

      const matches = await Promise.all(response.results.map((result) => result.data()));
      if (currentRequest !== requestId) return;

      results.replaceChildren(...matches.map(resultCard));
      status.textContent = matches.length
        ? `找到 ${matches.length} 篇相关文字`
        : "没有找到相关文字。也许它还没有落进这座微蓝的庭院里。";
    } catch (error) {
      if (currentRequest !== requestId) return;
      console.error("Pagefind search failed:", error);
      results.replaceChildren();
      status.textContent = "搜索暂时不可用，请稍后再试。";
    }
  }

  input.addEventListener("input", () => runSearch(input.value));

  input.addEventListener("search", () => {
    if (!input.value) setQueryInUrl("");
  });

  form.addEventListener("submit", (event) => {
    event.preventDefault();
    const query = input.value.trim();
    setQueryInUrl(query);
    runSearch(query, true);
  });

  clearButton.addEventListener("click", () => {
    input.value = "";
    setQueryInUrl("");
    resetSearch();
    input.focus();
  });

  const initialQuery = new URLSearchParams(window.location.search).get("q") || "";
  input.value = initialQuery;
  if (initialQuery) runSearch(initialQuery, true);
}

(() => {
  const input = document.querySelector("#search-input");
  const form = document.querySelector(".search-form");
  const status = document.querySelector("#search-status");
  const results = document.querySelector("#search-results");
  const indexElement = document.querySelector("#search-index");

  if (!input || !form || !status || !results || !indexElement) return;

  const pages = JSON.parse(indexElement.textContent).map((page) => ({
    ...page,
    searchText: [page.title, page.content, ...page.tags]
      .join(" ")
      .toLocaleLowerCase(),
  }));

  function resultCard(page) {
    const article = document.createElement("article");
    const title = document.createElement("h2");
    const link = document.createElement("a");
    const meta = document.createElement("p");
    const summary = document.createElement("p");

    article.className = "search-result";
    link.href = page.url;
    link.textContent = page.title;
    title.append(link);
    meta.className = "search-result-meta";
    meta.textContent = [page.date, ...page.tags.map((tag) => `#${tag}`)].join(" · ");
    summary.textContent = page.summary;
    article.append(title, meta, summary);
    return article;
  }

  function search(query) {
    const normalized = query.trim().toLocaleLowerCase();
    results.replaceChildren();

    if (!normalized) {
      status.textContent = "输入关键词开始搜索。";
      return;
    }

    const terms = normalized.split(/\s+/);
    const matches = pages.filter((page) => terms.every((term) => page.searchText.includes(term)));
    status.textContent = matches.length ? `找到 ${matches.length} 篇文章` : `没有找到与“${query.trim()}”相关的文章。`;
    results.append(...matches.map(resultCard));
  }

  form.addEventListener("submit", (event) => {
    event.preventDefault();
    const url = new URL(window.location.href);
    const query = input.value.trim();
    query ? url.searchParams.set("q", query) : url.searchParams.delete("q");
    window.history.replaceState(null, "", url);
    search(query);
  });

  input.addEventListener("search", () => {
    if (!input.value) search("");
  });

  const initialQuery = new URLSearchParams(window.location.search).get("q") || "";
  input.value = initialQuery;
  search(initialQuery);
})();

const root = document.querySelector("[data-about-stats]");

if (root) {
  const summaryURL = root.dataset.summaryUrl;
  const cacheKey = "suzuka:guestbook-summary:v1";
  const viewsEl = root.querySelector("[data-stat-views]");
  const reactionsEl = root.querySelector("[data-stat-reactions]");
  const formatter = new Intl.NumberFormat();

  function render(data) {
    if (viewsEl && Number.isFinite(data.views)) viewsEl.textContent = formatter.format(data.views);
    if (reactionsEl && Number.isFinite(data.reactions)) reactionsEl.textContent = formatter.format(data.reactions);
  }

  try {
    const cached = JSON.parse(window.sessionStorage.getItem(cacheKey));
    if (cached) render(cached);
  } catch {
    /* 缓存读不到就等接口 */
  }

  fetch(summaryURL, { headers: { Accept: "application/json" } })
    .then((response) => (response.ok ? response.json() : Promise.reject()))
    .then((data) => {
      render(data);
      try {
        window.sessionStorage.setItem(cacheKey, JSON.stringify({ views: data.views, reactions: data.reactions }));
      } catch {
        /* 私密模式等 sessionStorage 不可写时静默放弃 */
      }
    })
    .catch(() => {
      /* 汇总暂时取不到时保持占位符，不打扰页面 */
    });
}

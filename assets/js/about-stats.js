const root = document.querySelector("[data-about-stats]");

if (root) {
  const summaryURL = root.dataset.summaryUrl;
  const viewsEl = root.querySelector("[data-stat-views]");
  const reactionsEl = root.querySelector("[data-stat-reactions]");
  const formatter = new Intl.NumberFormat();

  fetch(summaryURL, { headers: { Accept: "application/json" } })
    .then((response) => (response.ok ? response.json() : Promise.reject()))
    .then((data) => {
      if (viewsEl) viewsEl.textContent = formatter.format(data.views);
      if (reactionsEl) reactionsEl.textContent = formatter.format(data.reactions);
    })
    .catch(() => {
      /* 汇总暂时取不到时保持占位符，不打扰页面 */
    });
}

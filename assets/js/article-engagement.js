const root = document.querySelector("[data-article-engagement]");

if (root) {
  const path = root.dataset.path;
  const viewsURL = root.dataset.viewsUrl;
  const reactionsURL = root.dataset.reactionsUrl;
  // 同一访客在该窗口内重复打开文章只读取、不再计入阅读数，避免刷新刷量。
  const viewWindowMs = 30 * 60 * 1000;

  const viewsEl = document.querySelector("[data-article-views]");
  const viewsTemplate = viewsEl ? viewsEl.dataset.template || "" : "";
  const reactionButton = root.querySelector("[data-reaction-button]");
  const reactionCountEl = root.querySelector("[data-reaction-count]");

  const formatCount = (template, count) =>
    template.replace("{count}", new Intl.NumberFormat().format(count));

  const requestJSON = async (url, options) => {
    const response = await fetch(url, options);
    if (!response.ok) throw new Error(`request failed: ${response.status}`);
    return response.json();
  };

  const readStorage = (key) => {
    try {
      return localStorage.getItem(key);
    } catch {
      return null;
    }
  };

  const writeStorage = (key, value) => {
    try {
      localStorage.setItem(key, value);
    } catch {
      /* localStorage 不可用时静默降级 */
    }
  };

  const showViews = (count) => {
    if (!viewsEl) return;
    viewsEl.textContent = viewsTemplate ? formatCount(viewsTemplate, count) : String(count);
    viewsEl.hidden = false;
  };

  const trackView = async () => {
    if (!viewsEl) return;
    const key = `views:${path}`;
    const last = Number(readStorage(key));
    const fresh = !last || Date.now() - last > viewWindowMs;
    try {
      const data = fresh
        ? await requestJSON(viewsURL, {
            method: "POST",
            headers: { "Content-Type": "application/json" },
            body: JSON.stringify({ path }),
          })
        : await requestJSON(`${viewsURL}?path=${encodeURIComponent(path)}`, {
            headers: { Accept: "application/json" },
          });
      if (fresh) writeStorage(key, String(Date.now()));
      showViews(data.count);
    } catch {
      /* 计数失败不打扰阅读，保持隐藏 */
    }
  };

  const showReactions = (count) => {
    if (reactionCountEl) reactionCountEl.textContent = new Intl.NumberFormat().format(count);
  };

  const markReacted = () => {
    if (reactionButton) {
      reactionButton.classList.add("is-active");
      reactionButton.disabled = true;
      reactionButton.setAttribute("aria-pressed", "true");
    }
  };

  const setupReactions = async () => {
    if (!reactionButton) return;
    const likedKey = `liked:${path}`;
    const alreadyLiked = readStorage(likedKey) === "1";
    if (alreadyLiked) markReacted();

    const fetchCount = () =>
      requestJSON(`${reactionsURL}?path=${encodeURIComponent(path)}`, {
        headers: { Accept: "application/json" },
      });

    try {
      showReactions((await fetchCount()).count);
    } catch {
      // 初次读取失败重试一次（可能是瞬时网络抖动）；两次都失败也照样把按钮
      // 挂上——只是数字读不到，不代表点赞本身也应该在本次访问里失效。
      try {
        await new Promise((resolve) => window.setTimeout(resolve, 1000));
        showReactions((await fetchCount()).count);
      } catch {
        /* 计数留空，点赞按钮照常可用 */
      }
    }
    root.hidden = false;

    reactionButton.addEventListener("click", async () => {
      if (reactionButton.disabled) return;
      reactionButton.disabled = true;
      try {
        const data = await requestJSON(reactionsURL, {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ path }),
        });
        showReactions(data.count);
        writeStorage(likedKey, "1");
        markReacted();
      } catch {
        reactionButton.disabled = false;
      }
    });
  };

  trackView();
  setupReactions();
}

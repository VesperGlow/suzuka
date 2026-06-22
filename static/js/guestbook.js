const root = document.querySelector("[data-guestbook-app]");

if (root) {
  const apiURL = root.dataset.apiUrl;
  const postsURL = root.dataset.postsUrl;
  const form = root.querySelector("[data-guestbook-form]");
  const submitButton = form.querySelector('[type="submit"]');
  const formStatus = root.querySelector("[data-guestbook-form-status]");
  const listStatus = root.querySelector("[data-guestbook-list-status]");
  const messageList = root.querySelector("[data-guestbook-message-list]");
  const count = root.querySelector("[data-guestbook-count]");
  const reference = root.querySelector("[data-guestbook-reference]");
  const referenceLink = root.querySelector("[data-guestbook-reference-link]");
  const referenceClear = root.querySelector("[data-guestbook-reference-clear]");
  const postPickerToggle = root.querySelector("[data-guestbook-post-picker-toggle]");
  const postPicker = root.querySelector("[data-guestbook-post-picker]");
  const postSearch = root.querySelector("[data-guestbook-post-search]");
  const postPickerStatus = root.querySelector("[data-guestbook-post-picker-status]");
  const postResults = root.querySelector("[data-guestbook-post-results]");

  let messages = [];
  let posts = null;
  let source = readSource();

  function postURL(value) {
    if (!value) return null;
    try {
      if (!String(value).startsWith("/") || String(value).startsWith("//")) return null;
      const url = new URL(value, window.location.origin);
      if (url.origin !== window.location.origin || !url.pathname.startsWith("/posts/")) return null;
      return url.pathname;
    } catch {
      return null;
    }
  }

  function externalURL(value) {
    if (!value) return null;
    try {
      const url = new URL(value);
      return url.protocol === "http:" || url.protocol === "https:" ? url.href : null;
    } catch {
      return null;
    }
  }

  function readSource() {
    const params = new URLSearchParams(window.location.search);
    const title = (params.get("ref_title") || "").trim().slice(0, 200);
    const url = postURL((params.get("ref_url") || "").trim().slice(0, 300));
    return title && url ? { title, url } : null;
  }

  function renderSource() {
    reference.hidden = !source;
    referenceLink.replaceChildren();
    if (!source) return;
    referenceLink.textContent = `《${source.title}》`;
    referenceLink.href = source.url;
  }

  function clearSource() {
    source = null;
    renderSource();
    const url = new URL(window.location.href);
    url.searchParams.delete("ref_title");
    url.searchParams.delete("ref_url");
    window.history.replaceState(null, "", `${url.pathname}${url.search}${url.hash}`);
  }

  function setSource(post) {
    const title = String(post.title || "").trim().slice(0, 200);
    const url = postURL(post.url);
    if (!title || !url) return;
    source = { title, url };
    renderSource();
    setPickerOpen(false);
  }

  function setPickerOpen(open) {
    postPicker.hidden = !open;
    postPickerToggle.setAttribute("aria-expanded", String(open));
    if (open) {
      loadPosts();
      postSearch.focus();
    }
  }

  function renderPosts() {
    const query = postSearch.value.trim().toLocaleLowerCase("zh-CN");
    const matches = (posts || [])
      .filter((post) => !query || post.title.toLocaleLowerCase("zh-CN").includes(query))
      .slice(0, 10);

    const elements = matches.map((post) => {
      const item = document.createElement("li");
      const button = document.createElement("button");
      const title = document.createElement("span");
      const date = document.createElement("time");
      button.type = "button";
      title.textContent = post.title;
      date.textContent = post.date;
      date.dateTime = post.date;
      button.append(title, date);
      button.addEventListener("click", () => setSource(post));
      item.append(button);
      return item;
    });
    postResults.replaceChildren(...elements);
    postPickerStatus.textContent = matches.length ? "" : "没有找到相关文章。";
  }

  async function loadPosts() {
    if (posts !== null) {
      renderPosts();
      return;
    }
    postPickerStatus.textContent = "正在读取文章…";
    try {
      const response = await fetch(postsURL, { headers: { Accept: "application/json" } });
      if (!response.ok) throw new Error(`HTTP ${response.status}`);
      const payload = await response.json();
      if (!Array.isArray(payload.items)) throw new Error("Invalid response");
      posts = payload.items.flatMap((item) => {
        const title = String(item.title || "").trim().slice(0, 200);
        const url = postURL(item.url);
        const date = String(item.date || "").slice(0, 10);
        return title && url ? [{ title, url, date }] : [];
      });
      renderPosts();
    } catch (error) {
      console.error("Unable to load guestbook posts", error);
      postPickerStatus.textContent = "文章列表暂时无法读取，请稍后再试。";
    }
  }

  function createMessageElement(item) {
    const listItem = document.createElement("li");
    listItem.className = "guestbook-message";

    const article = document.createElement("article");
    const header = document.createElement("header");
    header.className = "guestbook-message-header";

    const website = externalURL(item.website);
    const author = website ? document.createElement("a") : document.createElement("span");
    author.className = "guestbook-message-author";
    author.textContent = String(item.name || "");
    if (website) {
      author.href = website;
      author.target = "_blank";
      author.rel = "nofollow noreferrer noopener";
    }

    const time = document.createElement("time");
    time.className = "guestbook-message-time";
    const date = new Date(item.created_at);
    time.textContent = Number.isNaN(date.getTime())
      ? String(item.created_at || "")
      : new Intl.DateTimeFormat("zh-CN", { dateStyle: "medium", timeStyle: "short" }).format(date);
    if (!Number.isNaN(date.getTime())) time.dateTime = date.toISOString();
    header.append(author, time);

    const content = document.createElement("p");
    content.className = "guestbook-message-content";
    content.textContent = String(item.content || "");
    article.append(header, content);

    const refURL = postURL(item.ref_url);
    if (item.ref_title && refURL) {
      const ref = document.createElement("p");
      ref.className = "guestbook-message-reference";
      const label = document.createElement("span");
      label.textContent = "评论：";
      const link = document.createElement("a");
      link.textContent = `《${String(item.ref_title)}》`;
      link.href = refURL;
      ref.append(label, link);
      article.append(ref);
    }

    listItem.append(article);
    return listItem;
  }

  function renderMessages() {
    messageList.replaceChildren(...messages.map(createMessageElement));
    count.textContent = messages.length ? `${messages.length} 条` : "";
    listStatus.hidden = messages.length > 0;
    if (!messages.length) listStatus.textContent = "还没有留言。";
  }

  async function loadMessages() {
    try {
      const response = await fetch(apiURL, { headers: { Accept: "application/json" } });
      if (!response.ok) throw new Error(`HTTP ${response.status}`);
      const payload = await response.json();
      if (!Array.isArray(payload)) throw new Error("Invalid response");
      messages = payload;
      renderMessages();
    } catch (error) {
      console.error("Unable to load guestbook messages", error);
      listStatus.hidden = false;
      listStatus.textContent = "留言暂时无法读取，请稍后再试。";
    }
  }

  form.addEventListener("submit", async (event) => {
    event.preventDefault();
    if (!form.reportValidity()) return;

    const data = new FormData(form);
    const payload = {
      name: String(data.get("name") || "").trim(),
      email: String(data.get("email") || "").trim(),
      website: String(data.get("website") || "").trim(),
      content: String(data.get("content") || "").trim(),
      ref_title: source?.title || "",
      ref_url: source?.url || "",
    };

    submitButton.disabled = true;
    formStatus.textContent = "正在提交…";
    try {
      const response = await fetch(apiURL, {
        method: "POST",
        headers: { "Content-Type": "application/json", Accept: "application/json" },
        body: JSON.stringify(payload),
      });
      const result = await response.json().catch(() => ({}));
      if (!response.ok) throw new Error(result.error || `HTTP ${response.status}`);

      messages.unshift(result);
      renderMessages();
      form.elements.content.value = "";
      formStatus.textContent = "留言已留下。";
    } catch (error) {
      console.error("Unable to submit guestbook message", error);
      formStatus.textContent = "提交失败，请稍后再试。";
    } finally {
      submitButton.disabled = false;
    }
  });

  referenceClear.addEventListener("click", clearSource);
  postPickerToggle.addEventListener("click", () => setPickerOpen(postPicker.hidden));
  postSearch.addEventListener("input", renderPosts);
  document.addEventListener("click", (event) => {
    if (postPicker.hidden || postPicker.contains(event.target) || postPickerToggle.contains(event.target)) return;
    setPickerOpen(false);
  });
  document.addEventListener("keydown", (event) => {
    if (event.key !== "Escape" || postPicker.hidden) return;
    setPickerOpen(false);
    postPickerToggle.focus();
  });
  renderSource();
  loadMessages();
}

const root = document.querySelector("[data-guestbook-app]");

if (root) {
  const apiURL = root.dataset.apiUrl;
  const postsURL = root.dataset.postsUrl;
  const form = root.querySelector("[data-guestbook-form]");
  const submitButton = form.querySelector('[type="submit"]');
  const formStatus = root.querySelector("[data-guestbook-form-status]");
  const listStatus = root.querySelector("[data-guestbook-list-status]");
  const messageList = root.querySelector("[data-guestbook-message-list]");
  const loadMoreButton = root.querySelector("[data-guestbook-load-more]");
  const count = root.querySelector("[data-guestbook-count]");
  const reference = root.querySelector("[data-guestbook-reference]");
  const referenceLink = root.querySelector("[data-guestbook-reference-link]");
  const referenceClear = root.querySelector("[data-guestbook-reference-clear]");
  const layout = root.querySelector(".guestbook-layout");
  const pickerDrawer = root.querySelector("[data-guestbook-picker-drawer]");
  const postPickerToggle = root.querySelector("[data-guestbook-post-picker-toggle]");
  const postPicker = root.querySelector("[data-guestbook-post-picker]");
  const postSearch = root.querySelector("[data-guestbook-post-search]");
  const postPickerStatus = root.querySelector("[data-guestbook-post-picker-status]");
  const postResults = root.querySelector("[data-guestbook-post-results]");
  const pickerDesktopQuery = window.matchMedia("(min-width: 960px)");
  const pickerAutoFocusQuery = window.matchMedia("(min-width: 960px) and (hover: hover) and (pointer: fine)");
  const locale = document.documentElement.lang || "zh-CN";
  const labels = {
    postsEmpty: root.dataset.i18nPostsEmpty,
    postsLoading: root.dataset.i18nPostsLoading,
    postsUnavailable: root.dataset.i18nPostsUnavailable,
    commentOn: root.dataset.i18nCommentOn,
    referenceTitle: root.dataset.i18nReferenceTitle,
    countOne: root.dataset.i18nCountOne,
    countOther: root.dataset.i18nCountOther,
    messagesEmpty: root.dataset.i18nMessagesEmpty,
    messagesUnavailable: root.dataset.i18nMessagesUnavailable,
    loadMore: root.dataset.i18nLoadMore,
    submitting: root.dataset.i18nSubmitting,
    submitted: root.dataset.i18nSubmitted,
    submitFailed: root.dataset.i18nSubmitFailed,
    guest: root.dataset.i18nGuest,
  };

  let messages = [];
  let totalMessages = 0;
  let nextBeforeID = 0;
  let posts = null;
  let source = readSource();
  let pickerCloseTimer = 0;

  function postURL(value) {
    if (!value) return null;
    try {
      if (!String(value).startsWith("/") || String(value).startsWith("//")) return null;
      const url = new URL(value, window.location.origin);
      if (url.origin !== window.location.origin || !/^\/(?:[a-z]{2}(?:-[a-z]{2})?\/)?posts\//i.test(url.pathname)) return null;
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
    referenceLink.textContent = labels.referenceTitle.replace("{title}", source.title);
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
    window.clearTimeout(pickerCloseTimer);
    postPickerToggle.setAttribute("aria-expanded", String(open));
    if (open) {
      postPicker.hidden = false;
      requestAnimationFrame(() => layout.classList.add("is-picker-open"));
      loadPosts();
      if (pickerAutoFocusQuery.matches) {
        requestAnimationFrame(() => {
          if (layout.classList.contains("is-picker-open")) postSearch.focus({ preventScroll: true });
        });
      }
      return;
    }

    layout.classList.remove("is-picker-open");
    if (!pickerDesktopQuery.matches) {
      postPicker.hidden = true;
      return;
    }
    pickerCloseTimer = window.setTimeout(() => {
      if (!layout.classList.contains("is-picker-open")) postPicker.hidden = true;
    }, 260);
  }

  function renderPosts() {
    const query = postSearch.value.trim().toLocaleLowerCase(locale);
    const matches = (posts || [])
      .filter((post) => !query || post.title.toLocaleLowerCase(locale).includes(query))
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
    postPickerStatus.textContent = matches.length ? "" : labels.postsEmpty;
  }

  async function loadPosts() {
    if (posts !== null) {
      renderPosts();
      return;
    }
    postPickerStatus.textContent = labels.postsLoading;
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
      postPickerStatus.textContent = labels.postsUnavailable;
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
    author.textContent = String(item.name || labels.guest);
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
      : new Intl.DateTimeFormat(locale, { dateStyle: "medium", timeStyle: "short" }).format(date);
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
      label.textContent = labels.commentOn;
      const link = document.createElement("a");
      link.textContent = labels.referenceTitle.replace("{title}", String(item.ref_title));
      link.href = refURL;
      ref.append(label, link);
      article.append(ref);
    }

    listItem.append(article);
    return listItem;
  }

  function renderMessages() {
    messageList.replaceChildren(...messages.map(createMessageElement));
    const countTemplate = totalMessages === 1 ? labels.countOne : labels.countOther;
    count.textContent = totalMessages ? countTemplate.replace("{count}", String(totalMessages)) : "";
    listStatus.hidden = messages.length > 0;
    if (!messages.length) listStatus.textContent = labels.messagesEmpty;
    loadMoreButton.hidden = !nextBeforeID;
  }

  async function loadMessages(append = false) {
    if (append && !nextBeforeID) return;
    loadMoreButton.disabled = append;
    try {
      const requestURL = new URL(apiURL, window.location.origin);
      requestURL.searchParams.set("limit", "50");
      if (append) requestURL.searchParams.set("before_id", String(nextBeforeID));
      const response = await fetch(requestURL, { headers: { Accept: "application/json" } });
      if (!response.ok) throw new Error(`HTTP ${response.status}`);
      const payload = await response.json();
      if (!payload || !Array.isArray(payload.messages)) throw new Error("Invalid response");
      messages = append ? messages.concat(payload.messages) : payload.messages;
      totalMessages = Number.isSafeInteger(payload.total_count) ? payload.total_count : messages.length;
      nextBeforeID = Number.isSafeInteger(payload.next_before_id) ? payload.next_before_id : 0;
      renderMessages();
    } catch (error) {
      console.error("Unable to load guestbook messages", error);
      if (!append) {
        listStatus.hidden = false;
        listStatus.textContent = labels.messagesUnavailable;
      }
    } finally {
      loadMoreButton.disabled = false;
      loadMoreButton.textContent = labels.loadMore;
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
    formStatus.textContent = labels.submitting;
    try {
      const response = await fetch(apiURL, {
        method: "POST",
        headers: { "Content-Type": "application/json", Accept: "application/json" },
        body: JSON.stringify(payload),
      });
      const result = await response.json().catch(() => ({}));
      if (!response.ok) throw new Error(result.error || `HTTP ${response.status}`);

      messages.unshift(result);
      totalMessages += 1;
      renderMessages();
      form.elements.content.value = "";
      formStatus.textContent = labels.submitted;
    } catch (error) {
      console.error("Unable to submit guestbook message", error);
      formStatus.textContent = labels.submitFailed;
    } finally {
      submitButton.disabled = false;
    }
  });

  referenceClear.addEventListener("click", clearSource);
  postPickerToggle.addEventListener("click", () => setPickerOpen(!layout.classList.contains("is-picker-open")));
  postSearch.addEventListener("input", renderPosts);
  loadMoreButton.addEventListener("click", () => loadMessages(true));
  document.addEventListener("click", (event) => {
    if (!layout.classList.contains("is-picker-open") || pickerDrawer.contains(event.target) || postPickerToggle.contains(event.target)) return;
    setPickerOpen(false);
  });
  document.addEventListener("keydown", (event) => {
    if (event.key !== "Escape" || !layout.classList.contains("is-picker-open")) return;
    setPickerOpen(false);
    postPickerToggle.focus();
  });
  renderSource();
  loadMessages();
}

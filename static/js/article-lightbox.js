const articleContent = document.querySelector(".post-article .prose");
const lightbox = document.querySelector("[data-article-lightbox]");

if (articleContent && lightbox && typeof lightbox.showModal === "function") {
  const viewer = lightbox.querySelector("[data-article-lightbox-viewer]");
  const previewImage = lightbox.querySelector("[data-article-lightbox-image]");
  const closeButton = lightbox.querySelector("[data-article-lightbox-close]");
  const zoomInButton = lightbox.querySelector("[data-article-lightbox-zoom-in]");
  const zoomOutButton = lightbox.querySelector("[data-article-lightbox-zoom-out]");
  const resetButton = lightbox.querySelector("[data-article-lightbox-reset]");
  const pointers = new Map();
  const minScale = 1;
  const maxScale = 5;
  let scale = minScale;
  let translateX = 0;
  let translateY = 0;
  let trigger = null;
  let frame = 0;
  let transitionTimer = 0;
  let suppressClickTimer = 0;
  let gesture = null;
  let moved = false;
  let suppressBackgroundClick = false;
  let lastTap = null;

  function imageLinkFor(image) {
    const link = image.closest("a[href]");
    if (!link) return null;

    try {
      const url = new URL(link.href, window.location.href);
      return /\.(?:avif|gif|jpe?g|png|svg|webp)$/i.test(url.pathname) ? url.href : null;
    } catch {
      return null;
    }
  }

  function clamp(value, minimum, maximum) {
    return Math.min(maximum, Math.max(minimum, value));
  }

  function clampTranslation() {
    if (scale <= minScale) {
      translateX = 0;
      translateY = 0;
      return;
    }

    const maxX = Math.max(0, (previewImage.offsetWidth * scale - viewer.clientWidth) / 2);
    const maxY = Math.max(0, (previewImage.offsetHeight * scale - viewer.clientHeight) / 2);
    translateX = clamp(translateX, -maxX, maxX);
    translateY = clamp(translateY, -maxY, maxY);
  }

  function renderTransform() {
    cancelAnimationFrame(frame);
    frame = requestAnimationFrame(() => {
      previewImage.style.transform = `translate3d(${translateX}px, ${translateY}px, 0) scale(${scale})`;
      viewer.classList.toggle("is-zoomed", scale > minScale);
    });
  }

  function animateTransform() {
    clearTimeout(transitionTimer);
    previewImage.classList.add("is-transforming");
    transitionTimer = window.setTimeout(() => previewImage.classList.remove("is-transforming"), 180);
    renderTransform();
  }

  function setScale(nextScale, clientX = viewer.clientWidth / 2, clientY = viewer.clientHeight / 2, animate = false) {
    const previousScale = scale;
    const rect = viewer.getBoundingClientRect();
    const focalX = clientX - rect.left - rect.width / 2;
    const focalY = clientY - rect.top - rect.height / 2;
    scale = clamp(nextScale, minScale, maxScale);

    if (scale <= minScale) {
      translateX = 0;
      translateY = 0;
    } else {
      const ratio = scale / previousScale;
      translateX = focalX - (focalX - translateX) * ratio;
      translateY = focalY - (focalY - translateY) * ratio;
      clampTranslation();
    }

    animate ? animateTransform() : renderTransform();
  }

  function resetTransform(animate = false) {
    scale = minScale;
    translateX = 0;
    translateY = 0;
    animate ? animateTransform() : renderTransform();
  }

  function toggleZoom(clientX, clientY) {
    setScale(scale > minScale ? minScale : 2.5, clientX, clientY, true);
  }

  function points() {
    return [...pointers.values()];
  }

  function distanceBetween(first, second) {
    return Math.hypot(second.x - first.x, second.y - first.y);
  }

  function midpoint(first, second) {
    return { x: (first.x + second.x) / 2, y: (first.y + second.y) / 2 };
  }

  function startGesture() {
    const active = points();
    moved = false;

    if (active.length >= 2) {
      const center = midpoint(active[0], active[1]);
      gesture = {
        type: "pinch",
        distance: Math.max(1, distanceBetween(active[0], active[1])),
        center,
        scale,
        translateX,
        translateY,
      };
      lastTap = null;
    } else if (active.length === 1) {
      gesture = {
        type: "drag",
        x: active[0].x,
        y: active[0].y,
        translateX,
        translateY,
      };
    }
  }

  function suppressNextBackgroundClick() {
    clearTimeout(suppressClickTimer);
    suppressBackgroundClick = true;
    suppressClickTimer = window.setTimeout(() => {
      suppressBackgroundClick = false;
    }, 0);
  }

  function handlePointerDown(event) {
    if (event.pointerType === "mouse" && event.button !== 0) return;
    event.preventDefault();
    pointers.set(event.pointerId, { x: event.clientX, y: event.clientY, startedOnImage: event.target === previewImage });
    viewer.setPointerCapture(event.pointerId);
    previewImage.classList.remove("is-transforming");
    startGesture();
  }

  function handlePointerMove(event) {
    if (!pointers.has(event.pointerId)) return;
    event.preventDefault();
    pointers.set(event.pointerId, { ...pointers.get(event.pointerId), x: event.clientX, y: event.clientY });
    const active = points();

    if (active.length >= 2 && gesture?.type === "pinch") {
      const center = midpoint(active[0], active[1]);
      const nextScale = clamp(gesture.scale * distanceBetween(active[0], active[1]) / gesture.distance, minScale, maxScale);
      const rect = viewer.getBoundingClientRect();
      const startX = gesture.center.x - rect.left - rect.width / 2;
      const startY = gesture.center.y - rect.top - rect.height / 2;
      const currentX = center.x - rect.left - rect.width / 2;
      const currentY = center.y - rect.top - rect.height / 2;
      const ratio = nextScale / gesture.scale;
      scale = nextScale;
      translateX = currentX - (startX - gesture.translateX) * ratio;
      translateY = currentY - (startY - gesture.translateY) * ratio;
      clampTranslation();
      moved = true;
      suppressBackgroundClick = true;
      renderTransform();
      return;
    }

    if (active.length === 1 && gesture?.type === "drag" && scale > minScale) {
      const deltaX = active[0].x - gesture.x;
      const deltaY = active[0].y - gesture.y;
      if (Math.hypot(deltaX, deltaY) > 3) moved = true;
      translateX = gesture.translateX + deltaX;
      translateY = gesture.translateY + deltaY;
      clampTranslation();
      if (moved) suppressBackgroundClick = true;
      renderTransform();
    }
  }

  function handlePointerEnd(event) {
    if (!pointers.has(event.pointerId)) return;
    event.preventDefault();
    const pointer = pointers.get(event.pointerId);
    const endedOnImage = pointer.startedOnImage;
    const tap = { time: performance.now(), x: event.clientX, y: event.clientY, pointerType: event.pointerType };
    pointers.delete(event.pointerId);

    if (event.type === "pointerup" && endedOnImage && !moved && pointers.size === 0) {
      if (lastTap && tap.pointerType === lastTap.pointerType && tap.time - lastTap.time < 380 && Math.hypot(tap.x - lastTap.x, tap.y - lastTap.y) < 32) {
        toggleZoom(tap.x, tap.y);
        lastTap = null;
      } else {
        lastTap = tap;
      }
    }

    if (event.type === "pointerup" && (moved || endedOnImage)) suppressNextBackgroundClick();

    startGesture();
  }

  function closeLightbox() {
    if (lightbox.open) lightbox.close();
  }

  viewer.addEventListener("pointerdown", handlePointerDown, { passive: false });
  viewer.addEventListener("pointermove", handlePointerMove, { passive: false });
  viewer.addEventListener("pointerup", handlePointerEnd, { passive: false });
  viewer.addEventListener("pointercancel", handlePointerEnd, { passive: false });
  viewer.addEventListener("gesturestart", (event) => event.preventDefault(), { passive: false });
  viewer.addEventListener("gesturechange", (event) => event.preventDefault(), { passive: false });
  viewer.addEventListener("wheel", (event) => {
    event.preventDefault();
    previewImage.classList.remove("is-transforming");
    setScale(scale * Math.exp(-event.deltaY * .0015), event.clientX, event.clientY);
  }, { passive: false });
  zoomInButton.addEventListener("click", () => setScale(scale * 1.4, undefined, undefined, true));
  zoomOutButton.addEventListener("click", () => setScale(scale / 1.4, undefined, undefined, true));
  resetButton.addEventListener("click", () => resetTransform(true));
  closeButton.addEventListener("click", closeLightbox);

  articleContent.addEventListener("click", (event) => {
    if (event.button !== 0 || event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) return;
    const target = event.target;
    if (!(target instanceof Element)) return;
    const image = target instanceof HTMLImageElement
      ? target
      : target.closest("a[href]")?.querySelector("img");
    if (!image || !articleContent.contains(image)) return;

    event.preventDefault();
    trigger = image.closest("a[href]") || image;
    resetTransform();
    previewImage.src = imageLinkFor(image) || image.currentSrc || image.src;
    previewImage.alt = image.alt || "";
    document.documentElement.classList.add("lightbox-open");
    lightbox.showModal();
    closeButton.focus();
  });

  lightbox.addEventListener("click", (event) => {
    if (suppressBackgroundClick) {
      clearTimeout(suppressClickTimer);
      suppressBackgroundClick = false;
      return;
    }
    if (event.target === viewer || event.target === lightbox) closeLightbox();
  });
  lightbox.addEventListener("close", () => {
    document.documentElement.classList.remove("lightbox-open");
    pointers.clear();
    gesture = null;
    lastTap = null;
    suppressBackgroundClick = false;
    cancelAnimationFrame(frame);
    clearTimeout(transitionTimer);
    clearTimeout(suppressClickTimer);
    previewImage.classList.remove("is-transforming");
    previewImage.removeAttribute("style");
    previewImage.removeAttribute("src");
    previewImage.alt = "";
    trigger?.focus({ preventScroll: true });
    trigger = null;
  });
  previewImage.addEventListener("load", () => resetTransform());
  window.addEventListener("resize", () => {
    if (!lightbox.open) return;
    clampTranslation();
    renderTransform();
  });
}

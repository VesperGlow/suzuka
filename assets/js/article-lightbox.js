const articleContent = document.querySelector(".post-article .prose");
const lightbox = document.querySelector("[data-article-lightbox]");

if (articleContent && lightbox && typeof lightbox.showModal === "function") {
  const viewer = lightbox.querySelector("[data-article-lightbox-viewer]");
  const previewImage = lightbox.querySelector("[data-article-lightbox-image]");
  const previousButton = lightbox.querySelector("[data-article-lightbox-prev]");
  const nextButton = lightbox.querySelector("[data-article-lightbox-next]");
  const mobilePreviousButton = lightbox.querySelector("[data-article-lightbox-mobile-prev]");
  const mobileNextButton = lightbox.querySelector("[data-article-lightbox-mobile-next]");
  const imageCount = lightbox.querySelector("[data-article-lightbox-count]");
  const zoomInButton = lightbox.querySelector("[data-article-lightbox-zoom-in]");
  const zoomOutButton = lightbox.querySelector("[data-article-lightbox-zoom-out]");
  const resetButton = lightbox.querySelector("[data-article-lightbox-reset]");
  const galleryImages = [...articleContent.querySelectorAll("img")];
  const requiredElements = [viewer, previewImage, previousButton, nextButton,
    mobilePreviousButton, mobileNextButton, imageCount, zoomInButton, zoomOutButton, resetButton];

  if (requiredElements.every(Boolean) && galleryImages.length) {
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
  let currentIndex = 0;

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
    const isZoomed = scale > minScale;
    lightbox.classList.toggle("is-zoomed", isZoomed);
    resetButton.textContent = isZoomed ? `${scale.toFixed(1)}×` : "1:1";
    resetButton.setAttribute("aria-label", isZoomed
      ? lightbox.dataset.resetZoomAt.replace("{scale}", scale.toFixed(1))
      : lightbox.dataset.resetZoom);
    cancelAnimationFrame(frame);
    frame = requestAnimationFrame(() => {
      previewImage.style.transform = `translate3d(${translateX}px, ${translateY}px, 0) scale(${scale})`;
      viewer.classList.toggle("is-zoomed", isZoomed);
    });
  }

  function positionGalleryNavigation() {
    lightbox.style.setProperty("--lightbox-image-width", `${previewImage.offsetWidth}px`);
    const sideGutter = Math.max(0, (viewer.clientWidth - previewImage.offsetWidth) / 2);
    const buttonWidth = previousButton.offsetWidth || 64;
    const navSide = Math.max(12, (sideGutter - buttonWidth) / 2);
    lightbox.style.setProperty("--lightbox-nav-side", `${navSide}px`);
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

  function updateGalleryControls() {
    const hasMultipleImages = galleryImages.length > 1;
    previousButton.hidden = !hasMultipleImages;
    nextButton.hidden = !hasMultipleImages;
    mobilePreviousButton.hidden = !hasMultipleImages;
    mobileNextButton.hidden = !hasMultipleImages;
    imageCount.hidden = !hasMultipleImages;
    imageCount.textContent = hasMultipleImages ? `${currentIndex + 1} / ${galleryImages.length}` : "";
  }

  function showImage(index) {
    currentIndex = (index + galleryImages.length) % galleryImages.length;
    const image = galleryImages[currentIndex];
    resetTransform();
    // 用 image.src（原图）而不是 currentSrc：正文 srcset 选中的缩放变体
    // 不够灯箱放大看，灯箱始终加载全尺寸原图
    previewImage.src = imageLinkFor(image) || image.src;
    previewImage.alt = image.alt || "";
    // 新图未就绪时旧图会滞留在 <img> 上，先遮住并显示加载指示
    lightbox.classList.toggle("is-loading", !previewImage.complete);
    updateGalleryControls();

    // 相邻图预取的是多 MB 的全尺寸原图，省流模式/慢网络下不做
    const connection = navigator.connection;
    const avoidPreload =
      connection?.saveData ||
      connection?.effectiveType === "slow-2g" ||
      connection?.effectiveType === "2g";
    if (galleryImages.length > 1 && !avoidPreload) {
      for (const offset of [1, -1]) {
        const adjacent = galleryImages[(currentIndex + offset + galleryImages.length) % galleryImages.length];
        new Image().src = imageLinkFor(adjacent) || adjacent.src;
      }
    }
  }

  function changeImage(direction) {
    if (galleryImages.length < 2) return;
    showImage(currentIndex + direction);
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
    if (event.target instanceof Element && event.target.closest(".article-lightbox-nav")) return;
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
    const swipeX = gesture?.type === "drag" ? event.clientX - gesture.x : 0;
    const swipeY = gesture?.type === "drag" ? event.clientY - gesture.y : 0;
    const tap = { time: performance.now(), x: event.clientX, y: event.clientY, pointerType: event.pointerType };
    pointers.delete(event.pointerId);

    if (event.type === "pointerup" && scale === minScale && pointers.size === 0 && Math.abs(swipeX) > 60 && Math.abs(swipeX) > Math.abs(swipeY) * 1.25) {
      moved = true;
      lastTap = null;
      changeImage(swipeX < 0 ? 1 : -1);
    }

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
  previousButton.addEventListener("click", () => changeImage(-1));
  nextButton.addEventListener("click", () => changeImage(1));
  mobilePreviousButton.addEventListener("click", () => changeImage(-1));
  mobileNextButton.addEventListener("click", () => changeImage(1));

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
    showImage(galleryImages.indexOf(image));
    document.documentElement.classList.add("lightbox-open");
    lightbox.showModal();
  });

  lightbox.addEventListener("keydown", (event) => {
    if (event.key === "ArrowLeft" || event.key === "ArrowRight") {
      event.preventDefault();
      changeImage(event.key === "ArrowLeft" ? -1 : 1);
    } else if (event.key === "+" || event.key === "=") {
      event.preventDefault();
      setScale(scale * 1.4, undefined, undefined, true);
    } else if (event.key === "-") {
      event.preventDefault();
      setScale(scale / 1.4, undefined, undefined, true);
    } else if (event.key === "0") {
      event.preventDefault();
      resetTransform(true);
    }
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
    lightbox.classList.remove("is-zoomed");
    lightbox.classList.remove("is-loading");
    previewImage.removeAttribute("style");
    previewImage.removeAttribute("src");
    previewImage.alt = "";
    trigger?.focus({ preventScroll: true });
    trigger = null;
  });
  previewImage.addEventListener("load", () => {
    lightbox.classList.remove("is-loading");
    resetTransform();
    positionGalleryNavigation();
  });
  previewImage.addEventListener("error", () => lightbox.classList.remove("is-loading"));
  window.addEventListener("resize", () => {
    if (!lightbox.open) return;
    clampTranslation();
    renderTransform();
    positionGalleryNavigation();
  });
  }
}

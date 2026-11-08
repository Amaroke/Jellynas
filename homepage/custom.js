window.addEventListener("message", (event) => {
  if (!event.data || !("ratioBoardHeight" in event.data)) return;
  const height = event.data.ratioBoardHeight;
  for (const frame of document.querySelectorAll("iframe")) {
    if (frame.contentWindow !== event.source) continue;
    frame.style.height = Number.isFinite(height) && height > 0 && height < 10000 ? `${Math.ceil(height)}px` : "";
  }
});

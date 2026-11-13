// The host owns horizontal workspace navigation. Vertical page scrolling and
// Ctrl+wheel zoom stay native. No observers, animation frames or timers.
(() => {
  if (window.top !== window) return;
  const contain = () => {
    document.documentElement?.style.setProperty('overscroll-behavior-x', 'none', 'important');
    document.body?.style.setProperty('overscroll-behavior-x', 'none', 'important');
  };
  document.addEventListener('DOMContentLoaded', contain, {once:true});
  window.addEventListener('wheel', event => {
    if (!event.isTrusted || event.ctrlKey || Math.abs(event.deltaX) <= Math.abs(event.deltaY)) return;
    event.preventDefault();
    const delta = event.deltaX * (event.deltaMode === 1 ? 40 : event.deltaMode === 2 ? innerWidth : 1);
    window.chrome?.webview?.postMessage({channel:'lumo.workspace-swipe', delta});
  }, {passive:false, capture:true});
})();

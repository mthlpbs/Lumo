// Observe an explicitly opened provider page. No polling, network interception,
// prompt capture, or modification of the provider's UI.
(() => {
  "use strict";
  if (window.top !== window || window.__lumoAiSite) return;
  const site = __LUMO_SITE__, host = __LUMO_HOST__;
  if (location.hostname !== host && !(host === "www.perplexity.ai" && location.hostname === "perplexity.ai") && !(site === "Muse" && ["www.muse.ai", "app.muse.ai"].includes(location.hostname)) && !(__LUMO_TEST__ && location.href === "about:blank")) return;
  window.__lumoAiSite = true;
  const adapters = {
    ChatGPT: '[data-message-author-role="assistant"]',
    Gemini: 'model-response .model-response-text, model-response message-content, model-response',
    Claude: '[data-testid="assistant-message"], .font-claude-response',
    Muse: '[data-message-author-role="assistant"], [data-message-role="assistant"], [data-role="assistant"], [data-role="agent"], [data-from="assistant"], .is-assistant, .assistant-message, .agent-message, [data-message-type="assistant"], [data-author="assistant"], [data-testid^="assistant-message"], [data-testid="agent-message"], [data-sentry-component="AssistantMessage"], [data-sentry-component="AgentMessage"], [aria-label="Muse message"]',
    "Perplexity AI": '[data-testid="answer"], [data-testid="assistant-message"], .prose',
  };
  const stopSelector = 'button[data-testid*="stop"], button[aria-label*="Stop"], button[aria-label*="stop"], button[aria-label*="Cancel response"], button[title*="Stop"], [data-is-streaming="true"], [data-is-generating="true"], [data-status="streaming"], [data-state="streaming"], [aria-busy="true"][data-message-author-role="assistant"]';
  const sendSelector = 'button[data-testid="send-button"], button[aria-label*="Send"], button[aria-label*="send"], button[aria-label*="Submit"], button[aria-label*="submit"]';
  let active = false, seenBusy = false, baseline = "", baselineNode = null, scheduled = false, startupDeadline = null, quietDeadline = null, lastText = "", lastNode = null, responseMutation=false;
  const post = (kind, text = "") => window.chrome?.webview?.postMessage({channel: "lumo.ai-site", kind, text});
  const latest = () => {
    const nodes = document.querySelectorAll(adapters[site]);
    const node = nodes[nodes.length - 1];
    if (!node) return {node, text:""};
    // Read a bounded preview without forcing layout or copying a potentially
    // enormous code block. Provider controls are excluded from the preview.
    const walker = document.createTreeWalker(node, NodeFilter.SHOW_TEXT);
    const parts = [];
    let length = 0, visited = 0, textNode;
    while (length < 1500 && visited++ < 512 && (textNode = walker.nextNode())) {
      if (textNode.parentElement?.closest('button, script, style, [aria-hidden="true"]')) continue;
      const text = (textNode.nodeValue || "").slice(0, 1500 - length);
      parts.push(text); length += text.length;
    }
    return {node, text:parts.join(" ").trim().slice(0, 1500)};
  };
  // A hidden native host may have no client rects. Generation state still
  // matters there; hidden/disabled historical controls do not count.
  const busy = () => [...document.querySelectorAll(stopSelector)].some(node => !node.hidden && node.getAttribute('aria-hidden') !== 'true' && !node.disabled && (node.matches('[data-is-streaming="true"], [data-is-generating="true"], [data-status="streaming"], [data-state="streaming"], [aria-busy="true"]') || getComputedStyle(node).display !== 'none'));
  const editorSelector='textarea, [contenteditable="true"], [role="textbox"]';
  const observer = new MutationObserver(records => {
    responseMutation ||= records.some(record => {
      if(record.type==='attributes')return false;
      const element=record.target.nodeType===1?record.target:record.target.parentElement;
      return element?.closest(adapters[site])&&!element.closest('button, script, style, [aria-hidden="true"]');
    });
    if (scheduled) return;
    scheduled = true;
    queueMicrotask(() => { scheduled = false; check(); });
  });
  function start() {
    if (active) return;
    if (!document.querySelector(editorSelector)) return;
    const reply = latest();
    active = true; seenBusy = false; baseline = reply.text; baselineNode = reply.node; lastText=reply.text; lastNode=reply.node;
    post("working");
    // Observe only after a user submits, and disconnect after the reply. Idle
    // pages, loading chat history and site animations never run our observer.
    observer.observe(document.body, {subtree:true, childList:true, characterData:true, attributes:true, attributeFilter:["aria-busy", "aria-label", "data-is-streaming", "data-is-generating", "data-status", "data-state", "disabled", "hidden", "class"]});
    // One bounded startup deadline, not a repeating poll. A rejected submit or
    // unsupported site layout must never retain a hidden browser indefinitely.
    startupDeadline = setTimeout(() => finish("idle"), site === "Muse" ? 120000 : 15000);
    check();
  }
  function finish(kind, text = "") {
    clearTimeout(startupDeadline); startupDeadline = null;clearTimeout(quietDeadline);quietDeadline=null;
    active = false; observer.disconnect(); post(kind, text);
  }
  function check() {
    if (!active) return;
    const textMutated=responseMutation;responseMutation=false;
    if (busy()) { seenBusy = true; clearTimeout(startupDeadline); startupDeadline = null; clearTimeout(quietDeadline);quietDeadline=null;return; }
    const reply = latest();
    const changed=reply.text && (reply.text !== baseline || reply.node !== baselineNode);
    if (seenBusy && changed) {finish("ready",reply.text.slice(0,1500));return;}
    if (seenBusy) {finish("idle");return;}
    // Muse can deliver a completed message without exposing a stop button.
    // Only a response that changed after an explicit submit is eligible.
    // One trailing quiet check is reset by response text, not by animations;
    // no timer remains after the reply, cancellation or the startup deadline.
    if(site === "Muse" && changed && (reply.text !== lastText || reply.node !== lastNode || textMutated)) {
      clearTimeout(startupDeadline);startupDeadline=null;
      lastText=reply.text;lastNode=reply.node;clearTimeout(quietDeadline);
      quietDeadline=setTimeout(()=>{
        quietDeadline=null;if(!active||busy())return;
        const final=latest();
        if(final.text===lastText&&final.node===lastNode&&final.text)finish("ready",final.text.slice(0,1500));
      },4000);
    }
  }
  document.addEventListener("submit", event => {
    if (event.target instanceof Element && event.target.querySelector(editorSelector)) start();
  }, true);
  document.addEventListener("pointerdown", () => post("dismiss"), true);
  document.addEventListener("click", event => {
    const target = event.target instanceof Element ? event.target.closest(sendSelector) : null;
    if (target && !target.disabled) start();
    if (event.target instanceof Element && event.target.closest(stopSelector)) {
      finish("idle");
    }
  }, true);
  document.addEventListener("keydown", event => {
    if (event.key === "Enter" && !event.shiftKey && !event.isComposing && event.target instanceof Element && event.target.closest(editorSelector) && (event.target.value || event.target.textContent || "").trim()) start();
  }, true);
  addEventListener("pagehide", () => { clearTimeout(startupDeadline);clearTimeout(quietDeadline); observer.disconnect(); if (active) post("idle"); active = false; });
})();

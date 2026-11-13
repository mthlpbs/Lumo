// Cosmetic filtering for Brave's engine. Mutation-driven, bounded batches;
// no polling, animation frame loop, network requests, or work in hidden pages.
(() => {
  'use strict';
  if (window.__lumoBlock) return;
  let active=false, generic=true, scheduled=false, initial=false, session=0;
  let actions=[], style=null;
  const classes=new Set(), ids=new Set(), selectors=new Set(), roots=new Set();
  const own=node=>node?.nodeType===1 && node.id==='__lumo_web_filters';
  const observer=new MutationObserver(records=>{
    for (const record of records) {
      if (own(record.target)) continue;
      if (record.type==='attributes') roots.add(record.target);
      else for (const node of record.addedNodes||[]) if (node.nodeType===1&&!own(node)) roots.add(node);
    }
    schedule();
  });
  const post=value=>window.chrome?.webview?.postMessage({channel:'lumo.adblocking',session,...value});
  function observe() {
    observer.disconnect();
    if (active&&!document.hidden&&document.documentElement) observer.observe(document.documentElement,{subtree:true,childList:true,attributes:true,attributeFilter:['class','id'],characterData:actions.length>0});
  }
  function schedule() {
    if (!active||document.hidden||scheduled) return;
    scheduled=true;
    queueMicrotask(()=>{scheduled=false;if(active&&!document.hidden) scan();});
  }
  function scan() {
    const newClasses=[],newIds=[];
    let visited=0;
    const collect=node=>{
      if (own(node)) return;
      if (generic) {
        for (const name of node.classList||[]) if(name.length<=160&&!classes.has(name)&&classes.size<16000){classes.add(name);newClasses.push(name);}
        const id=node.id;if(id&&id.length<=160&&!ids.has(id)&&ids.size<16000){ids.add(id);newIds.push(id);}
      }
    };
    for (const root of roots) {
      if (!root.isConnected) continue;
      collect(root);
      const walk=document.createTreeWalker(root,NodeFilter.SHOW_ELEMENT);
      let node;while(visited++<10000&&(node=walk.nextNode())) collect(node);
      if(visited>=10000) break;
    }
    roots.clear();
    if (initial||newClasses.length||newIds.length) {
      // Keep each COM message bounded, without losing tokens from a large DOM.
      let first=initial;initial=false;
      do {post({initial:first,classes:newClasses.splice(0,64),ids:newIds.splice(0,64)});first=false;}while(newClasses.length||newIds.length);
    }
    applyActions();
  }
  function addSelectors(values) {
    if(!values?.length||!document.documentElement) return;
    if(!style?.isConnected){style=document.createElement('style');style.id='__lumo_web_filters';(document.head||document.documentElement).append(style);selectors.clear();}
    for(const value of values){if(selectors.has(value))continue;try{style.sheet.insertRule(`${value}{display:none!important}`,style.sheet.cssRules.length);selectors.add(value);}catch{}}
  }
  function matches(value,text) {
    if(value.startsWith('/')){const last=value.lastIndexOf('/');if(last>0){try{return new RegExp(value.slice(1,last),value.slice(last+1)).test(text);}catch{return false;}}}
    return text.includes(value);
  }
  function select(operators) {
    let nodes=[document];
    for(const op of operators||[]){
      const arg=op.arg||'';
      try {
        switch(op.type){
          case 'css-selector':nodes=nodes.flatMap(n=>[...n.querySelectorAll(arg)]);break;
          case 'has-text':nodes=nodes.filter(n=>matches(arg,n.textContent||''));break;
          case 'min-text-length':nodes=nodes.filter(n=>(n.textContent||'').length>=Number(arg));break;
          case 'matches-path':if(!matches(arg,location.pathname+location.search))nodes=[];break;
          case 'matches-attr':{const at=arg.indexOf('=');const name=at<0?arg:arg.slice(0,at);const value=at<0?'':arg.slice(at+1);nodes=nodes.filter(n=>n.hasAttribute(name)&&matches(value,n.getAttribute(name)||''));break;}
          case 'matches-css':case 'matches-css-before':case 'matches-css-after':{const at=arg.indexOf(':');if(at<0)return [];const property=arg.slice(0,at).trim(),value=arg.slice(at+1).trim();const pseudo=op.type==='matches-css-before'?'::before':op.type==='matches-css-after'?'::after':null;nodes=nodes.filter(n=>matches(value,getComputedStyle(n,pseudo).getPropertyValue(property)));break;}
          case 'upward':nodes=nodes.map(n=>{if(/^\d+$/.test(arg)){let count=Math.min(64,Number(arg));while(n&&count--)n=n.parentElement;return n;}return n.parentElement?.closest(arg);}).filter(Boolean);break;
          case 'xpath':nodes=nodes.flatMap(n=>{const found=document.evaluate(arg,n,null,XPathResult.ORDERED_NODE_ITERATOR_TYPE,null),out=[];let node;while(out.length<1000&&(node=found.iterateNext()))if(node.nodeType===1)out.push(node);return out;});break;
          default:return [];
        }
      }catch{return [];}
      if(nodes.length>2000)nodes.length=2000;
    }
    return nodes.filter(n=>n.nodeType===1);
  }
  function applyActions() {
    for(const rule of actions){
      const nodes=select(rule.selector),action=rule.action;
      for(const node of nodes){
        if(!action){if(node.style.getPropertyValue('display')!=='none')node.style.setProperty('display','none','important');}
        else if(action.type==='remove')node.remove();
        else if(action.type==='style'){for(const declaration of action.arg.split(';')){const at=declaration.indexOf(':');if(at>0){const key=declaration.slice(0,at).trim(),value=declaration.slice(at+1).trim();if(node.style.getPropertyValue(key)!==value)node.style.setProperty(key,value.replace(/!important\s*$/,''),'important');}}}
        else if(action.type==='remove-attr')node.removeAttribute(action.arg);
        else if(action.type==='remove-class')node.classList.remove(action.arg);
      }
    }
  }
  function stop() {session++;active=false;observer.disconnect();roots.clear();actions=[];style?.remove();style=null;selectors.clear();}
  function start() {
    stop();classes.clear();ids.clear();generic=true;active=true;initial=true;
    if(document.documentElement)roots.add(document.documentElement);
    observe();schedule();
  }
  window.__lumoBlock={start,stop,get enabled(){return active;}};
  window.chrome?.webview?.addEventListener('message',event=>{
    const value=event.data;if(value?.channel!=='lumo.adblocking'||value.session!==session||value.url!==location.href)return;
    if(!value.enabled){stop();return;}
    if(!active)return;
    generic=value.generic!==false;
    if(value.initial){actions=value.actions||[];observe();}
    addSelectors(value.selectors);applyActions();
  });
  document.addEventListener('DOMContentLoaded',()=>{if(active){initial=true;roots.add(document.documentElement);observe();schedule();}},{once:true});
  document.addEventListener('visibilitychange',()=>{observe();if(active&&!document.hidden){roots.add(document.documentElement);schedule();}});
  window.addEventListener('pagehide',()=>observer.disconnect());
  window.addEventListener('popstate',()=>{if(active)start();});
  if (__LUMO_BLOCKING_ENABLED__) start();
})();

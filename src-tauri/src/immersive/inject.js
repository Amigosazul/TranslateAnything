// 沉浸模式注入脚本
//
// 运行在目标页面的隔离环境（isolated world）里：和页面自己的脚本互相看不见，只共享 DOM，
// 不会污染页面的全局变量，也不受页面内容安全策略限制。
//
// 工作方式（对应 GText 的"H5 页面"通道）：
//   遍历 DOM 文本节点和 placeholder/title 属性 → 需要翻译的原文先查页面内缓存，命中当场替换；
//   没命中的攒成一批，通过 CDP binding（__tfSend）交给 translate-filter 主程序，
//   主程序查本地译文库 / 调翻译引擎后调用 __tf.receive() 回填。
//   MutationObserver 持续监听新增节点和文字变化，界面怎么刷新都能跟上。
(() => {
  'use strict';
  // 同一隔离环境里已有旧版脚本（主程序重新接管时）：先让它恢复原文并卸载
  if (globalThis.__tf) globalThis.__tf.dispose(true);

  // 主程序注入时替换为实际配置：{ enabled, skipCjk, skipSelectors, preload }
  const CFG = /*__TF_CONFIG__*/ {};

  // 文字不翻译的区域：代码、输入框、可编辑区（不能改用户正在输入的内容）、显式声明不翻译的元素
  // select 整个跳过：没有 value 属性的 option 改了文字会连带改掉提交值
  const SKIP = [
    'script', 'style', 'noscript', 'template', 'textarea', 'input', 'select',
    'code', 'pre', 'kbd', 'samp', 'var', 'svg', 'math', 'canvas',
    '[contenteditable=""]', '[contenteditable="true"]', '[contenteditable="plaintext-only"]',
    '[translate="no"]', '.notranslate',
    ...(CFG.skipSelectors || []),
  ].join(',');
  // 属性的跳过范围宽一些：输入框的 placeholder 要翻，但可编辑区和声明不翻译的不碰
  const ATTR_SKIP = [
    '[contenteditable=""]', '[contenteditable="true"]', '[contenteditable="plaintext-only"]',
    '[translate="no"]', '.notranslate',
    ...(CFG.skipSelectors || []),
  ].join(',');
  const ATTRS = ['placeholder', 'title'];
  const BATCH = 200;
  const CJK = /[぀-ヿ㐀-䶿一-鿿豈-﫿]/g;
  const LETTER = /\p{L}/gu;

  let enabled = CFG.enabled !== false;
  const cache = new Map(Object.entries(CFG.preload || {})); // 原文 → 译文
  const textRec = new Map(); // Text 节点 → { src, a, core, b, shown }
  const attrRec = new Map(); // Element → Map(属性名 → 同上)
  const waiting = new Map(); // 原文 → Set(等待回填的目标)
  const queue = new Set(); // 待发送的原文
  const inflight = new Set(); // 已发送、等待译文的原文
  let timer = 0;
  let disposed = false;
  // 主程序重启或重新接管时会在新的隔离环境里再注入一份；隔离环境之间互相看不见，
  // 只能通过共享的 DOM 事件通知旧脚本让位（恢复原文并卸载），避免两份脚本同时改写页面
  const TAKEOVER = '__translate_filter_takeover__';

  /** 是否值得翻译（与悬浮窗模式 layout.rs 的 needs_translation 保持一致，再多过滤掉标识符） */
  function worth(s) {
    const t = s.trim();
    if (t.length < 2 || t.length > 5000) return false;
    const letters = (t.match(LETTER) || []).length;
    if (letters < 2) return false;
    if (CFG.skipCjk && (t.match(CJK) || []).length * 2 >= letters) return false;
    // 单个"词"且像路径、文件名、版本号、变量名的不翻
    if (!/\s/.test(t) && /[\/\\._:@#=<>{}\[\]$|]|\d|[a-z][A-Z]/.test(t)) return false;
    // 带版本号的短名字（"Opus 5.5"、"Node 22"）多是产品/型号名，翻了反而错
    if (t.split(/\s+/).length <= 3 && /\d/.test(t)) return false;
    return true;
  }

  /** 拆出首尾空白，只翻中间部分，写回时原样补上 */
  function split(v) {
    const m = /^(\s*)([\s\S]*?)(\s*)$/.exec(v);
    return { src: v, a: m[1], core: m[2], b: m[3], shown: null };
  }

  function skippedText(el) {
    return !el || !!el.closest(SKIP);
  }

  // ───────────── 文字节点 ─────────────

  function handleText(node, checked) {
    if (!enabled) return;
    const v = node.data;
    const old = textRec.get(node);
    if (old && v === old.shown) return; // 我们自己写进去的
    if (!worth(v) || (!checked && skippedText(node.parentElement))) {
      if (old) textRec.delete(node);
      return;
    }
    const r = split(v);
    textRec.set(node, r);
    const tr = cache.get(r.core);
    if (tr !== undefined) applyText(node, r, tr);
    else want(r.core, node, null);
  }

  function applyText(node, r, tr) {
    if (textRec.get(node) !== r || node.data !== r.src) return; // 期间被页面改掉了
    r.shown = r.a + tr + r.b;
    if (r.shown !== r.src) node.data = r.shown;
  }

  // ───────────── 属性 ─────────────

  function handleAttr(el, name) {
    if (!enabled) return;
    const v = el.getAttribute(name);
    let m = attrRec.get(el);
    const old = m && m.get(name);
    if (old && v === old.shown) return;
    if (v == null || !worth(v) || el.closest(ATTR_SKIP)) {
      if (old) m.delete(name);
      return;
    }
    const r = split(v);
    if (!m) attrRec.set(el, (m = new Map()));
    m.set(name, r);
    const tr = cache.get(r.core);
    if (tr !== undefined) applyAttr(el, name, r, tr);
    else want(r.core, el, name);
  }

  function applyAttr(el, name, r, tr) {
    const m = attrRec.get(el);
    if (!m || m.get(name) !== r || el.getAttribute(name) !== r.src) return;
    r.shown = r.a + tr + r.b;
    if (r.shown !== r.src) el.setAttribute(name, r.shown);
  }

  // ───────────── 与主程序通信 ─────────────

  function want(core, target, attr) {
    let set = waiting.get(core);
    if (!set) waiting.set(core, (set = new Set()));
    set.add(attr ? { el: target, attr } : { node: target });
    if (!inflight.has(core)) {
      queue.add(core);
      if (!timer) timer = setTimeout(flush, 30);
    }
  }

  function flush() {
    timer = 0;
    if (!queue.size || !enabled) return;
    const texts = [...queue].slice(0, BATCH);
    for (const t of texts) {
      queue.delete(t);
      inflight.add(t);
    }
    try {
      globalThis.__tfSend(JSON.stringify({ texts }));
    } catch (e) {
      // binding 尚未就绪：稍后再发
      for (const t of texts) {
        inflight.delete(t);
        queue.add(t);
      }
      timer = setTimeout(flush, 500);
      return;
    }
    if (queue.size) timer = setTimeout(flush, 30);
  }

  /** 主程序回填译文：{ map: { 原文: 译文 } } */
  function receive(msg) {
    if (disposed) return;
    for (const [src, tr] of Object.entries(msg.map || {})) {
      cache.set(src, tr);
      inflight.delete(src);
      const set = waiting.get(src);
      waiting.delete(src);
      if (!set || !enabled) continue;
      for (const t of set) {
        if (t.node) {
          const r = textRec.get(t.node);
          if (r && r.core === src) applyText(t.node, r, tr);
        } else {
          const r = attrRec.get(t.el)?.get(t.attr);
          if (r && r.core === src) applyAttr(t.el, t.attr, r, tr);
        }
      }
    }
  }

  /** 翻译失败：过一会儿重试仍在等待的原文 */
  function failed(msg) {
    const texts = msg.texts || [];
    for (const t of texts) inflight.delete(t);
    setTimeout(() => {
      for (const t of texts) if (waiting.has(t) && !inflight.has(t)) queue.add(t);
      if (queue.size && !timer) timer = setTimeout(flush, 30);
    }, 10000);
  }

  // ───────────── 遍历与监听 ─────────────

  const mo = new MutationObserver((list) => {
    for (const m of list) {
      if (m.type === 'characterData') handleText(m.target, false);
      else if (m.type === 'attributes') handleAttr(m.target, m.attributeName);
      else for (const n of m.addedNodes) if (n.isConnected) scanRoot(n);
    }
  });

  function observe(root) {
    mo.observe(root, { subtree: true, childList: true, characterData: true, attributes: true, attributeFilter: ATTRS });
  }

  /** 新增子树：先确认它不在跳过区域内 */
  function scanRoot(n) {
    if (n.nodeType === 3) return handleText(n, false);
    if (n.nodeType !== 1) return;
    if (n.parentElement && n.parentElement.closest(SKIP)) {
      // 跳过区域里的元素仍可能带 placeholder/title
      for (const a of ATTRS) if (n.hasAttribute(a)) handleAttr(n, a);
      return;
    }
    scan(n);
  }

  /** 处理元素自身的属性和开放的 Shadow DOM；返回是否继续深入它的子节点 */
  function visitElement(el) {
    for (const a of ATTRS) if (el.hasAttribute(a)) handleAttr(el, a);
    if (el.shadowRoot) {
      observe(el.shadowRoot);
      scan(el.shadowRoot);
    }
    return !el.matches(SKIP);
  }

  function scan(root) {
    if (root.nodeType === 1 && !visitElement(root)) return;
    const walker = document.createTreeWalker(root, NodeFilter.SHOW_ELEMENT | NodeFilter.SHOW_TEXT, {
      acceptNode(n) {
        if (n.nodeType === 3) return NodeFilter.FILTER_ACCEPT;
        return visitElement(n) ? NodeFilter.FILTER_SKIP : NodeFilter.FILTER_REJECT;
      },
    });
    for (let n = walker.nextNode(); n; n = walker.nextNode()) handleText(n, true);
  }

  function start() {
    const root = document.documentElement;
    if (!root) return;
    observe(root);
    scan(document.body || root);
  }

  function stop(restore = true) {
    mo.disconnect();
    if (restore) {
      for (const [node, r] of textRec) if (r.shown !== null && node.data === r.shown) node.data = r.src;
      for (const [el, m] of attrRec)
        for (const [name, r] of m) if (r.shown !== null && el.getAttribute(name) === r.shown) el.setAttribute(name, r.src);
    }
    textRec.clear();
    attrRec.clear();
    waiting.clear();
    queue.clear();
    inflight.clear();
  }

  function setEnabled(on) {
    if (disposed) return;
    on = !!on;
    if (on === enabled) return;
    enabled = on;
    if (on) start();
    else stop();
  }

  /**
   * 主程序推送的修改（改了术语表、译文库）：
   * set：原文 → 新译文，已显示的立即换成新译文；del：忘掉这些原文的旧译文，重新请求翻译
   */
  function patch(msg) {
    if (disposed) return;
    const set = msg.set || {};
    const del = new Set(msg.del || []);
    for (const [src, tr] of Object.entries(set)) cache.set(src, tr);
    for (const src of del) cache.delete(src);
    if (!enabled) return;
    const redo = (r, cur, write, target, attr) => {
      if (cur !== r.src && cur !== r.shown) return; // 已被页面改成别的内容，由观察者处理
      if (Object.hasOwn(set, r.core)) {
        r.shown = r.a + set[r.core] + r.b;
        if (cur !== r.shown) write(r.shown);
      } else if (del.has(r.core)) {
        if (cur !== r.src) write(r.src);
        r.shown = null;
        want(r.core, target, attr);
      }
    };
    for (const [node, r] of textRec) redo(r, node.data, (v) => (node.data = v), node, null);
    for (const [el, m] of attrRec)
      for (const [name, r] of m) redo(r, el.getAttribute(name), (v) => el.setAttribute(name, v), el, name);
  }

  /** 卸载：停止监听、清掉定时器；restore 为真时把页面恢复成原文 */
  function dispose(restore) {
    if (disposed) return;
    stop(restore);
    disposed = true;
    enabled = false;
    clearInterval(gc);
    clearTimeout(timer);
    document.removeEventListener(TAKEOVER, onTakeover);
    if (globalThis.__tf === api) delete globalThis.__tf;
  }

  function onTakeover() {
    dispose(true);
  }

  // 定期清理已离开页面的节点，长时间运行的单页应用不至于越积越多
  const gc = setInterval(() => {
    for (const node of textRec.keys()) if (!node.isConnected) textRec.delete(node);
    for (const el of attrRec.keys()) if (!el.isConnected) attrRec.delete(el);
  }, 30000);

  const api = { receive, failed, setEnabled, patch, dispose };
  globalThis.__tf = api;

  // 先通知其他隔离环境里的旧脚本让位，再开始监听（自己发的事件不会被自己收到）
  document.dispatchEvent(new CustomEvent(TAKEOVER));
  document.addEventListener(TAKEOVER, onTakeover);

  if (enabled) {
    if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', start, { once: true });
    else start();
  }
})();

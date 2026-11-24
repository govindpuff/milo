(() => {
  const limit = { controls: 150, texts: 90, name: 110, text: 180, value: 80, choices: 8, read: 4000, thin: 3, held: 200 };
  const clean = (s, n) => {
    s = String(s || '').replace(/\s+/g, ' ').trim();
    return s.length > n ? s.slice(0, n - 1) + '…' : s;
  };
  const quote = (s) => '"' + s.replace(/"/g, "'") + '"';
  const tags = { BUTTON: 'button', TEXTAREA: 'text area', SELECT: 'pop-up menu', SUMMARY: 'button', VIDEO: 'video', AUDIO: 'audio' };
  const roles = {
    button: 'button', link: 'link', checkbox: 'checkbox', radio: 'radio button', tab: 'tab', menuitem: 'menu item',
    menuitemcheckbox: 'menu item', menuitemradio: 'menu item', option: 'choice', switch: 'switch', textbox: 'text field',
    searchbox: 'text field', combobox: 'text field', slider: 'slider', spinbutton: 'text field',
  };
  const loose = { row: 'row', gridcell: 'cell', listitem: 'item', treeitem: 'item' };
  const skipped = new Set(['SCRIPT', 'STYLE', 'NOSCRIPT', 'TEMPLATE', 'HEAD', 'META', 'LINK', 'BR', 'HR']);
  const leaves = new Set(['text field', 'text area', 'pop-up menu', 'slider', 'video', 'audio', 'file chooser']);
  const controls = 'a[href],button,input,select,textarea,[role=button],[role=link],[role=checkbox],[role=menuitem],[role=tab]';
  const blocks = 'p,li,td,th,h1,h2,h3,h4,h5,h6,div,section,article,dd,dt,blockquote,pre,label,figcaption,[role=heading]';
  const up = (node) => node.assignedSlot || node.parentNode || node.host || null;
  const within = (a, b) => {
    for (let i = 0; a && i < 400; i++) {
      if (a === b) return true;
      a = up(a);
    }
    return false;
  };
  const active = () => {
    let a = document.activeElement;
    for (let i = 0; a && i < 20; i++) {
      if (a.shadowRoot && a.shadowRoot.activeElement) {
        a = a.shadowRoot.activeElement;
        continue;
      }
      if (a.tagName === 'IFRAME') {
        let d = null;
        try { d = a.contentDocument; } catch (e) { d = null; }
        if (d && d.activeElement && d.activeElement !== d.body) {
          a = d.activeElement;
          continue;
        }
      }
      break;
    }
    return a;
  };
  const shift = (el) => {
    let x = 0, y = 0, w = el.ownerDocument.defaultView;
    for (let i = 0; w && w !== window && i < 20; i++) {
      let f = null;
      try { f = w.frameElement; } catch (e) { f = null; }
      if (!f) break;
      const r = f.getBoundingClientRect();
      x += r.left + f.clientLeft;
      y += r.top + f.clientTop;
      w = f.ownerDocument.defaultView;
    }
    return [x, y];
  };
  const box = (el, style) => {
    const rects = el.getClientRects();
    const inline = rects.length > 1 && (style || getComputedStyle(el)).display.startsWith('inline');
    const r = inline ? rects[0] : el.getBoundingClientRect();
    const [x, y] = shift(el);
    return { left: r.left + x, top: r.top + y, right: r.right + x, bottom: r.bottom + y, width: r.width, height: r.height };
  };
  const deep = (x, y) => {
    let el = document.elementFromPoint(x, y);
    for (let i = 0; el && i < 20; i++) {
      if (el.shadowRoot) {
        const inner = el.shadowRoot.elementFromPoint(x, y);
        if (inner && inner !== el) {
          el = inner;
          continue;
        }
      }
      if (el.tagName === 'IFRAME' || el.tagName === 'FRAME') {
        let d = null;
        try { d = el.contentDocument; } catch (e) { d = null; }
        if (!d) break;
        const r = el.getBoundingClientRect();
        x -= r.left + el.clientLeft;
        y -= r.top + el.clientTop;
        const inner = d.elementFromPoint(x, y);
        if (!inner) break;
        el = inner;
        continue;
      }
      break;
    }
    return el;
  };
  const kind = (el) => {
    const tag = el.tagName;
    const given = (el.getAttribute('role') || '').split(' ')[0];
    if (tag === 'INPUT') {
      const t = (el.type || 'text').toLowerCase();
      if (t === 'hidden') return '';
      if (t === 'checkbox') return given === 'switch' ? 'switch' : 'checkbox';
      if (t === 'radio') return 'radio button';
      if (t === 'range') return 'slider';
      if (t === 'file') return 'file chooser';
      if (['submit', 'button', 'reset', 'image'].includes(t)) return 'button';
      return 'text field';
    }
    if (roles[given]) return roles[given];
    if (tag === 'A') return el.hasAttribute('href') ? 'link' : '';
    if (tags[tag]) return tags[tag];
    if (el.isContentEditable && !(el.parentElement && el.parentElement.isContentEditable)) return 'text area';
    return '';
  };
  const named = (el) => {
    const said = el.getAttribute('aria-label');
    if (said && said.trim()) return said;
    const by = el.getAttribute('aria-labelledby');
    if (by) {
      const root = el.getRootNode();
      const text = by.split(/\s+/).map((id) => (root.getElementById && root.getElementById(id) || {}).innerText || '').join(' ');
      if (text.trim()) return text;
    }
    const tag = el.tagName;
    if (tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT') {
      if (tag === 'INPUT' && ['submit', 'button', 'reset'].includes(el.type) && el.value) return el.value;
      const label = el.labels && el.labels[0] && el.labels[0].innerText;
      return label || el.placeholder || el.title || el.alt || el.name || '';
    }
    if (tag === 'IMG') return el.alt || el.title || '';
    if (tag === 'VIDEO' || tag === 'AUDIO') return el.title || '';
    if (el.isContentEditable) return el.getAttribute('aria-placeholder') || el.getAttribute('data-placeholder') || el.title || '';
    const text = el.innerText;
    if (text && text.trim()) return text;
    const inner = el.querySelector('img[alt],[aria-label],[title]');
    if (inner) return inner.getAttribute('alt') || inner.getAttribute('aria-label') || inner.getAttribute('title') || '';
    return el.title || el.getAttribute('data-tooltip') || (el.getAttribute('href') || '').replace(/^https?:\/\//, '').slice(0, 40);
  };
  const clock = (s) => {
    if (!isFinite(s)) return '';
    s = Math.floor(s);
    const m = Math.floor(s / 60) % 60, h = Math.floor(s / 3600);
    const two = (n) => String(n).padStart(2, '0');
    return h ? h + ':' + two(m) + ':' + two(s % 60) : m + ':' + two(s % 60);
  };
  const state = (el, k, focus) => {
    const out = [];
    const on = (name) => el.getAttribute(name) === 'true';
    if (k === 'text field' || k === 'text area') {
      let v = typeof el.value === 'string' ? el.value : el.innerText;
      if (el.type === 'password' && v) v = '•'.repeat(Math.min(v.length, 12));
      if (v && v.trim()) out.push('containing ' + quote(clean(v, limit.value)));
    }
    if (el.tagName === 'SELECT') {
      const chosen = el.selectedOptions[0];
      if (chosen) out.push('showing ' + quote(clean(chosen.text, limit.value)));
      const all = [...el.options].map((o) => clean(o.text, 40)).filter(Boolean);
      out.push('choices: ' + all.slice(0, limit.choices).join(', ') + (all.length > limit.choices ? ', and ' + (all.length - limit.choices) + ' more' : ''));
    }
    if (k === 'slider' && el.value !== undefined) out.push('at ' + el.value);
    if (k === 'checkbox' || k === 'switch' || k === 'radio button') {
      out.push(el.checked === true || on('aria-checked') ? '(on)' : '(off)');
    } else if (on('aria-checked') || on('aria-pressed')) out.push('(on)');
    if (k === 'video' || k === 'audio') {
      out.push(el.paused ? '(paused)' : '(playing)');
      const at = clock(el.currentTime), all = clock(el.duration);
      if (at && all) out.push('at ' + at + ' of ' + all);
      if (el.muted) out.push('(muted)');
    }
    if (on('aria-selected') || (el.getAttribute('aria-current') || 'false') !== 'false') out.push('(selected)');
    if (on('aria-expanded')) out.push('(open)');
    if (el.disabled || on('aria-disabled')) out.push('(disabled)');
    if (focus === el) out.push('(focused)');
    return out.join(' ');
  };
  const look = () => {
    const W = innerWidth, H = innerHeight, kept = [], lines = [];
    let texts = 0, cut = false, run = null;
    const focus = active();
    const seen = (b) => b.width >= limit.thin && b.height >= limit.thin && b.bottom > 0 && b.top < H && b.right > 0 && b.left < W;
    const pin = (v, most) => Math.min(Math.max(v, 1), most - 1);
    const reach = (el, b, k) => {
      const spots = [[(b.left + b.right) / 2, (b.top + b.bottom) / 2], [b.left + Math.min(8, b.width / 2), b.top + Math.min(8, b.height / 2)]];
      for (const [x, y] of spots) {
        const hit = deep(pin(x, W), pin(y, H));
        if (!hit) continue;
        if (within(hit, el)) return true;
        const label = hit.closest && hit.closest('label');
        if (label && label.control === el) return true;
        if (leaves.has(k) || k === 'checkbox' || k === 'radio button' || k === 'switch') {
          const near = el.parentElement && el.parentElement.parentElement || el.parentElement;
          if (near && within(hit, near) && !kind(hit)) return true;
        }
      }
      return false;
    };
    const word = (node, hidden) => {
      if (texts >= limit.texts || hidden) return;
      const raw = node.nodeValue;
      if (!raw || !/\S\S/.test(raw)) return;
      const parent = node.parentElement;
      if (!parent) return;
      const range = node.ownerDocument.createRange();
      range.selectNodeContents(node);
      const r = range.getBoundingClientRect();
      const [x, y] = shift(parent);
      const b = { left: r.left + x, top: r.top + y, right: r.right + x, bottom: r.bottom + y, width: r.width, height: r.height };
      if (!seen(b)) return;
      const block = parent.closest(blocks) || parent;
      const hit = deep(pin((b.left + b.right) / 2, W), pin((b.top + b.bottom) / 2, H));
      if (!hit || !within(hit, block)) return;
      if (run && run.block === block && run.at === lines.length - 1) {
        run.raw += ' ' + raw;
        lines[run.at] = run.lead + clean(run.raw, limit.text);
        return;
      }
      const last = lines[lines.length - 1] || '';
      if (last.startsWith('[') && last.includes(quote(clean(raw, limit.name)))) return;
      const lead = parent.closest('h1,h2,h3,h4,h5,h6,[role=heading]') ? 'heading: ' : 'text: ';
      lines.push(lead + clean(raw, limit.text));
      run = { block, at: lines.length - 1, raw, lead };
      texts += 1;
    };
    const walk = (node, inside, pointer, above, hidden) => {
      if (node.nodeType === 3) {
        if (!inside) word(node, hidden);
        return;
      }
      if (node.nodeType !== 1) return;
      const el = node, tag = el.tagName;
      if (skipped.has(tag)) return;
      if (el.checkVisibility && !el.checkVisibility()) return;
      const style = getComputedStyle(el);
      const unseen = style.visibility === 'hidden' || style.visibility === 'collapse';
      const point = style.cursor === 'pointer';
      let k = kind(el), free = false;
      if (!k) {
        const given = (el.getAttribute('role') || '').split(' ')[0];
        const acts = el.hasAttribute('onclick') || el.hasAttribute('tabindex') && el.tabIndex >= 0 || point && !pointer;
        if (acts && loose[given]) k = loose[given];
        else if (acts && tag !== 'BODY' && tag !== 'HTML' && tag !== 'LABEL' && !el.querySelector(controls)) {
          k = 'button';
          free = true;
        }
      }
      let listed = false, name = above;
      if (k && !unseen) {
        const b = box(el, style);
        if (seen(b) && reach(el, b, k)) {
          const own = clean(named(el), limit.name);
          const twin = own && own === above && (k === 'link' || k === 'button');
          if (kept.length >= limit.controls) cut = true;
          else if (!twin && (own || !free)) {
            kept.push(el);
            const more = state(el, k, focus);
            lines.push('[' + kept.length + '] ' + k + (own ? ' ' + quote(own) : '') + (more ? ' ' + more : ''));
            listed = true;
            name = own;
          }
        }
      }
      if (leaves.has(k)) return;
      const next = inside || listed;
      let kids = el.childNodes;
      if (el.shadowRoot) kids = el.shadowRoot.childNodes;
      else if (tag === 'SLOT') {
        const given = el.assignedNodes({ flatten: true });
        if (given.length) kids = given;
      } else if (tag === 'IFRAME' || tag === 'FRAME') {
        let d = null;
        try { d = el.contentDocument; } catch (e) { d = null; }
        kids = d && d.body ? [d.body] : [];
      } else if (tag === 'SVG' || tag === 'svg') kids = [];
      for (const kid of kids) walk(kid, next, point, name, unseen);
    };
    if (document.body) walk(document.body, false, false, '', false);
    window.__milo_kept = kept;
    const page = document.scrollingElement || document.documentElement;
    const tall = page.scrollHeight - H;
    if (tall > 40) {
      const top = page.scrollTop < 20, bottom = page.scrollTop >= tall - 20;
      if (top) lines.push('(this is the top of the page; there is more below)');
      else if (bottom) lines.push('(this is the bottom of the page; there is more above)');
      else lines.push('(there is more of the page above and below)');
    }
    if (cut) lines.push('(there are more controls here than are listed; scroll to see them)');
    if (document.readyState !== 'complete') lines.push('(the page is still loading)');
    return { title: clean(document.title, limit.name), url: location.href, lines, width: W, height: H, ratio: devicePixelRatio || 1 };
  };
  const pick = (n) => {
    const el = (window.__milo_kept || [])[n - 1];
    return el && el.isConnected ? el : null;
  };
  const spot = (n) => {
    const el = pick(n);
    if (!el) return null;
    let b = box(el);
    if (b.top < 0 || b.bottom > innerHeight || b.left < 0 || b.right > innerWidth) {
      el.scrollIntoView({ block: 'center', inline: 'center', behavior: 'instant' });
      b = box(el);
    }
    const pin = (v, most) => Math.min(Math.max(v, 1), most - 1);
    return { x: pin((b.left + b.right) / 2, innerWidth), y: pin((b.top + b.bottom) / 2, innerHeight), list: el.tagName === 'SELECT' };
  };
  const ready = (n) => {
    const el = pick(n);
    if (!el) return 'gone';
    const now = active();
    if (now !== el && !within(now, el)) el.focus();
    const at = active();
    if (at && typeof at.select === 'function' && at.tagName !== 'SELECT') {
      at.select();
      return 'field';
    }
    if (at && at.isContentEditable) {
      const range = at.ownerDocument.createRange();
      range.selectNodeContents(at);
      range.collapse(false);
      const chosen = at.ownerDocument.defaultView.getSelection();
      chosen.removeAllRanges();
      chosen.addRange(range);
      return 'area';
    }
    return 'other';
  };
  const holds = () => {
    const el = active();
    if (!el || el === document.body) return null;
    const v = typeof el.value === 'string' ? el.value : el.isContentEditable ? el.innerText : null;
    return v === null ? null : clean(v, limit.held);
  };
  const force = (text) => {
    const el = active();
    if (!el || typeof el.value !== 'string') return false;
    const proto = el.tagName === 'TEXTAREA' ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
    const set = Object.getOwnPropertyDescriptor(proto, 'value');
    if (!set || !set.set) return false;
    set.set.call(el, text);
    el.dispatchEvent(new Event('input', { bubbles: true }));
    el.dispatchEvent(new Event('change', { bubbles: true }));
    return true;
  };
  const choose = (n, text) => {
    const el = pick(n);
    if (!el || el.tagName !== 'SELECT') return null;
    const want = text.trim().toLowerCase();
    const all = [...el.options];
    const hit = all.find((o) => o.text.trim().toLowerCase() === want) || all.find((o) => o.text.toLowerCase().includes(want));
    if (!hit) return '';
    el.value = hit.value;
    el.dispatchEvent(new Event('input', { bubbles: true }));
    el.dispatchEvent(new Event('change', { bubbles: true }));
    return hit.text.trim();
  };
  const under = (x, y) => {
    let el = deep(x, y);
    for (let i = 0; el && i < 12; i++) {
      const k = kind(el);
      if (k) return k + ' ' + quote(clean(named(el), limit.name));
      el = up(el);
      if (el && el.nodeType !== 1) el = up(el);
    }
    return '';
  };
  const read = () => {
    const main = document.querySelector('main,[role=main],article') || document.body;
    const text = (main ? main.innerText : '').replace(/[ \t]+/g, ' ').replace(/\n\s*\n+/g, '\n').trim();
    return text.length > limit.read ? text.slice(0, limit.read) + '…' : text;
  };
  const glance = () => {
    const page = document.scrollingElement || document.documentElement;
    const body = document.body;
    return [location.href, document.readyState, document.getElementsByTagName('*').length, Math.round(page.scrollTop), body ? body.innerText.length : 0].join('|');
  };
  const wheel = (dy) => {
    const page = document.scrollingElement || document.documentElement;
    const was = page.scrollTop;
    page.scrollBy(0, dy);
    return page.scrollTop !== was;
  };
  const size = () => [innerWidth, innerHeight, devicePixelRatio || 1, scrollX, scrollY];
  return { look, spot, ready, holds, force, choose, under, read, glance, wheel, size };
})()

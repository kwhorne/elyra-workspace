// Elyra Workspace browser: "Pick element". The user points at something on the
// page and clicks it; Elyra reads back a description of that element for a
// message to the agent. Runs in WebKit's isolated client world, so the page's
// own scripts can't see or change it. While picking, clicks are caught before
// the page gets them; Escape stops.
(() => {
  if (window.__elyraPick) return;

  const STYLES = ['display', 'position', 'top', 'left', 'width', 'height', 'margin', 'padding', 'border',
    'border-radius', 'box-sizing', 'overflow', 'z-index', 'opacity', 'color', 'background-color', 'font-family',
    'font-size', 'font-weight', 'line-height', 'text-align', 'flex-direction', 'align-items', 'justify-content',
    'gap', 'grid-template-columns', 'transform'];

  let overlay = null;
  let label = null;
  let current = null;
  let result = null;

  const escape = (value) => (window.CSS && CSS.escape ? CSS.escape(value) : value);

  // A selector that matches only `el`: an id if it is unique, otherwise a
  // path of tag, a couple of classes and :nth-of-type where needed.
  const selectorOf = (el) => {
    if (el.id && document.querySelectorAll('#' + escape(el.id)).length === 1) return '#' + escape(el.id);
    const parts = [];
    let node = el;
    while (node && node.nodeType === 1 && node !== document.documentElement && parts.length < 8) {
      let part = node.tagName.toLowerCase();
      if (node.id && document.querySelectorAll('#' + escape(node.id)).length === 1) {
        parts.unshift('#' + escape(node.id));
        break;
      }
      const classes = [...node.classList].filter((c) => !/^\d/.test(c)).slice(0, 2);
      if (classes.length) part += '.' + classes.map(escape).join('.');
      const parent = node.parentElement;
      if (parent) {
        const same = [...parent.children].filter((c) => c.tagName === node.tagName);
        if (same.length > 1) part += `:nth-of-type(${same.indexOf(node) + 1})`;
      }
      parts.unshift(part);
      const selector = parts.join(' > ');
      try {
        if (document.querySelectorAll(selector).length === 1) return selector;
      } catch (_) {}
      node = parent;
    }
    return parts.join(' > ');
  };

  const describe = (el) => {
    const r = el.getBoundingClientRect();
    const style = getComputedStyle(el);
    const styles = {};
    for (const name of STYLES) styles[name] = style.getPropertyValue(name);
    let html = el.outerHTML;
    if (el.type === 'password') html = html.replace(/value="[^"]*"/, 'value="…"');
    if (html.length > 1500) html = html.slice(0, 1500) + '…';
    return {
      url: location.href,
      selector: selectorOf(el),
      tag: el.tagName.toLowerCase(),
      text: (el.innerText || el.value || '').replace(/\s+/g, ' ').trim().slice(0, 300),
      html,
      box: { x: Math.round(r.x), y: Math.round(r.y), width: Math.round(r.width), height: Math.round(r.height) },
      viewport: { width: innerWidth, height: innerHeight },
      styles,
    };
  };

  const show = (el) => {
    current = el;
    const r = el.getBoundingClientRect();
    Object.assign(overlay.style, {
      left: r.left + 'px', top: r.top + 'px', width: r.width + 'px', height: r.height + 'px', display: 'block',
    });
    label.textContent = `${selectorOf(el)}  ${Math.round(r.width)}×${Math.round(r.height)}`;
    label.style.top = (r.top > 24 ? r.top - 22 : r.bottom + 4) + 'px';
    label.style.left = Math.max(0, r.left) + 'px';
    label.style.display = 'block';
  };

  const onMove = (event) => {
    const el = document.elementFromPoint(event.clientX, event.clientY);
    if (el && el !== current && el !== overlay && el !== label) show(el);
  };
  const swallow = (event) => {
    event.preventDefault();
    event.stopPropagation();
    event.stopImmediatePropagation();
  };
  const onClick = (event) => {
    swallow(event);
    const el = document.elementFromPoint(event.clientX, event.clientY) || current;
    if (el) result = describe(el);
    stop();
  };
  const onKey = (event) => {
    if (event.key === 'Escape') {
      swallow(event);
      result = { cancelled: true };
      stop();
    }
  };

  const start = () => {
    stop();
    result = null;
    overlay = document.createElement('div');
    Object.assign(overlay.style, {
      position: 'fixed', zIndex: '2147483647', pointerEvents: 'none', display: 'none',
      outline: '2px solid #f97316', background: 'rgba(249, 115, 22, 0.12)', borderRadius: '2px',
    });
    label = document.createElement('div');
    Object.assign(label.style, {
      position: 'fixed', zIndex: '2147483647', pointerEvents: 'none', display: 'none',
      font: '11px/1.6 -apple-system, sans-serif', color: '#fff', background: '#f97316',
      padding: '0 6px', borderRadius: '3px', whiteSpace: 'nowrap',
    });
    document.documentElement.append(overlay, label);
    document.addEventListener('mousemove', onMove, true);
    document.addEventListener('mousedown', swallow, true);
    document.addEventListener('mouseup', swallow, true);
    document.addEventListener('click', onClick, true);
    document.addEventListener('keydown', onKey, true);
    return 'started';
  };

  function stop() {
    document.removeEventListener('mousemove', onMove, true);
    document.removeEventListener('mousedown', swallow, true);
    document.removeEventListener('mouseup', swallow, true);
    document.removeEventListener('click', onClick, true);
    document.removeEventListener('keydown', onKey, true);
    overlay?.remove();
    label?.remove();
    overlay = label = current = null;
    return 'stopped';
  }

  // The picked element as JSON (once), `{"cancelled":true}`, or "" while picking.
  const take = () => {
    if (!result) return '';
    const json = JSON.stringify(result);
    result = null;
    return json;
  };

  window.__elyraPick = { start, stop, take };
})();

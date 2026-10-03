// Elyra Workspace browser, for coding agents: looking at a local development
// page (see src/browser_tools.rs). Adapted from Litr (© Wirelabs AS, used under
// the MIT licence with its owner's permission). Runs in WebKit's isolated client
// world: it reads the DOM the page built and changes nothing. Password fields'
// values are never read.
(() => {
  if (window.__elyraLook) return;
  const LIMIT = 40000;

  const describe = (el) => {
    let d = el.tagName.toLowerCase();
    if (el.id) d += '#' + el.id;
    if (el.classList && el.classList.length) d += '.' + [...el.classList].slice(0, 4).join('.');
    const attrs = [];
    for (const name of ['href', 'src', 'type', 'name', 'role', 'aria-label', 'placeholder', 'alt', 'title', 'for', 'disabled', 'hidden']) {
      if (el.hasAttribute(name)) {
        const value = el.getAttribute(name) || '';
        attrs.push(value ? `${name}=${JSON.stringify(value.slice(0, 120))}` : name);
      }
    }
    if ((el.tagName === 'INPUT' || el.tagName === 'TEXTAREA' || el.tagName === 'SELECT') && el.type !== 'password') {
      attrs.push(`value=${JSON.stringify(String(el.value).slice(0, 120))}`);
    }
    return attrs.length ? `${d} [${attrs.join(' ')}]` : d;
  };

  const skip = new Set(['SCRIPT', 'STYLE', 'NOSCRIPT', 'TEMPLATE', 'META', 'LINK']);

  const snapshot = () => {
    let out = '';
    let cut = false;
    const walk = (node, depth) => {
      for (const child of node.childNodes) {
        if (out.length > LIMIT) { cut = true; return; }
        const pad = '  '.repeat(depth);
        if (child.nodeType === Node.TEXT_NODE) {
          const text = child.textContent.replace(/\s+/g, ' ').trim();
          if (text) out += pad + JSON.stringify(text.slice(0, 200)) + '\n';
        } else if (child.nodeType === Node.ELEMENT_NODE && !skip.has(child.tagName)) {
          if (child.tagName === 'svg') { out += pad + 'svg\n'; continue; }
          out += pad + describe(child) + '\n';
          walk(child.shadowRoot || child, depth + 1);
        }
      }
    };
    if (document.body) walk(document.body, 0);
    return JSON.stringify({ url: location.href, title: document.title, outline: out, truncated: cut });
  };

  const STYLES = ['display', 'visibility', 'opacity', 'position', 'z-index', 'width', 'height', 'margin', 'padding',
    'overflow', 'color', 'background-color', 'font-size', 'font-family', 'flex-direction', 'grid-template-columns', 'transform'];

  const query = (selector, limit) => {
    let found;
    try { found = [...document.querySelectorAll(selector)]; } catch (e) {
      return JSON.stringify({ error: `Not a valid selector: ${e.message}` });
    }
    const elements = found.slice(0, limit || 5).map((el) => {
      const r = el.getBoundingClientRect();
      const style = getComputedStyle(el);
      const styles = {};
      for (const name of STYLES) styles[name] = style.getPropertyValue(name);
      let html = el.outerHTML;
      if (el.type === 'password') html = html.replace(/value="[^"]*"/, 'value="…"');
      return {
        element: describe(el),
        html: html.slice(0, 2000),
        text: (el.innerText || '').replace(/\s+/g, ' ').trim().slice(0, 500),
        box: { x: Math.round(r.x), y: Math.round(r.y), width: Math.round(r.width), height: Math.round(r.height) },
        styles,
      };
    });
    return JSON.stringify({ selector, matches: found.length, elements });
  };

  window.__elyraLook = { snapshot, query };
})();

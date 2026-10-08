// Elyra Workspace browser, for coding agents: acting on a local development
// page (see src/browser_tools.rs) — click, fill in, press a key. Runs in
// WebKit's isolated client world, so the page's scripts can't see or change
// it; the events it sends reach the page's own handlers. Every call answers
// JSON: what it did, or an error.
(() => {
  if (window.__elyraAct) return;

  const describe = (el) => {
    let d = el.tagName.toLowerCase();
    if (el.id) d += '#' + el.id;
    if (el.classList && el.classList.length) d += '.' + [...el.classList].slice(0, 3).join('.');
    const text = (el.innerText || el.value || el.getAttribute('aria-label') || '').replace(/\s+/g, ' ').trim();
    return text && el.type !== 'password' ? `${d} "${text.slice(0, 60)}"` : d;
  };

  const visible = (el) => {
    const r = el.getBoundingClientRect();
    const style = getComputedStyle(el);
    return r.width > 0 && r.height > 0 && style.visibility !== 'hidden' && style.display !== 'none';
  };

  // The element to act on: a CSS selector, a visible text, or both (the
  // first visible match whose text contains it).
  const find = (selector, text) => {
    let candidates;
    try {
      candidates = [...document.querySelectorAll(selector || 'a, button, input, select, textarea, label, summary, [role], [onclick], [tabindex]')];
    } catch (e) {
      return { error: `Not a valid selector: ${e.message}` };
    }
    if (text) {
      const wanted = text.toLowerCase();
      const matches = candidates.filter((el) => {
        // A field is also known by its label.
        const labels = [...(el.labels || [])].map((label) => label.innerText).join(' ');
        const own = [el.innerText, el.value, el.getAttribute('aria-label'), el.getAttribute('placeholder'), labels]
          .filter(Boolean).join(' ').toLowerCase();
        return own.includes(wanted);
      });
      // The innermost match, not every ancestor around it.
      candidates = matches.filter((el) => !matches.some((other) => other !== el && el.contains(other)));
    }
    const shown = candidates.filter(visible);
    const el = shown[0] || candidates[0];
    if (!el) return { error: `Nothing matches${selector ? ' ' + selector : ''}${text ? ` with the text "${text}"` : ''}.` };
    return { el, matches: candidates.length };
  };

  const done = (action, el, extra) => JSON.stringify({ done: action, element: describe(el), url: location.href, ...extra });
  const fail = (error) => JSON.stringify({ error });

  const click = (selector, text) => {
    const found = find(selector, text);
    if (found.error) return fail(found.error);
    const el = found.el;
    if (el.disabled) return fail(`${describe(el)} is disabled.`);
    el.scrollIntoView({ block: 'center', inline: 'center' });
    const r = el.getBoundingClientRect();
    const at = { bubbles: true, cancelable: true, composed: true, clientX: r.x + r.width / 2, clientY: r.y + r.height / 2, button: 0 };
    el.dispatchEvent(new PointerEvent('pointerdown', at));
    el.dispatchEvent(new MouseEvent('mousedown', at));
    if (el.focus) el.focus();
    el.dispatchEvent(new PointerEvent('pointerup', at));
    el.dispatchEvent(new MouseEvent('mouseup', at));
    el.click();
    return done('clicked', el, { matches: found.matches });
  };

  // Set a value the way typing does, so frameworks that watch the input
  // (React, Vue, Livewire, Alpine) see the change.
  const setValue = (el, value) => {
    const proto = el.tagName === 'TEXTAREA' ? HTMLTextAreaElement.prototype
      : el.tagName === 'SELECT' ? HTMLSelectElement.prototype : HTMLInputElement.prototype;
    const setter = Object.getOwnPropertyDescriptor(proto, 'value').set;
    setter.call(el, value);
    el.dispatchEvent(new Event('input', { bubbles: true }));
    el.dispatchEvent(new Event('change', { bubbles: true }));
  };

  const fill = (selector, text, value) => {
    const found = find(selector || 'input, textarea, select, [contenteditable="true"]', text);
    if (found.error) return fail(found.error);
    let el = found.el;
    // A label stands for its field.
    if (el.tagName === 'LABEL') el = el.control || el.querySelector('input, textarea, select') || el;
    if (el.disabled || el.readOnly) return fail(`${describe(el)} can't be changed.`);
    el.scrollIntoView({ block: 'center' });
    if (el.focus) el.focus();
    if (el.type === 'checkbox' || el.type === 'radio') {
      const on = !['false', '0', 'off', 'no', ''].includes(String(value).toLowerCase());
      if (el.checked !== on) el.click();
      return done(on ? 'checked' : 'unchecked', el);
    }
    if (el.tagName === 'SELECT') {
      const option = [...el.options].find((o) => o.value === value || o.text.trim() === value);
      if (!option) return fail(`${describe(el)} has no option "${value}"; it has: ${[...el.options].map((o) => o.text.trim()).join(', ')}`);
      setValue(el, option.value);
      return done('selected', el, { value: option.text.trim() });
    }
    if (el.isContentEditable) {
      el.textContent = value;
      el.dispatchEvent(new InputEvent('input', { bubbles: true }));
      return done('filled', el);
    }
    if (!('value' in el)) return fail(`${describe(el)} isn't a field.`);
    setValue(el, value);
    return done('filled', el);
  };

  const KEYS = { Enter: 13, Tab: 9, Escape: 27, Backspace: 8, ArrowUp: 38, ArrowDown: 40, ArrowLeft: 37, ArrowRight: 39, ' ': 32 };

  const press = (key, selector, text) => {
    let el = document.activeElement || document.body;
    if (selector || text) {
      const found = find(selector, text);
      if (found.error) return fail(found.error);
      el = found.el;
      if (el.focus) el.focus();
    }
    const init = { key, code: key.length === 1 ? `Key${key.toUpperCase()}` : key, keyCode: KEYS[key] || key.toUpperCase().charCodeAt(0), which: KEYS[key] || 0, bubbles: true, cancelable: true, composed: true };
    const allowed = el.dispatchEvent(new KeyboardEvent('keydown', init));
    el.dispatchEvent(new KeyboardEvent('keyup', init));
    // Synthetic keys don't trigger the browser's own action: do the common ones.
    if (allowed && key === 'Enter' && el.form && el.tagName === 'INPUT') {
      el.form.requestSubmit();
      return done('pressed Enter and submitted the form', el);
    }
    if (allowed && key === 'Enter' && (el.tagName === 'BUTTON' || el.tagName === 'A')) el.click();
    return done(`pressed ${key}`, el);
  };

  // Whether something matching is on the page (for waiting).
  const present = (selector, text) => {
    const found = find(selector || (text ? 'body *' : null), text);
    return JSON.stringify(found.error ? { present: false } : { present: true, element: describe(found.el) });
  };

  window.__elyraAct = { click, fill, press, present };
})();

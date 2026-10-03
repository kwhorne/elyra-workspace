// Elyra Workspace browser, for coding agents: what a local development page
// writes to its console and asks the network for, kept for the agent tools
// (see src/browser_tools.rs). Adapted from Litr (© Wirelabs AS, used under the
// MIT licence with its owner's permission). Runs in the page's own world, since
// only there can console, fetch and XMLHttpRequest be watched; only on pages
// served from this Mac. Nothing leaves the page but what Elyra Workspace reads
// back when an agent asks.
(() => {
  const host = location.hostname;
  const local = host === 'localhost' || host === '127.0.0.1' || host === '[::1]' ||
    /\.(test|local|localhost)$/.test(host);
  if (!local || window.__elyraAgent) return;
  const store = { console: [], network: [] };
  Object.defineProperty(window, '__elyraAgent', { value: store });

  const MAX = 500;
  const keep = (list, item) => {
    list.push(item);
    if (list.length > MAX) list.shift();
  };
  const show = (value) => {
    try {
      // WebKit's stack has no message in it, so put name and message first.
      if (value instanceof Error) return `${value.name}: ${value.message}${value.stack ? `\n${value.stack}` : ''}`;
      if (value !== null && typeof value === 'object') return JSON.stringify(value).slice(0, 2000);
      return String(value);
    } catch (_) {
      return String(value);
    }
  };

  for (const level of ['log', 'info', 'warn', 'error', 'debug']) {
    const original = console[level];
    if (typeof original !== 'function') continue;
    console[level] = function (...args) {
      keep(store.console, { level, time: Date.now(), text: args.map(show).join(' ').slice(0, 4000) });
      return original.apply(this, args);
    };
  }

  // Errors nothing caught never reach console.error; record them as if they had.
  window.addEventListener('error', (event) => {
    const where = event.filename ? ` (${event.filename}:${event.lineno}:${event.colno})` : '';
    const text = event.error ? show(event.error) : String(event.message || 'Script error');
    keep(store.console, { level: 'error', time: Date.now(), text: `Uncaught ${text}${where}`.slice(0, 4000) });
  });
  window.addEventListener('unhandledrejection', (event) => {
    keep(store.console, { level: 'error', time: Date.now(), text: `Unhandled rejection: ${show(event.reason)}`.slice(0, 4000) });
  });

  const textual = (type) => /json|text|xml|javascript/.test(type || '');
  const absolute = (url) => {
    try { return new URL(url, location.href).href; } catch (_) { return String(url); }
  };

  const fetch = window.fetch;
  if (typeof fetch === 'function') {
    window.fetch = function (input, init) {
      const started = performance.now();
      const record = {
        kind: 'fetch',
        method: String((init && init.method) || (input && input.method) || 'GET').toUpperCase(),
        url: absolute(input && input.url ? input.url : input),
        time: Date.now(),
      };
      keep(store.network, record);
      return fetch.apply(this, arguments).then(
        (response) => {
          record.status = response.status;
          record.ms = Math.round(performance.now() - started);
          record.type = response.headers.get('content-type') || '';
          if (textual(record.type)) {
            response.clone().text().then((body) => { record.body = body.slice(0, 4000); }, () => {});
          }
          return response;
        },
        (error) => {
          record.error = String(error);
          record.ms = Math.round(performance.now() - started);
          throw error;
        },
      );
    };
  }

  const open = XMLHttpRequest.prototype.open;
  const send = XMLHttpRequest.prototype.send;
  XMLHttpRequest.prototype.open = function (method, url) {
    this.__elyraRecord = { kind: 'xhr', method: String(method).toUpperCase(), url: absolute(url), time: Date.now() };
    return open.apply(this, arguments);
  };
  XMLHttpRequest.prototype.send = function () {
    const record = this.__elyraRecord;
    if (record) {
      keep(store.network, record);
      const started = performance.now();
      this.addEventListener('loadend', () => {
        record.status = this.status;
        record.ms = Math.round(performance.now() - started);
        record.type = this.getResponseHeader('content-type') || '';
        if (this.status === 0) record.error = 'failed';
        if (textual(record.type) && (this.responseType === '' || this.responseType === 'text')) {
          record.body = String(this.responseText).slice(0, 4000);
        }
      });
    }
    return send.apply(this, arguments);
  };
})();

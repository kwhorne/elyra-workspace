# Browser

Each thread has its own web browser in the **Browser** tab of the tools panel
(**⇧⌘B**). Use it to look at the app you and the agent are building, next to
the conversation, without switching windows.

## Opening a page

- Type an address in the address field and press Enter. `localhost:3000`,
  `myapp.test` and other local addresses open over `http`; other addresses with a
  dot open over `https`; anything else searches DuckDuckGo.
- Before a page is open, the tab lists the development servers running from the
  thread's folder. Click one to open it.
- Clicking a server address under **Local servers** in the Context tab also opens
  it here.

The toolbar has back, forward, reload (stop while loading) and **Open in your
browser**, which hands the page to your default browser. Links that open a new
window open in the same tab. Downloads, `mailto:` links and other things the
browser can't show go to the app that handles them on your Mac.

Copy, paste, cut, select all and undo (⌘C, ⌘V, ⌘X, ⌘A, ⌘Z, ⇧⌘Z) work in the page
while it has focus. Click the conversation or press a shortcut to go back to the
rest of the app.

The browser uses Safari's engine (WebKit). Pages share cookies and storage across
threads, as in one Safari window with several tabs. Closing a thread's tab closes
its browser.

## Letting the agent look at the page

With the [agent gateway](agent-gateway.md) on, agents get tools to open a page in
their thread's browser and look at it: its structure, elements and their styles,
the console, network calls and a screenshot. You can ask things like:

> Open localhost:5173, check why the cart total is wrong, fix it and reload to
> confirm.

For safety, agents can only open and read pages served from this Mac:
`localhost`, `127.0.0.1`, `[::1]`, and names ending in `.localhost`, `.test` or
`.local`. If the thread's browser shows any other site, the tools refuse to read
it. Apart from opening and reloading a page, the tools only read; an agent can't
click, type or run its own scripts in the page.

The console and network calls are recorded from when a local page loads. A
screenshot needs the Browser tab to be on screen.

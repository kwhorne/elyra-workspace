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

## Pointing at an element

To show the agent exactly what you mean, press the **target** button in the
toolbar and click the thing on the page: a button that sits too low, a heading in
the wrong colour. While picking, the element under the pointer is outlined and
the page doesn't react to clicks; **Esc** or the button again stops.

The element is attached to your message as a picture of it with a little of its
surroundings, and its details: a selector that matches only it, its position and
size, its text, its computed styles (layout, spacing, colours, fonts) and its
HTML. Write what's wrong, *this button sits too low*, and send.

It works on any page, since it only adds to the message you send. Password field
values are left out.

## Errors on the page

While a local page is open, Elyra watches it for errors: anything written with
`console.error`, exceptions nothing caught, and requests that fail or return an
error status. When new ones appear, a chip above the message box says so:
**3 new errors in the browser**.

- **Add to message** attaches them to your next message, with the page address,
  so you don't have to copy them from the console.
- **×** dismisses them. Either way they aren't offered again.

When [Elyra Grove](context-and-goals.md#grove) runs the project as an app, the
chip also counts **server errors**: requests Grove recorded with a 5xx status
since the thread was opened, from any browser. **Add to message** attaches
Grove's explanation of each (up to three): the request and its body, the SQL it
ran and the mail it sent, and the error log with its stack trace. The page in the
Browser tab doesn't have to be open for these.

## Pictures before and after a turn

When the Browser tab shows a local page while you send a message, Elyra keeps a
picture of the page from before the turn. When the turn ends it reloads the
page, waits a moment for the dev server, and takes another. Both appear under the
turn in the conversation, so you see what the change did to the page. Click a
picture to open it full size.

This only happens while the Browser tab is on screen, since the page can only be
pictured then. The pictures are kept in `~/.elyra/snapshots` and deleted with the
thread.

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

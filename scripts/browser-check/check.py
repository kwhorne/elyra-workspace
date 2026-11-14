#!/usr/bin/env python3
"""Browser check: the thread's browser in the real app, with a window (the
headless scenarios have no web pages), against a small local site.

Journeys, with the thread's Browser tab on screen and out of sight (as after
a turn, when a page that was never shown has no size at all):
- the first replay saves a clean layout, at the desktop width;
- a page broken only at a phone's width fails at 390 px, and only there;
- a page broken at a desktop's width fails there, and not as a phone problem;
- the page gets its own width back afterwards.

Pictures of the page before and after a turn (on screen; WebKit pictures
only what is): both are taken, the same size, also when the project replays
its journeys after the turn.

The page's errors (both ways): agents read its console and failed requests.

    cargo build -p elyra-app && scripts/browser-check/check.py

The app runs from target/debug/elyra (ELYRA_BIN overrides) in a throwaway
ELYRA_HOME with ELYRA_NO_ACTIVATE, so it never takes focus; with
BROWSER_CHECK_KEEP=1 that home is kept (the page pictures are in it). Needs
macOS.
"""
import http.server
import json
import os
import re
import shutil
import socketserver
import subprocess
import sys
import tempfile
import threading
import time

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
sys.path.insert(0, os.path.join(ROOT, "scripts", "scenarios"))
import run  # noqa: E402  (the scenario harness: World, the scripted agent)
from run import Failure, expect  # noqa: E402

JOURNEY = "Orders"

PAGE = """<!doctype html><html><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Orders</title><style>
body {{ font: 16px system-ui; margin: 16px; }}
.orders {{ width: 100%; max-width: 700px; }}
{css}
</style></head><body>
<h1>Orders</h1>
<div class="orders" id="orders">Order 1, Order 2</div>
<button id="more">Show more</button>
<p id="width"></p>
<script>
const width = () => document.getElementById('width').textContent = 'width:' + innerWidth;
width(); addEventListener('resize', width);
document.getElementById('more').onclick = () =>
  document.getElementById('orders').insertAdjacentHTML('beforeend', '<div id="shown">Order 3</div>');
</script></body></html>"""

# A page with an error in its console and a request that fails.
ERRORS_PAGE = """<!doctype html><html><head><meta charset="utf-8"><title>Broken</title></head>
<body><h1>Broken</h1><script>
console.error('totals: cannot read property amount of undefined');
fetch('/missing-orders.json');
</script></body></html>"""

# Breaks the layout only below 500 px: the list wider than a phone.
PHONE_ONLY = "@media (max-width: 500px) { .orders { width: 900px; max-width: none; } }"
# Breaks it at any width: the button pushed out of the window.
DESKTOP = "#more { position: relative; left: 2000px; }"


class Site:
    """A page per project, served from this Mac, whose CSS can change."""

    def __init__(self):
        self.dir = tempfile.mkdtemp(prefix="elyra-browser-site-")
        site = self.dir

        class Quiet(http.server.SimpleHTTPRequestHandler):
            def __init__(self, *args, **kwargs):
                super().__init__(*args, directory=site, **kwargs)

            def log_message(self, *args):
                pass

        # Threads: WebKit opens connections ahead of time that would block a
        # server answering one at a time.
        self.server = socketserver.ThreadingTCPServer(("127.0.0.1", 0), Quiet)
        self.server.daemon_threads = True
        threading.Thread(target=self.server.serve_forever, daemon=True).start()

    def url(self, name):
        return f"http://127.0.0.1:{self.server.server_address[1]}/{name}/index.html"

    def write(self, name, css="", page=None):
        os.makedirs(os.path.join(self.dir, name), exist_ok=True)
        with open(os.path.join(self.dir, name, "index.html"), "w") as f:
            f.write(page if page is not None else PAGE.format(css=css))

    def close(self):
        self.server.shutdown()
        shutil.rmtree(self.dir, ignore_errors=True)


def call(world, tool, **arguments):
    """A gateway tool's text, and whether it is an error."""
    reply = world.call_raw({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
                            "params": {"name": tool, "arguments": arguments}})
    expect("result" in reply, f"{tool}: {reply}")
    return reply["result"]["content"][0]["text"], reply["result"]["isError"]


def replay(world, thread):
    text, failed = call(world, "browser_run_journey", thread_id=thread, name=JOURNEY)
    return text, not failed


def page_width(world, thread):
    text, _ = call(world, "browser_query", thread_id=thread, selector="#width")
    found = re.findall(r"width:(\d+)", text)
    expect(found, f"the page's width: {text[:300]}")
    return int(found[0])


def open_page(world, thread, url):
    """Open `url` in the thread's browser; its width once it has laid itself
    out."""
    call(world, "browser_open", thread_id=thread, url=url)
    deadline = time.time() + 15
    width = 0
    while time.time() < deadline and width == 0:
        time.sleep(0.5)
        width = page_width(world, thread)
    return width


def items_of(world, thread, kind):
    return [item for item in world.items(thread) if item["kind"] == kind]


def wait_for(what, seconds=20):
    deadline = time.time() + seconds
    while time.time() < deadline:
        found = what()
        if found:
            return found
        time.sleep(0.5)
    return None


def picture_width(path):
    expect(path and os.path.isfile(path), f"a picture: {path}")
    out = subprocess.run(["sips", "-g", "pixelWidth", path], capture_output=True, text=True).stdout
    return int(out.split()[-1])


def turn_pictures(world, thread, text):
    """Send `text` (a turn that changes a file); the page pictures it left."""
    known = len(items_of(world, thread, "page_snapshot"))
    world.tool("send_message", thread_id=thread, text=text)
    world.wait(thread, 120)
    snaps = wait_for(lambda: items_of(world, thread, "page_snapshot")[known:])
    expect(snaps, "a picture of the page after the turn")
    snap = snaps[0]
    return picture_width(snap.get("before")), picture_width(snap.get("after"))


def pictures(world, site, name, thread):
    """Before and after a turn, on screen; then with journeys after the turn."""
    site.write(name)
    open_page(world, thread, site.url(name))
    before, after = turn_pictures(world, thread, "[write notes.txt] one")
    expect(before == after, f"before and after the same size: {before} and {after} px")
    # Now with the project's journeys replayed after each turn (they resize
    # the page to measure it).
    with world.db() as db:
        project = db.execute("SELECT id FROM projects WHERE name = ?", (name,)).fetchone()[0]
        db.execute("INSERT OR REPLACE INTO settings (key, value) VALUES (?, '1')",
                   (f"journeys_after_turn:{project}",))
    known = len(items_of(world, thread, "journey"))
    with_journeys = turn_pictures(world, thread, "[write notes.txt] two")
    expect(len(items_of(world, thread, "journey")) > known, "the journeys replayed after the turn")
    expect(with_journeys == (before, after),
           f"pictures with journeys after the turn: {with_journeys}, without: {(before, after)}")
    page = page_width(world, thread)
    expect(page < 1280, f"the page's width after the turn: {page}")


def page_errors(world, site, name, thread, shown):
    """What an agent reads of the page's console and requests."""
    site.write(f"{name}-errors", page=ERRORS_PAGE)
    call(world, "browser_open", thread_id=thread, url=site.url(f"{name}-errors"))
    console = wait_for(lambda: (lambda t: "cannot read property amount" in t and t)(
        call(world, "browser_console", thread_id=thread)[0]))
    expect(console, f"the console error: {call(world, 'browser_console', thread_id=thread)[0][:400]}")
    network = call(world, "browser_network", thread_id=thread)[0]
    expect("missing-orders.json" in network and "404" in network, f"the failed request: {network[:400]}")


def journeys(world, site, name, thread, shown):
    """The layout guard's cases for one project's thread."""
    path = world.projects[name]["path"]
    if shown:
        before = open_page(world, thread, site.url(name))
        expect(0 < before < 1280, f"the Browser tab's width: {before}")

    text, passed = replay(world, thread)
    expect(passed, f"the first replay: {text}")
    with open(os.path.join(path, ".elyra", "journeys", "layout", "orders.json")) as f:
        saved = json.load(f)
    problems = [key for step in saved["steps"] for key in step["keys"] + step.get("phone_keys", [])]
    expect(not problems, f"a clean page saved with problems: {sorted(set(problems))}")
    expect(all("phone_keys" in step for step in saved["steps"]), f"measured at a phone's width: {saved}")
    expect(saved.get("width") == 1280, f"measured at 1280 px: {saved}")

    text, passed = replay(world, thread)
    expect(passed, f"the same page again: {text}")

    site.write(name, PHONE_ONLY)
    text, passed = replay(world, thread)
    expect(not passed and "at 390 px wide, the page is too wide" in text, f"broken on a phone: {text}")
    expect("outside the window" not in text, f"only the phone's width is broken: {text}")

    site.write(name, DESKTOP)
    text, passed = replay(world, thread)
    expect(not passed and "outside the window: button#more" in text, f"broken on a desktop: {text}")

    if shown:
        site.write(name)
        replay(world, thread)
        after = page_width(world, thread)
        expect(after == before, f"the page's width after replaying: {after}, was {before}")


def main():
    if not os.path.exists(run.BIN):
        sys.exit(f"{run.BIN} is missing: run cargo build -p elyra-app")
    site = Site()
    world = run.World()
    cases = [("hidden", False), ("shown", True)]
    for name, _ in cases:
        site.write(name)
        journey = {"name": JOURNEY, "steps": [
            {"open": site.url(name)}, {"click": {"text": "Show more"}},
            {"expect": {"selector": "#shown"}}]}
        world.project(name, "true", 0, files={".elyra/journeys/orders.json": json.dumps(journey)})
    failed = 0
    checks = []
    try:
        # A thread per project, made headless; the one whose Browser tab is
        # on screen is open, with that tab showing.
        world.start()
        world.stop()
        world.seed()
        world.start()
        threads = {}
        for name, _ in cases:
            threads[name] = world.new_thread(name, "[talk] hi")
            world.wait(threads[name], 30)
        world.stop()
        with world.db() as db:
            for key, value in [("open_tabs", threads["shown"]), ("layout", "1,1,browser"),
                               ("window_bounds", "60,60,1300,850"), ("onboarding_done", "1")]:
                db.execute("INSERT OR REPLACE INTO settings (key, value) VALUES (?, ?)", (key, value))
        world.start(window=True)
        time.sleep(3)
        checks = []
        for name, shown in cases:
            thread = threads[name]
            checks.append((f"journeys, Browser tab {name}",
                           lambda name=name, thread=thread, shown=shown: journeys(world, site, name, thread, shown)))
        checks.append(("pictures before and after a turn",
                       lambda: pictures(world, site, "shown", threads["shown"])))
        for name, shown in cases:
            thread = threads[name]
            checks.append((f"page errors, Browser tab {name}",
                           lambda name=name, thread=thread, shown=shown: page_errors(world, site, name, thread, shown)))
        for label, check in checks:
            try:
                check()
                print(f"ok    {label}")
            except Failure as err:
                failed += 1
                print(f"FAIL  {label}: {err}")
    finally:
        world.stop()
        site.close()
    if failed:
        print(f"\n{failed} of {len(checks)} failed; the app's log is in {world.dir}")
        sys.exit(1)
    if os.environ.get("BROWSER_CHECK_KEEP"):
        print(f"the app's home is kept in {world.dir}")
    else:
        shutil.rmtree(world.dir, ignore_errors=True)
    print(f"\n{len(checks)} browser checks passed")


if __name__ == "__main__":
    main()

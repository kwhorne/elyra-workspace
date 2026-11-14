#!/usr/bin/env python3
"""Journey check: replay a journey in the real app, with a window (the
headless scenarios have no web pages), against a small local site, and see
that the layout guard measures what it should:

- the first replay saves a clean layout, at the desktop width;
- a page broken only at a phone's width fails at 390 px, and only there;
- a page broken at a desktop's width fails there, and not as a phone problem;
- the page gets its own width back afterwards;

each with the thread's Browser tab on screen and out of sight (as after a
turn, when a page that was never shown has no size at all).

    cargo build -p elyra-app && scripts/journey-check/check.py

The app runs from target/debug/elyra (ELYRA_BIN overrides) in a throwaway
ELYRA_HOME with ELYRA_NO_ACTIVATE, so it never takes focus. Needs macOS.
"""
import http.server
import json
import os
import re
import shutil
import socketserver
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

# Breaks the layout only below 500 px: the list wider than a phone.
PHONE_ONLY = "@media (max-width: 500px) { .orders { width: 900px; max-width: none; } }"
# Breaks it at any width: the button pushed out of the window.
DESKTOP = "#more { position: relative; left: 2000px; }"


class Site:
    """A page per project, served from this Mac, whose CSS can change."""

    def __init__(self):
        self.dir = tempfile.mkdtemp(prefix="elyra-journey-site-")
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

    def write(self, name, css=""):
        os.makedirs(os.path.join(self.dir, name), exist_ok=True)
        with open(os.path.join(self.dir, name, "index.html"), "w") as f:
            f.write(PAGE.format(css=css))

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


def check(world, site, name, thread, shown):
    """The layout guard's cases for one project's thread."""
    path = world.projects[name]["path"]
    if shown:
        call(world, "browser_open", thread_id=thread, url=site.url(name))
        # Until it has loaded and laid itself out in the tab.
        deadline = time.time() + 15
        before = 0
        while time.time() < deadline and before == 0:
            time.sleep(0.5)
            before = page_width(world, thread)
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
        for name, shown in cases:
            label = f"Browser tab {name}"
            try:
                check(world, site, name, threads[name], shown)
                print(f"ok    {label}")
            except Failure as err:
                failed += 1
                print(f"FAIL  {label}: {err}")
    finally:
        world.stop()
        site.close()
    if failed:
        print(f"\n{failed} of {len(cases)} failed; the app's log is in {world.dir}")
        sys.exit(1)
    shutil.rmtree(world.dir, ignore_errors=True)
    print(f"\n{len(cases)} journey checks passed")


if __name__ == "__main__":
    main()

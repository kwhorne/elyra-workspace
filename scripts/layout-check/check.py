#!/usr/bin/env python3
"""Layout check: screenshots of Elyra Workspace in states that tend to break
the layout (many tabs, narrow windows, long names, rich conversations,
panels, the light theme, first launch), and optionally a review of them by
Claude Code against rubric.md.

    cargo build -p elyra-app && scripts/layout-check/check.py [--review] [scene ...]

The app runs from target/debug/elyra (ELYRA_BIN overrides) in throwaway
ELYRA_HOMEs with ELYRA_NO_ACTIVATE, so it never takes focus; each scene is set
up before launch and captured from its first frame. Screenshots and the
review go to target/layout-check/<time>/. Needs macOS (screencapture, swift).
"""
import json
import os
import shutil
import subprocess
import sys
import time
import uuid

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
sys.path.insert(0, os.path.join(ROOT, "scripts", "scenarios"))
import run  # noqa: E402  (the scenario harness: World, the scripted agent)

LONG = [
    "Kan du sjekke pr",
    "Kan du sjekke pr for kundeportalen",
    "Refactor the measurement editor so Insert and Replace keep the text",
    "Jeg vurderer en refaktorering av rapportmodulen",
    "Fix flaky checkout test",
    "Upgrade to Laravel 12 and PHP 8.4",
    "Why does the cart total round wrong for discounts over 100 %?",
    "Add dark mode",
    "Nightly dependency check",
    "Investigate 500 on /api/orders when the customer has no address",
    "Translate the onboarding emails to Norwegian, Swedish and Danish",
    "Clean up",
]

SCENES = [
    # name, window (w, h), layout "sidebar,right,tab", which tabs, env, prefs
    ("tabs-narrow", (1000, 700), "1,1,changes", "all", {}, {}),
    ("tabs-wide", (1700, 950), "1,1,changes", "all", {}, {}),
    ("thread-rich", (1360, 860), "1,1,context", "rich", {}, {}),
    ("thread-rich-narrow", (980, 680), "1,1,changes", "rich", {}, {}),
    ("no-sidebar-no-panel", (1100, 760), "0,0,changes", "rich", {}, {}),
    ("light-theme", (1360, 860), "1,1,changes", "rich", {}, {"theme": "Default Light"}),
    ("files-sheet", (1360, 860), "1,1,files", "rich", {"ELYRA_OPEN_PANEL": "files:value.txt"}, {}),
    ("panel-tasks", (1100, 720), "1,1,changes", "rich", {"ELYRA_OPEN_PANEL": "tasks"}, {}),
    ("panel-automations", (1100, 720), "1,1,changes", "rich", {"ELYRA_OPEN_PANEL": "automations"}, {}),
    ("panel-stats", (1100, 720), "1,1,changes", "rich", {"ELYRA_OPEN_PANEL": "stats"}, {}),
    ("panel-review", (1100, 720), "1,1,changes", "rich", {"ELYRA_OPEN_PANEL": "review"}, {}),
]


def rich_items():
    """Transcript entries that exercise every kind of row."""
    markdown = (
        "Here is what I found:\n\n| File | Problem |\n| --- | --- |\n"
        "| `app/Http/Controllers/OrderController.php` | missing null check on `$customer->address` |\n"
        "| `resources/views/pages/ipa/projects/⚡edit.blade.php` | `Flux::modal(...)->close()` does nothing |\n\n"
        "```php\nif ($customer->address === null) {\n    return response()->json(['error' => 'no address'], 422);\n}\n```"
    )
    return [
        {"kind": "user", "text": "Why does /api/orders return 500 for some customers? Check the logs and fix it."},
        {"kind": "tool_use", "tool_use_id": "t1", "name": "Bash",
         "input": {"command": "tail -n 200 storage/logs/laravel.log | grep -n 'ErrorException' | head -40 && php artisan route:list --path=api/orders"}},
        {"kind": "tool_result", "tool_use_id": "t1", "content": "ErrorException: Attempt to read property \"street\" on null", "is_error": False},
        {"kind": "tool_use", "tool_use_id": "t2", "name": "Edit",
         "input": {"file_path": "app/Http/Controllers/OrderController.php", "old_string": "$customer->address->street", "new_string": "$customer->address?->street"}},
        {"kind": "tool_result", "tool_use_id": "t2", "content": "ok", "is_error": False},
        {"kind": "assistant", "text": markdown},
        {"kind": "turn_summary", "duration_ms": 84200, "cost_usd": 0.412, "is_error": False},
        {"kind": "check", "command": "vendor/bin/pint --test && php artisan test --parallel --stop-on-failure", "passed": False,
         "exit_code": 1, "duration_ms": 41200, "output": "FAILED  Tests\\Feature\\OrderApiTest > it returns 422 without an address\nExpected 422, got 500."},
        {"kind": "notice", "text": "The checks still fail after 2 automatic fixes; over to you.", "is_error": True},
        {"kind": "check", "command": "vendor/bin/pint --test && php artisan test --parallel --stop-on-failure", "passed": True,
         "duration_ms": 39800, "output": "Tests: 214 passed"},
    ]


def capture(world, scene, out_dir, tabs):
    name, (width, height), layout, which, env, prefs = scene
    with world.db() as db:
        settings = {
            "open_tabs": ",".join(tabs[which]),
            "layout": layout,
            "window_bounds": f"60,60,{width},{height}",
            "onboarding_done": "1",
        }
        row = db.execute("SELECT value FROM settings WHERE key = 'preferences'").fetchone()
        merged = json.loads(row[0]) if row else {}
        merged.update({"theme": "Default Dark"}, **prefs)
        settings["preferences"] = json.dumps(merged)
        for key, value in settings.items():
            db.execute("INSERT OR REPLACE INTO settings (key, value) VALUES (?, ?)", (key, value))
    return launch_and_capture(world.home, name, out_dir, env)


def launch_and_capture(home, name, out_dir, env=None):
    log = open(os.path.join(out_dir, f"{name}.log"), "w")
    environment = dict(os.environ, ELYRA_HOME=home, ELYRA_NO_ACTIVATE="1", **(env or {}))
    app = subprocess.Popen([run.BIN], env=environment, stdout=log, stderr=log)
    path = os.path.join(out_dir, f"{name}.png")
    try:
        window = None
        deadline = time.time() + 20
        while time.time() < deadline and not window:
            time.sleep(1)
            window = subprocess.run(["swift", os.path.join(HERE, "winid.swift"), str(app.pid)],
                                    capture_output=True, text=True).stdout.strip()
        if not window:
            print(f"FAIL  {name}: no window")
            return None
        time.sleep(4)  # let it lay out and load
        # The largest window by now (a small helper window may come first).
        window = subprocess.run(["swift", os.path.join(HERE, "winid.swift"), str(app.pid)],
                                capture_output=True, text=True).stdout.strip() or window
        subprocess.run(["screencapture", "-o", "-x", "-l", window, path], check=True)
        print(f"shot  {name}")
        return path
    finally:
        app.terminate()
        try:
            app.wait(10)
        except subprocess.TimeoutExpired:
            app.kill()


def build_world():
    """Projects with long names and colours, a dozen threads, one rich one."""
    world = run.World()
    world.start()
    world.stop()
    names = ["inside-helpdesk", "outside", "ElyraSQL-Client", "e", "a-project-with-a-very-long-name-indeed"]
    for name in names:
        world.project(name, "php artisan test", 0)
    world.seed()
    with world.db() as db:
        for (name,), (icon, color) in zip(db.execute("SELECT name FROM projects").fetchall(),
                                           [("🛟", "#e5484d"), (None, "#3e63dd"), ("🗄️", None), (None, None), (None, "#30a46c")]):
            db.execute("UPDATE projects SET icon = ?, color = ? WHERE name = ?", (icon, color, name))
    world.start()
    threads = []
    for index, title in enumerate(LONG):
        project = names[index % len(names)]
        thread = world.new_thread(project, "[talk] hi")
        world.wait(thread, 30)
        world.tool("set_thread_title", thread_id=thread, title=title)
        threads.append(thread)
    rich = world.new_thread(
        "outside",
        '[call propose_automation {"name": "Nightly dependency check", "prompt": "Run composer outdated and npm outdated; '
        'update anything with a security fix, run the tests and open a pull request.", "schedule": {"kind": "weekdays", "time": "02:00"}}]')
    world.wait(rich, 30)
    world.tool("set_thread_title", thread_id=rich, title="Investigate 500 on /api/orders when the customer has no address")
    world.stop()
    with world.db() as db:
        seq = db.execute("SELECT COALESCE(MAX(seq), 0) FROM transcript_items WHERE thread_id = ?", (rich,)).fetchone()[0]
        for offset, item in enumerate(rich_items(), start=1):
            db.execute("INSERT INTO transcript_items (id, thread_id, seq, content, created_at) VALUES (?, ?, ?, ?, ?)",
                       (uuid.uuid4().hex, rich, seq + offset, json.dumps(item), "2026-01-01T00:00:00Z"))
    return world, {"all": threads + [rich], "rich": [rich]}


def review(out_dir, shots):
    """Ask Claude Code to judge the screenshots against the rubric."""
    if not shutil.which("claude"):
        print("claude (Claude Code) isn't on PATH; skipping the review")
        return
    listing = "\n".join(f"- {os.path.basename(p)[:-4]}: {p}" for p in shots)
    prompt = (open(os.path.join(HERE, "rubric.md")).read()
              + f"\n\nRead each screenshot with the Read tool:\n{listing}\n\nWrite the review in Markdown.")
    print("review: asking Claude Code (this takes a minute or two)…")
    result = subprocess.run(
        ["claude", "-p", prompt, "--allowedTools", "Read", "--add-dir", out_dir],
        capture_output=True, text=True, timeout=900)
    report = os.path.join(out_dir, "review.md")
    with open(report, "w") as f:
        f.write(result.stdout or result.stderr)
    print(f"review: {report}")


def main():
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    wanted = [s for s in SCENES if not args or s[0] in args]
    if not os.path.exists(run.BIN):
        sys.exit(f"{run.BIN} is missing: run cargo build -p elyra-app")
    out_dir = os.path.join(ROOT, "target", "layout-check", time.strftime("%Y%m%d-%H%M%S"))
    os.makedirs(out_dir)
    world, tabs = build_world()
    shots = []
    try:
        for scene in wanted:
            shot = capture(world, scene, out_dir, tabs)
            if shot:
                shots.append(shot)
        if not args or "onboarding" in args:
            fresh = os.path.join(world.dir, "fresh")
            os.makedirs(fresh)
            shot = launch_and_capture(fresh, "onboarding", out_dir)
            if shot:
                shots.append(shot)
    finally:
        world.stop()
        shutil.rmtree(world.dir, ignore_errors=True)
    print(f"\n{len(shots)} screenshots in {out_dir}")
    if "--review" in sys.argv:
        review(out_dir, shots)


if __name__ == "__main__":
    main()

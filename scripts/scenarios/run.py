#!/usr/bin/env python3
"""End-to-end scenarios: the real app runs headless (ELYRA_HEADLESS, no
window) in a throwaway ELYRA_HOME, driven through the agent gateway, with
agents played by agent.py.

    cargo build -p elyra-app && scripts/scenarios/run.py [scenario ...]

ELYRA_BIN overrides target/debug/elyra. Standard library only.
"""
import datetime
import json
import os
import shutil
import signal
import socket
import sqlite3
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request
import uuid

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
BIN = os.environ.get("ELYRA_BIN", os.path.join(ROOT, "target", "debug", "elyra"))
TOKEN = "scenario-token"
AT_LEAST_TWO = 'test "$(cat value.txt)" -ge 2 || { echo "value is $(cat value.txt), expected 2"; exit 1; }'

SCENARIOS = []


def scenario(check=None, value=0, files=None, allow_mcp_json=False, instructions=None):
    """Register a scenario with the project it runs in: its check command,
    value.txt, extra files, whether its .mcp.json is allowed and the
    instructions Elyra gives its agents."""
    def register(fn):
        SCENARIOS.append((fn.__name__, fn, {"check": check, "value": value, "files": files or {},
                                            "allow_mcp_json": allow_mcp_json,
                                            "instructions": instructions}))
        return fn
    return register


def fnv1a(data):
    """The fingerprint Elyra keeps for an allowed .mcp.json."""
    value = 0xcbf29ce484222325
    for byte in data:
        value = ((value ^ byte) * 0x100000001b3) & 0xFFFFFFFFFFFFFFFF
    return f"{value:016x}"


class Failure(Exception):
    pass


def expect(condition, message):
    if not condition:
        raise Failure(message)


def listening(port):
    try:
        socket.create_connection(("127.0.0.1", port), timeout=1).close()
        return True
    except OSError:
        return False


class World:
    """A throwaway home with one Git project per scenario, and the app."""

    def __init__(self):
        self.dir = tempfile.mkdtemp(prefix="elyra-scenarios-")
        self.home = os.path.join(self.dir, "home")
        self.projects = {}
        self.app = None
        self.port = None

    def project(self, name, check, value, files=None, allow_mcp_json=False, instructions=None):
        path = os.path.join(self.dir, name)
        os.makedirs(path)
        for file, text in (files or {}).items():
            with open(os.path.join(path, file), "w") as f:
                f.write(text)
        git = lambda *args: subprocess.run(["git", *args], cwd=path, check=True, capture_output=True)
        git("init", "-q")
        with open(os.path.join(path, "value.txt"), "w") as f:
            f.write(f"{value}\n")
        with open(os.path.join(path, ".gitignore"), "w") as f:
            f.write("prompts.log\nmodels.log\ninstructions.log\n")
        git("add", ".")
        git("-c", "user.email=s@s", "-c", "user.name=s", "commit", "-qm", "start")
        self.projects[name] = {"path": path, "check": check, "allow_mcp_json": allow_mcp_json,
                               "instructions": instructions}
        return path

    def start(self):
        log = open(os.path.join(self.dir, "app.log"), "a")
        env = dict(
            os.environ, ELYRA_HOME=self.home, ELYRA_HEADLESS="1", RUST_LOG="info",
            # Fake tools (gh) first on the PATH the app takes from its login shell.
            SHELL=os.path.join(HERE, "login-shell"),
            SCENARIO_PATH=os.path.join(HERE, "bin") + os.pathsep + os.environ.get("PATH", ""),
            FAKE_GH_STATE=os.path.join(self.dir, "gh.json"),
            ELYRA_REVIEW_POLL_SECS="2",
            ELYRA_EVENT_POLL_SECS="1",
        )
        self.app = subprocess.Popen([BIN], env=env, stdout=log, stderr=log)
        deadline = time.time() + 60
        while time.time() < deadline:
            self.port = self.setting("mcp_port")
            if self.port and listening(int(self.port)):
                return
            if self.app.poll() is not None:
                raise Failure(f"the app exited ({self.app.returncode}); see {log.name}")
            time.sleep(0.3)
        raise Failure("the app's gateway did not come up")

    def stop(self):
        if self.app and self.app.poll() is None:
            self.app.send_signal(signal.SIGTERM)
            try:
                self.app.wait(10)
            except subprocess.TimeoutExpired:
                self.app.kill()
                self.app.wait()

    def db(self):
        return sqlite3.connect(os.path.join(self.home, "state.db"), timeout=10)

    def setting(self, key):
        try:
            with self.db() as db:
                row = db.execute("SELECT value FROM settings WHERE key = ?", (key,)).fetchone()
                return row[0] if row else None
        except sqlite3.Error:
            return None

    def seed(self):
        """Projects, a paired gateway client and the scripted agent."""
        with self.db() as db:
            for name, project in self.projects.items():
                project_id = str(uuid.uuid4())
                db.execute(
                    "INSERT INTO projects (id, name, path, created_at, check_command, instructions) "
                    "VALUES (?, ?, ?, ?, ?, ?)",
                    (project_id, name, project["path"], "2026-01-01T00:00:00Z", project["check"],
                     project.get("instructions")),
                )
                if project["allow_mcp_json"]:
                    with open(os.path.join(project["path"], ".mcp.json"), "rb") as f:
                        fingerprint = fnv1a(f.read())
                    db.execute("INSERT INTO settings (key, value) VALUES (?, ?)",
                               (f"mcp_json_allowed:{project_id}", fingerprint))
            client = str(uuid.uuid4())
            db.execute("INSERT INTO mcp_clients (id, data) VALUES (?, ?)", (client, json.dumps({
                "id": client, "name": "Scenarios", "token": TOKEN, "scope": "full",
                "created_at": "2026-01-01T00:00:00Z"})))
            row = db.execute("SELECT value FROM settings WHERE key = 'preferences'").fetchone()
            prefs = json.loads(row[0]) if row else {}
            prefs.setdefault("providers", {})["acp"] = {
                "path": sys.executable, "args": os.path.join(HERE, "agent.py"),
                "escalation_model": "big"}
            # The scripted agent also stands in for Gemini CLI.
            prefs["providers"]["gemini"] = {
                "path": sys.executable, "args": os.path.join(HERE, "agent.py")}
            prefs["check_fix_attempts"] = 2
            prefs["agent_gateway"] = True
            prefs["check_updates"] = False
            db.execute("INSERT OR REPLACE INTO settings (key, value) VALUES ('preferences', ?)",
                       (json.dumps(prefs),))

    # ---- the gateway ------------------------------------------------------

    def call_raw(self, message):
        request = urllib.request.Request(
            f"http://127.0.0.1:{self.port}/mcp",
            data=json.dumps(message).encode(),
            headers={"Authorization": f"Bearer {TOKEN}", "Content-Type": "application/json"},
        )
        try:
            with urllib.request.urlopen(request, timeout=300) as response:
                body = response.read()
                return json.loads(body) if body else {}
        except urllib.error.HTTPError as err:
            raise Failure(f"gateway: HTTP {err.code}") from err

    def tool(self, tool_name, **arguments):
        reply = self.call_raw({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
                               "params": {"name": tool_name, "arguments": arguments}})
        expect("result" in reply, f"{tool_name}: {reply}")
        text = reply["result"]["content"][0]["text"]
        expect(not reply["result"]["isError"], f"{tool_name}: {text}")
        return text

    def new_thread(self, project, prompt):
        text = self.tool("create_thread", project=project, prompt=prompt, provider="acp")
        return json.loads(text)["thread_id"]

    def wait(self, thread, seconds=120):
        return self.tool("wait_for_thread", thread_id=thread, timeout_seconds=seconds)

    # ---- what happened -----------------------------------------------------

    def items(self, thread):
        with self.db() as db:
            rows = db.execute(
                "SELECT content FROM transcript_items WHERE thread_id = ? ORDER BY seq", (thread,)
            ).fetchall()
        return [json.loads(row[0]) for row in rows]

    def status(self, thread):
        with self.db() as db:
            return db.execute("SELECT status FROM threads WHERE id = ?", (thread,)).fetchone()[0]

    def value(self, project):
        with open(os.path.join(self.projects[project]["path"], "value.txt")) as f:
            return int(f.read().strip())

    def prompts(self, project):
        path = os.path.join(self.projects[project]["path"], "prompts.log")
        return open(path).read().splitlines() if os.path.exists(path) else []


def checks_of(items):
    return [item["passed"] for item in items if item["kind"] == "check"]


# ---- scenarios --------------------------------------------------------------


@scenario(check=AT_LEAST_TWO, value=0)
def failed_checks_go_back_to_the_agent(world, project):
    thread = world.new_thread(project, "Make the value right")
    reply = world.wait(thread)
    expect("Status: idle" in reply, f"done when green: {reply}")
    items = world.items(thread)
    expect(checks_of(items) == [False, True], f"checks: {checks_of(items)}")
    expect(world.value(project) == 2, f"value {world.value(project)}")
    expect(len(world.prompts(project)) == 2, "one automatic fix")
    expect("checks failed" in world.prompts(project)[1], "the fix carries the failure")


@scenario(check=AT_LEAST_TWO, value=-5)
def checks_hand_over_after_the_attempts(world, project):
    thread = world.new_thread(project, "Make the value right")
    world.wait(thread)
    items = world.items(thread)
    expect(checks_of(items) == [False, False, False], f"checks: {checks_of(items)}")
    expect(world.status(thread) == "failed", f"status {world.status(thread)}")
    expect(any(i["kind"] == "notice" and "over to you" in i["text"] for i in items), "said so")


@scenario(check=AT_LEAST_TWO, value=-5)
def a_second_fix_gets_the_escalation_model(world, project):
    thread = world.new_thread(project, "Make the value right")
    world.wait(thread)
    models = os.path.join(world.projects[project]["path"], "models.log")
    switched = open(models).read().split() if os.path.exists(models) else []
    expect(switched == ["big"], f"switched to {switched}")
    notices = [i["text"] for i in world.items(thread) if i["kind"] == "notice"]
    expect(any("model big" in n for n in notices), f"said so: {notices}")
    with world.db() as db:
        model = db.execute("SELECT model FROM threads WHERE id = ?", (thread,)).fetchone()[0]
    expect(model is None, f"back to its own model, not {model}")


@scenario(check="echo checked; exit 1", value=0)
def turns_that_change_nothing_are_not_checked(world, project):
    thread = world.new_thread(project, "[talk] Just answer")
    world.wait(thread)
    expect(checks_of(world.items(thread)) == [], "no checks")
    expect(world.status(thread) == "idle", f"status {world.status(thread)}")


@scenario(check="sleep 60", value=0)
def stop_ends_running_checks(world, project):
    thread = world.new_thread(project, "Change something")
    deadline = time.time() + 30
    while not any(i["kind"] == "turn_summary" for i in world.items(thread)):
        expect(time.time() < deadline, "the turn did not end")
        time.sleep(0.2)
    time.sleep(1)  # the checks are running now
    started = time.time()
    world.tool("interrupt_thread", thread_id=thread)
    world.wait(thread, 30)
    expect(time.time() - started < 10, "stopped quickly")
    checks = [i for i in world.items(thread) if i["kind"] == "check"]
    expect(len(checks) == 1 and not checks[0]["passed"], f"checks: {checks}")
    expect(world.status(thread) != "running", "not running")


@scenario()
def an_agent_that_crashes_mid_turn(world, project):
    thread = world.new_thread(project, "[crash] Go")
    world.wait(thread, 30)
    expect(world.status(thread) != "running", f"status {world.status(thread)}")
    # The next message starts the agent again.
    world.tool("send_message", thread_id=thread, text="Try again")
    world.wait(thread, 30)
    expect(world.value(project) == 1, f"value {world.value(project)}")


@scenario()
def an_agent_that_fails_the_turn(world, project):
    thread = world.new_thread(project, "[error] Go")
    world.wait(thread, 30)
    items = world.items(thread)
    expect(world.status(thread) == "failed", f"status {world.status(thread)}")
    expect(any(i["kind"] == "notice" and i.get("is_error") for i in items), "the error is shown")


@scenario()
def stop_cancels_a_long_turn(world, project):
    thread = world.new_thread(project, "[wait] Take your time")
    time.sleep(1.5)
    world.tool("interrupt_thread", thread_id=thread)
    reply = world.wait(thread, 20)
    expect("Status: running" not in reply, reply)
    expect(world.status(thread) != "running", f"status {world.status(thread)}")


@scenario()
def agents_reach_the_gateway_as_themselves(world, project):
    thread = world.new_thread(project, '[call list_threads {"project": "%s"}]' % project)
    reply = world.wait(thread, 30)
    expect(thread in reply, f"the agent saw its own thread: {reply[:300]}")


@scenario()
def agents_propose_automations_for_the_user_to_accept(world, project):
    schedule = '{"kind": "daily", "time": "02:00"}'
    thread = world.new_thread(
        project,
        '[call propose_automation {"name": "Nightly deps", "prompt": "Check for outdated dependencies", "schedule": %s}]'
        % schedule,
    )
    reply = world.wait(thread, 30)
    expect("as a card in this thread" in reply, reply[-300:])
    proposals = [i for i in world.items(thread) if i["kind"] == "automation_proposal"]
    expect(len(proposals) == 1, f"one card: {proposals}")
    expect(proposals[0]["automation"]["name"] == "Nightly deps", "the proposed automation")
    expect("outcome" not in proposals[0], "the user hasn't decided")
    with world.db() as db:
        expect(db.execute("SELECT COUNT(*) FROM automations").fetchone()[0] == 0, "nothing scheduled yet")
    # A schedule that never runs is refused.
    thread = world.new_thread(
        project,
        '[call propose_automation {"name": "Past", "prompt": "x", "schedule": {"kind": "once", "at": "2020-01-01T00:00:00Z"}}]',
    )
    expect("never runs" in world.wait(thread, 30), "refused")
    # Only agents in a thread can propose.
    try:
        world.tool("propose_automation", name="x", prompt="x", schedule={"kind": "interval", "minutes": 5})
        raise Failure("a client could propose")
    except Failure as err:
        expect("Only an agent" in str(err), str(err))


MCP_JSON = '{"mcpServers": {"probe": {"command": "/bin/cat", "env": {"MODE": "${MISSING:-test}"}}}}'


@scenario(files={".mcp.json": MCP_JSON}, allow_mcp_json=True)
def an_allowed_mcp_json_reaches_other_agents(world, project):
    thread = world.new_thread(project, "[mcp] Which servers?")
    reply = world.wait(thread, 30)
    expect("probe" in reply and "elyra" in reply, reply[-200:])


@scenario(files={".mcp.json": MCP_JSON}, allow_mcp_json=True)
def servers_that_stay_the_same_stay_allowed(world, project):
    # Allowing remembers each server; boost:update writing the file again
    # with one more server keeps the others shared.
    with world.db() as db:
        project_id = db.execute("SELECT id FROM projects WHERE name = ?", (project,)).fetchone()[0]
        spec = json.loads(MCP_JSON)["mcpServers"]["probe"]
        entry = "probe\0" + json.dumps(spec, separators=(",", ":"), sort_keys=True)
        db.execute("INSERT OR REPLACE INTO settings (key, value) VALUES (?, ?)",
                   (f"mcp_json_servers:{project_id}", fnv1a(entry.encode())))
    path = world.projects[project]["path"]
    servers = json.loads(MCP_JSON)
    servers["mcpServers"]["fresh"] = {"command": "/bin/echo"}
    with open(os.path.join(path, ".mcp.json"), "w") as f:
        json.dump(servers, f, indent=4)
    thread = world.new_thread(project, "[mcp] Which servers?")
    reply = world.wait(thread, 30)
    expect("probe" in reply and "fresh" not in reply, "the unchanged one only: " + reply[-200:])


@scenario(files={"composer.json": '{"require": {"laravel/framework": "^13.0"}}', "artisan": ""})
def a_worktree_gets_what_git_leaves_out(world, project):
    path = world.projects[project]["path"]
    with open(os.path.join(path, ".gitignore"), "a") as f:
        f.write(".env\nvendor/\n")
    with open(os.path.join(path, ".env"), "w") as f:
        f.write("APP_ENV=local\n")
    os.makedirs(os.path.join(path, "vendor"))
    with open(os.path.join(path, "vendor", "autoload.php"), "w") as f:
        f.write("<?php\n")
    subprocess.run(["git", "-c", "user.email=s@s", "-c", "user.name=s", "commit", "-qam", "ignore"],
                   cwd=path, check=True)
    text = world.tool("create_thread", project=project, prompt="[talk]", provider="acp", worktree=True)
    thread = json.loads(text)["thread_id"]
    world.wait(thread, 60)
    with world.db() as db:
        environment = db.execute("SELECT environment FROM threads WHERE id = ?", (thread,)).fetchone()[0]
    worktree = json.loads(environment).get("path")
    expect(worktree and worktree != path, f"a worktree: {environment}")
    expect(os.path.isfile(os.path.join(worktree, ".env")), "the .env came along")
    expect(os.path.isfile(os.path.join(worktree, "vendor", "autoload.php")), "and vendor/")
    notices = [i["text"] for i in world.items(thread) if i["kind"] == "notice"]
    expect(any(n.startswith("Copied into the worktree") and ".env" in n for n in notices), str(notices))


@scenario(files={".mcp.json": MCP_JSON})
def an_mcp_json_nobody_allowed_is_not_passed_on(world, project):
    thread = world.new_thread(project, "[mcp] Which servers?")
    reply = world.wait(thread, 30)
    expect("probe" not in reply and "elyra" in reply, reply[-200:])


@scenario()
def agents_save_journeys_with_the_project(world, project):
    steps = [{"open": "http://shop.test/"}, {"click": {"text": "Add to cart"}}]
    thread = world.new_thread(project, "[call browser_save_journey %s]" % json.dumps(
        {"name": "Add to cart", "steps": steps, "expect": ["1 item"]}))
    reply = world.wait(thread, 30)
    expect("Saved" in reply and "3 steps" in reply, reply[-300:])
    path = os.path.join(world.projects[project]["path"], ".elyra", "journeys", "add-to-cart.json")
    saved = json.load(open(path))
    expect(saved["steps"][-1] == {"expect": {"text": "1 item"}}, saved)
    thread = world.new_thread(project, "[call browser_list_journeys {}]")
    expect("Add to cart" in world.wait(thread, 30), "listed")
    # A journey that proves nothing is refused.
    thread = world.new_thread(project, "[call browser_save_journey %s]" % json.dumps(
        {"name": "Nothing", "steps": steps}))
    expect("expect step" in world.wait(thread, 30), "refused without expect")


@scenario()
def agents_propose_rules_for_the_user_to_keep(world, project):
    thread = world.new_thread(project, '[call propose_rule {"rule": "Validate requests with Form Requests.", "reason": "you said so"}]')
    reply = world.wait(thread, 30)
    expect("as a card" in reply, reply[-200:])
    cards = [i for i in world.items(thread) if i["kind"] == "rule_proposal"]
    expect(len(cards) == 1 and cards[0]["rule"] == "Validate requests with Form Requests.", cards)
    expect("outcome" not in cards[0], "the user hasn't decided")
    expect(not os.path.exists(os.path.join(world.projects[project]["path"], "AGENTS.md")), "no file written yet")


def github(world, **state):
    with open(os.path.join(world.dir, "gh.json"), "w") as f:
        json.dump(state, f)


def pull_request(head, commits, state="OPEN", reviewed="c1", requested=False):
    return {
        "title": "Fix totals", "url": "https://github.com/acme/shop/pull/7", "state": state,
        "headRefOid": head, "commits": [{"oid": c} for c in commits],
        "reviews": [{"author": {"login": "kh"}, "state": "COMMENTED", "commit": {"oid": reviewed}}],
        "reviewRequests": [{"login": "kh"}] if requested else [],
    }


@scenario()
def a_reviewed_pull_request_comes_back_when_it_changes(world, project):
    github(world, login="kh", pr=pull_request("c1", ["c1"]))
    thread = world.new_thread(project, "[say] Posted the review: https://github.com/acme/shop/pull/7#pullrequestreview-1")
    world.wait(thread, 30)
    time.sleep(3)  # followed now
    github(world, login="kh", pr=pull_request("c3", ["c1", "c2", "c3"], requested=True))
    deadline = time.time() + 20
    cards = []
    while time.time() < deadline and not cards:
        cards = [i for i in world.items(thread) if i["kind"] == "pr_update"]
        time.sleep(0.5)
    expect(len(cards) == 1, f"one card: {cards}")
    card = cards[0]
    expect((card["number"], card["commits"], card["requested"], card["from"], card["to"]) == (7, 2, True, "c1", "c3"), card)
    time.sleep(5)
    expect(len([i for i in world.items(thread) if i["kind"] == "pr_update"]) == 1, "told once")
    # Merged: no longer followed.
    github(world, login="kh", pr=pull_request("c3", ["c1", "c2", "c3"], state="MERGED"))
    deadline = time.time() + 20
    while time.time() < deadline:
        if any(i["kind"] == "notice" and "merged" in i["text"] for i in world.items(thread)):
            break
        time.sleep(0.5)
    else:
        raise Failure("no word about the merge")


RUST = """pub struct Cart { total: u32 }

impl Cart {
    pub fn total(&self) -> u32 { discount(self.total) }
}

fn discount(amount: u32) -> u32 { amount * 9 / 10 }
"""


@scenario(files={"lib.rs": RUST})
def agents_look_up_symbols(world, project):
    thread = world.new_thread(project, '[call symbols {"name": "discount"}]')
    reply = world.wait(thread, 60)
    expect("Definitions (1):" in reply and "lib.rs:7" in reply, reply[-400:])
    expect("References (1):" in reply and "in total" in reply, "the caller: " + reply[-400:])
    # A client names the project.
    text = world.tool("symbols", name="Cart", mode="definitions", project=project)
    expect("struct Cart" in text, text)


@scenario(files={"lib.rs": "// written by hand before any agent\n"})
def lines_lead_back_to_the_message_that_wrote_them(world, project):
    thread = world.new_thread(project, "[write lib.rs] fn discount(amount: u32) -> u32 { amount * 9 / 10 }")
    world.wait(thread, 30)
    world.tool("set_thread_title", thread_id=thread, title="Add the discount")
    time.sleep(1.5)  # the turn's lines are recorded after it ends
    why = world.tool("why", path="lib.rs", line=2, project=project)
    expect("Written in the thread “Add the discount”" in why, why)
    expect("[write lib.rs] fn discount" in why and "Wrote lib.rs." in why, "the message and the answer: " + why)
    expect("No person has marked it as read yet." in why, "nobody has read it: " + why)
    by_hand = world.tool("why", path="lib.rs", line=1, project=project)
    expect("No agent in Elyra Workspace is recorded" in by_hand, by_hand)
    history = world.tool("history", path="lib.rs", project=project)
    expect("“Add the discount”" in history and "1 line" in history, history)
    by_symbol = world.tool("history", symbol="discount", project=project)
    expect(by_symbol.startswith("discount: lib.rs:2") and "Add the discount" in by_symbol, by_symbol)


@scenario(files={"lib.rs": "// written by hand before any agent\n"})
def the_agent_that_wrote_a_line_says_why(world, project):
    author = world.new_thread(project, "[write lib.rs] fn discount(amount: u32) -> u32 { amount * 9 / 10 }")
    world.wait(author, 30)
    world.tool("send_message", thread_id=author, text="[write lib.rs] fn shipping() -> u32 { 49 }")
    world.wait(author, 30)
    # The other-agents note names it by title; keep the messages' tags out of it.
    world.tool("set_thread_title", thread_id=author, title="Discount and shipping")
    time.sleep(1.5)  # the turn's lines are recorded after it ends
    asker = world.new_thread(
        project, '[call ask_author {"path": "lib.rs", "line": 2, "question": "Why nine tenths?"}]')
    reply = world.wait(asker, 90)
    expect("The agent that wrote lib.rs:2 answered" in reply, reply[-600:])
    expect("<earlier-conversation>" in reply and "[write lib.rs] fn discount" in reply,
           "it has the conversation that wrote it: " + reply[-1500:])
    expect("[write lib.rs] fn shipping" not in reply, "but not what came after: " + reply[-1500:])
    expect("What that turn changed in the file:" in reply and "+fn discount" in reply,
           "and what that turn changed: " + reply[-1500:])
    expect("fn shipping() -> u32 { 49 }" in reply and ">    2  fn discount" in reply,
           "and the code as it is now: " + reply[-1500:])
    expect(reply.rstrip().endswith("Question: Why nine tenths?"), reply[-300:])
    with world.db() as db:
        chat = db.execute("SELECT title, parent_id, permission_mode FROM threads WHERE title LIKE 'Why%'").fetchone()
    expect(chat is not None and chat[0] == "Why · lib.rs:2" and chat[1] == asker and chat[2] == "plan",
           f"a read-only side chat of the asking thread: {chat}")
    by_hand = world.call_raw({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {
        "name": "ask_author", "arguments": {"path": "lib.rs", "line": 1, "project": project}}})
    text = by_hand["result"]["content"][0]["text"]
    expect(by_hand["result"]["isError"] and "no agent in Elyra Workspace is recorded" in text, text)


@scenario(files={"composer.json": "{}"})
def app_errors_lead_back_to_the_instruction(world, project):
    path = world.projects[project]["path"]
    os.makedirs(os.path.join(path, "app"))
    os.makedirs(os.path.join(path, "storage", "logs"))
    thread = world.new_thread(project, "[write app/Cart.php] return $item['price'];")
    world.wait(thread, 30)
    world.tool("set_thread_title", thread_id=thread, title="Cart prices")
    time.sleep(1.5)  # the turn's lines are recorded after it ends
    log = ('[2026-10-15 12:00:02] local.ERROR: Undefined array key "price" {"exception":"[object] '
           '(ErrorException(code: 0): Undefined array key \\"price\\" at /var/www/html/app/Cart.php:1)\n'
           '[stacktrace]\n#0 /var/www/html/vendor/laravel/framework/src/X.php(9): x()\n'
           '#1 /var/www/html/app/Cart.php(1): y()\n"}\n')
    with open(os.path.join(path, "storage", "logs", "laravel.log"), "w") as f:
        f.write(log)
    text = world.tool("error_origin", project=project)
    expect('ERROR: Undefined array key "price"' in text, text)
    expect("Failing line in the app's code: app/Cart.php:1:" in text, "found from the container's path: " + text)
    expect("Written by the agent in “Cart prices”" in text and "[write app/Cart.php]" in text, text)


@scenario(files={"tax.rs": "// VAT\n"})
def rewriting_another_instructions_lines_is_noticed(world, project):
    author = world.new_thread(project, "[write tax.rs] if customer.is_foreign() { return 0; }")
    world.wait(author, 30)
    world.tool("set_thread_title", thread_id=author, title="VAT for foreign customers")
    time.sleep(1.5)  # the turn's lines are recorded after it ends
    tidy = world.new_thread(
        project, "[replace tax.rs] if customer.is_foreign() { return 0; } => let vat = total / 4;")
    world.wait(tidy, 30)
    time.sleep(1.5)
    cards = [i for i in world.items(tidy) if i["kind"] == "rewrote"]
    expect(len(cards) == 1, f"one card: {[i['kind'] for i in world.items(tidy)]}")
    intent = cards[0]["instructions"][0]
    expect(intent["title"] == "VAT for foreign customers" and intent["files"] == ["tax.rs"]
           and intent["count"] == 1 and intent["instruction"].startswith("[write tax.rs] if customer"), str(intent))
    expect("is_foreign" in intent["lines"][0], str(intent))
    world.tool("send_message", thread_id=tidy, text="[talk] carry on")
    world.wait(tidy, 30)
    prompt = world.prompts(project)[-1]  # the log keeps a prompt's start only
    expect(prompt.startswith("<rewritten-intent>") and "“VAT for foreign customers”" in prompt,
           "the agent hears of it: " + prompt[:300])
    # Moving lines isn't taking them out, and the author's own later turns aren't flagged.
    world.tool("send_message", thread_id=author, text="[write tax.rs] // reviewed")
    world.wait(author, 30)
    time.sleep(1.5)
    expect(not [i for i in world.items(author) if i["kind"] == "rewrote"], "nothing to flag")


@scenario(files={"lib.rs": RUST})
def parallel_agents_see_what_the_others_change(world, project):
    first = world.new_thread(project, "[edit lib.rs] fn member_discount(total: u32) -> u32 { total * 85 / 100 }")
    world.wait(first, 30)
    world.tool("set_thread_title", thread_id=first, title="Member discount")
    second = world.new_thread(project, "[edit lib.rs] fn shipping(total: u32) -> u32 { 49 }")
    world.wait(second, 30)
    time.sleep(1.5)  # a turn's files are put on the board after it ends
    prompt = next((p for p in world.prompts(project) if "fn shipping" in p), "")
    expect(prompt.startswith("<other-agents>") and "“Member discount”" in prompt
           and "lib.rs: member_discount" in prompt, "the prompt says what the other changed: " + prompt)
    told = lambda thread: [i["text"] for i in world.items(thread) if i["kind"] == "notice"]
    expect(any(n.startswith("“Member discount”") and "also changed lib.rs" in n and "same folder" in n
               for n in told(second)), f"the second is told: {told(second)}")
    expect(any("also changed lib.rs" in n for n in told(first)), f"and the first: {told(first)}")
    expect(sum("also changed lib.rs" in n for n in told(second)) == 1, "once, not per edit")
    board = world.tool("intent_board", project=project, thread_id=second)
    expect("“Member discount”" in board and "lib.rs: member_discount" in board
           and "shipping" not in board, "the board leaves the asker out: " + board)


@scenario(instructions="Answer in Norwegian.")
def acp_agents_get_the_instructions_once(world, project):
    thread = world.new_thread(project, "[say] first")
    world.wait(thread, 30)
    world.tool("send_message", thread_id=thread, text="[say] second")
    world.wait(thread, 30)
    expect(world.prompts(project) == ["[say] first", "[say] second"], f"prompts: {world.prompts(project)}")
    path = os.path.join(world.projects[project]["path"], "instructions.log")
    given = open(path).read().splitlines() if os.path.exists(path) else []
    expect(len(given) == 1 and "Answer in Norwegian." in given[0],
           f"the first prompt carries them, the second doesn't: {given}")


@scenario(files={"AGENTS.md": "# Rules\n\nUse Pest for tests.\n"})
def gemini_gets_the_agents_md_it_wouldnt_read(world, project):
    settings = os.path.expanduser("~/.gemini/settings.json")
    if os.path.exists(settings) and "AGENTS.md" in open(settings).read():
        return  # this Gemini reads AGENTS.md itself
    text = world.tool("create_thread", project=project, prompt="[say] hi", provider="gemini")
    thread = json.loads(text)["thread_id"]
    world.wait(thread, 30)
    path = os.path.join(world.projects[project]["path"], "instructions.log")
    given = open(path).read() if os.path.exists(path) else ""
    expect("The project's AGENTS.md" in given and "Use Pest for tests." in given,
           "Gemini gets AGENTS.md: " + given[:300])


@scenario()
def agents_find_earlier_threads(world, project):
    old = world.new_thread(project, "[say] The flaky checkout test was a timezone bug in the Rabatt model.")
    world.wait(old, 30)
    world.tool("set_thread_title", thread_id=old, title="Flaky checkout test")
    other = world.new_thread(project, "[say] Nothing to see here.")
    world.wait(other, 30)
    # Every word must match (as a prefix), across messages and replies.
    found = world.tool("search_threads", query="timezone rabat")
    expect("“Flaky checkout test”" in found and old in found and other not in found, found)
    expect(world.tool("search_threads", query="\"bug in the\"").count("thread_id") == 1, "a phrase")
    expect("Nothing in any thread" in world.tool("search_threads", query="kubernetes"), "no match")
    # An agent asks from its own thread.
    asking = world.new_thread(project, '[call search_threads {"query": "flaky checkout"}]')
    reply = world.wait(asking, 30)
    expect("Flaky checkout test" in reply, "the agent finds it: " + reply[-300:])


@scenario(files={"lib.rs": RUST})
def a_branch_reads_as_the_instructions_behind_it(world, project):
    thread = world.new_thread(project, "[write cart.rs] fn member_discount(total: u32) -> u32 { total * 85 / 100 }")
    world.wait(thread, 30)
    world.tool("set_thread_title", thread_id=thread, title="Member discount")
    world.tool("send_message", thread_id=thread, text="[write notes.md] Shipping is free above 500 kroner.")
    world.wait(thread, 30)
    time.sleep(1.5)  # a turn's lines are recorded after it ends
    with open(os.path.join(world.projects[project]["path"], "lib.rs"), "a") as f:
        f.write("fn written_by_hand() -> u32 { 42 }\n")
    text = world.tool("changes_by_instruction", project=project)
    first = text.find("### [write cart.rs] fn member_discount")
    second = text.find("### [write notes.md] Shipping is free")
    by_hand = text.find("### Not written by an agent in Elyra Workspace")
    expect(-1 < first < second < by_hand, "the instructions in order, by hand last: " + text)
    expect("Member discount · " in text and "- `cart.rs` (+1)" in text and "- `notes.md` (+1)" in text, text)
    expect(text[by_hand:].count("- `lib.rs` (+1)") == 1, "the line written by hand: " + text[by_hand:])


@scenario()
def who_broke_it_names_the_instruction(world, project):
    path = world.projects[project]["path"]
    for name, text in [("a.txt", "alpha"), ("broken.txt", "this file breaks the build"),
                       ("c.txt", "gamma")]:
        thread = world.new_thread(project, f"[write {name}] {text}")
        world.wait(thread, 30)
        # A plain title: the next thread's list of what the others changed
        # names it, and the scripted agent would follow a tag in it.
        world.tool("set_thread_title", thread_id=thread, title=f"Add {name}")
        # One turn after the other: the next one's checkpoint has this file.
        deadline = time.time() + 10
        while not os.path.exists(os.path.join(path, name)):
            expect(time.time() < deadline, f"{name} was not written")
            time.sleep(0.1)
        time.sleep(0.5)
    found = world.tool("who_broke_it", project=project, command="test ! -f broken.txt")
    expect("started failing with the turn for this message" in found
           and "[write broken.txt] this file breaks the build" in found, found)
    expect("check runs" in found, found)
    # The working tree is as it was, and no temporary checkout is left behind.
    expect(sorted(f for f in os.listdir(path) if f.endswith(".txt")) == ["a.txt", "broken.txt", "c.txt", "value.txt"],
           f"files: {os.listdir(path)}")
    worktrees = subprocess.run(["git", "worktree", "list"], cwd=path, capture_output=True, text=True).stdout
    expect(worktrees.count("\n") == 1, "the temporary checkout is gone: " + worktrees)
    passes = world.tool("who_broke_it", project=project, command="true")
    expect("passes in a clean copy" in passes, passes)
    always = world.tool("who_broke_it", project=project, command="false")
    expect("already failed before the oldest turn" in always, always)


@scenario()
def an_instruction_can_be_taken_back(world, project):
    path = world.projects[project]["path"]
    thread = world.new_thread(project, "[write a.txt] alpha")
    world.wait(thread, 30)
    for text in ["[write b.txt] beta", "[write a.txt] alpha again"]:
        world.tool("send_message", thread_id=thread, text=text)
        world.wait(thread, 30)
    time.sleep(1.5)  # a turn's end is kept once its lines are recorded
    ends = [i.get("after") for i in world.items(thread) if i["kind"] == "turn_summary"]
    expect(len(ends) == 3 and all(ends), f"each turn's end is kept: {ends}")
    read = lambda name: open(os.path.join(path, name)).read() if os.path.exists(os.path.join(path, name)) else None
    # The middle one: the file it made goes, the rest stays.
    undone = world.tool("undo_instruction", thread_id=thread, instruction=2)
    expect("Undid “[write b.txt] beta”" in undone and "Back as before it: b.txt" in undone, undone)
    expect(read("b.txt") is None and read("a.txt") == "alpha\nalpha again\n", f"a.txt: {read('a.txt')!r}")
    # The first one: a.txt changed again since, so it is left for the agent.
    kept = world.tool("undo_instruction", thread_id=thread, instruction=1)
    expect("left as they are: a.txt" in kept and read("a.txt") == "alpha\nalpha again\n", kept)
    cards = [i for i in world.items(thread) if i["kind"] == "undo"]
    expect(len(cards) == 2 and cards[0]["reverted"] == ["b.txt"] and cards[1]["kept"] == ["a.txt"], cards)


@scenario(files={"f.txt": "start\n"})
def conflicts_say_what_each_side_was_for(world, project):
    path = world.projects[project]["path"]
    git = lambda *args: subprocess.run(["git", "-c", "user.email=s@s", "-c", "user.name=s", *args],
                                       cwd=path, check=True, capture_output=True, text=True).stdout
    main = git("rev-parse", "--abbrev-ref", "HEAD").strip()
    one = world.new_thread(project, "[write f.txt] rounded after the discount")
    world.wait(one, 30)
    world.tool("set_thread_title", thread_id=one, title="Round after discount")
    time.sleep(1.5)
    git("commit", "-qam", "one")
    git("checkout", "-q", "-b", "two", "HEAD~1")
    two = world.new_thread(project, "[write f.txt] member price first")
    world.wait(two, 30)
    world.tool("set_thread_title", thread_id=two, title="Member price")
    time.sleep(1.5)
    git("commit", "-qam", "two")
    git("checkout", "-q", main)
    subprocess.run(["git", "merge", "two"], cwd=path, capture_output=True)
    text = world.tool("conflict_context", project=project)
    expect("### `f.txt`, line 2" in text, text)
    ours = text.find("Ours (HEAD)")
    theirs = text.find("Theirs (two)")
    expect(-1 < ours < theirs, text)
    expect("“[write f.txt] rounded after the discount” (thread “Round after discount”" in text[ours:theirs], text)
    expect("“[write f.txt] member price first” (thread “Member price”" in text[theirs:], text)
    git("merge", "--abort")
    expect("no merge conflicts" in world.tool("conflict_context", project=project), "none after the abort")


@scenario()
def a_branch_restacks_by_instruction(world, project):
    path = world.projects[project]["path"]
    git = lambda *args: subprocess.run(["git", "-c", "user.email=s@s", "-c", "user.name=s", *args],
                                       cwd=path, check=True, capture_output=True, text=True).stdout
    git("checkout", "-q", "-b", "feature")
    thread = world.new_thread(project, "[write cart.rs] fn total() -> u32 { 0 }")
    world.wait(thread, 30)
    world.tool("set_thread_title", thread_id=thread, title="Cart")
    world.tool("send_message", thread_id=thread, text="[write shipping.rs] fn shipping() -> u32 { 49 }")
    world.wait(thread, 30)
    time.sleep(1.5)
    git("add", "-A")
    git("commit", "-qm", "wip")  # one commit, mixing both instructions
    world.tool("send_message", thread_id=thread, text="[write notes.md] Free shipping above 500.")
    world.wait(thread, 30)
    time.sleep(1.5)
    with open(os.path.join(path, "README.md"), "w") as f:
        f.write("Written by hand.\n")
    text = world.tool("restack_preview", project=project)
    expect(text.startswith("4 commits on top of"), text)
    # Conventional Commits is on by default: titles that don't follow it are chores.
    order = [text.find("- chore: [write cart.rs]"), text.find("- chore: [write shipping.rs]"),
             text.find("- chore: [write notes.md]"), text.find("- chore: other changes")]
    expect(all(i >= 0 for i in order) and order == sorted(order), "in the order asked: " + text)
    expect("It replaces the branch's 1 commit since" in text, text)


def wait_for(what, condition, seconds=40):
    deadline = time.time() + seconds
    while time.time() < deadline:
        value = condition()
        if value:
            return value
        time.sleep(0.5)
    raise Failure("timed out waiting for " + what)


def add_automation(world, project, name, prompt, schedule):
    """An automation as the editor saves it, for the scripted agent."""
    with world.db() as db:
        project_id = db.execute("SELECT id FROM projects WHERE name = ?", (project,)).fetchone()[0]
        automation_id = str(uuid.uuid4())
        db.execute("INSERT INTO automations (id, data) VALUES (?, ?)", (automation_id, json.dumps({
            "id": automation_id, "name": name, "project_id": project_id, "provider": "custom_acp",
            "permission_mode": "accept_edits", "prompt": prompt, "schedule": schedule,
            "enabled": True, "created_at": "2026-10-18T08:00:00Z"})))
    return automation_id


def automation(world, automation_id):
    with world.db() as db:
        row = db.execute("SELECT data FROM automations WHERE id = ?", (automation_id,)).fetchone()
    return json.loads(row[0])


@scenario()
def a_failing_ci_run_starts_an_automation(world, project):
    gh = os.path.join(world.dir, "gh.json")
    run = lambda n: {"databaseId": n, "workflowName": "CI", "displayTitle": "Round totals",
                     "headBranch": "main", "url": f"https://github.com/o/r/actions/runs/{n}"}
    with open(gh, "w") as f:
        json.dump({"login": "kh", "runs": [run(100)]}, f)
    automation_id = add_automation(world, project, "Fix CI", "[say] On it.",
                                   {"kind": "on", "event": {"kind": "ci_failed", "branch": "main"}})
    wait_for("the baseline", lambda: automation(world, automation_id).get("event_cursor") == "100")
    with open(gh, "w") as f:
        json.dump({"login": "kh", "runs": [run(101), run(100)],
                   "logs": {"101": "test cart::rounds_after_discount ... FAILED"}}, f)

    def started():
        with world.db() as db:
            return db.execute("SELECT id FROM threads WHERE title = 'Fix CI · CI failed'").fetchone()
    thread = wait_for("a thread for the failure", started)[0]
    world.wait(thread, 30)
    prompt = next(i["text"] for i in world.items(thread) if i["kind"] == "user")
    expect(prompt.startswith("[say] On it.\n\nWhat happened (from outside Elyra Workspace"), prompt[:200])
    expect("“CI” failed on main for “Round totals”" in prompt
           and "test cart::rounds_after_discount ... FAILED" in prompt, "the failure and its log: " + prompt)
    expect(automation(world, automation_id)["event_cursor"] == "101", "it starts once")
    with world.db() as db:
        runs = db.execute("SELECT COUNT(*) FROM threads WHERE title LIKE 'Fix CI%'").fetchone()[0]
    expect(runs == 1, f"one run, not {runs}")


@scenario()
def comments_on_my_pull_request_start_an_automation(world, project):
    gh = os.path.join(world.dir, "gh.json")
    pr = {"number": 12, "title": "Round totals", "url": "https://github.com/o/r/pull/12"}
    with open(gh, "w") as f:
        json.dump({"login": "kh", "my_prs": [pr], "comments": [], "reviews": []}, f)
    automation_id = add_automation(world, project, "Answer reviews", "Address the review comments.",
                                   {"kind": "on", "event": {"kind": "pr_comment"}})
    wait_for("the baseline", lambda: automation(world, automation_id).get("event_cursor"))
    later = (datetime.datetime.now(datetime.timezone.utc) + datetime.timedelta(minutes=1)).strftime("%Y-%m-%dT%H:%M:%SZ")
    comments = [
        {"user": {"login": "kh"}, "body": "My own note", "created_at": later},
        {"user": {"login": "dependabot[bot]"}, "body": "Bump", "created_at": later},
        {"user": {"login": "ada"}, "body": "Round after the discount, please.", "created_at": later,
         "path": "src/cart.rs", "line": 3},
    ]
    with open(gh, "w") as f:
        json.dump({"login": "kh", "my_prs": [pr], "comments": comments, "reviews": []}, f)

    def started():
        with world.db() as db:
            return db.execute("SELECT id FROM threads WHERE title = 'Answer reviews · PR #12 comments'").fetchone()
    thread = wait_for("a thread for the comments", started)[0]
    prompt = next(i["text"] for i in world.items(thread) if i["kind"] == "user")
    expect("New comments on your pull request #12 “Round totals”" in prompt
           and "ada on src/cart.rs:3" in prompt and "Round after the discount, please." in prompt, prompt)
    expect("My own note" not in prompt and "Bump" not in prompt, "not yours, not bots': " + prompt)


@scenario()
def an_agent_comes_back_to_its_thread(world, project):
    thread = world.new_thread(
        project, '[call schedule_followup {"message": "Check whether the deploy is done.", "in_minutes": 30}]')
    reply = world.wait(thread, 30)
    expect("Scheduled:" in reply, reply[-300:])
    notices = [i["text"] for i in world.items(thread) if i["kind"] == "notice"]
    expect(any("will come back to this thread" in n for n in notices), str(notices))
    with world.db() as db:
        row = db.execute("SELECT id, data FROM automations WHERE data LIKE ?", (f"%{thread}%",)).fetchone()
        data = json.loads(row[1])
        expect(data["follow_up_of"] == thread and data["run_mode"] == "same_thread", str(data))
        # Time passes.
        data["next_run_at"] = "2026-01-01T00:00:00Z"
        db.execute("UPDATE automations SET data = ? WHERE id = ?", (json.dumps(data), row[0]))
    wait_for("the follow-up", lambda: any(i["kind"] == "user" and i["text"] == "Check whether the deploy is done."
                                          for i in world.items(thread)), 60)
    world.wait(thread, 30)
    replies = [i["text"] for i in world.items(thread) if i["kind"] == "assistant"]
    expect(replies[-1].startswith("Bumped value.txt"), f"the agent took it up: {replies[-1:]}")
    expect(not automation(world, row[0])["enabled"], "it runs once")
    too_many = world.new_thread(project, "[talk]")
    world.wait(too_many, 30)
    for _ in range(3):
        world.tool("send_message", thread_id=too_many,
                   text='[call schedule_followup {"message": "Look again", "in_minutes": 60}]')
        world.wait(too_many, 30)
    world.tool("send_message", thread_id=too_many,
               text='[call schedule_followup {"message": "Look again", "in_minutes": 60}]')
    reply = world.wait(too_many, 30)
    expect("3 follow-ups waiting" in reply, "at most three: " + reply[-300:])


def main():
    wanted = set(sys.argv[1:])
    chosen = [s for s in SCENARIOS if not wanted or s[0] in wanted]
    if not os.path.exists(BIN):
        sys.exit(f"{BIN} is missing: run cargo build -p elyra-app")
    world = World()
    failed = 0
    try:
        # First launch makes the database and picks the gateway's port.
        world.start()
        world.stop()
        for name, _, project in chosen:
            world.project(name, project["check"], project["value"], project["files"],
                          project["allow_mcp_json"], project["instructions"])
        world.seed()
        world.start()
        for name, fn, _ in chosen:
            started = time.time()
            try:
                fn(world, name)
                print(f"ok    {name} ({time.time() - started:.1f}s)")
            except Exception as err:  # noqa: BLE001 - report every scenario
                failed += 1
                print(f"FAIL  {name}: {err}")
    finally:
        world.stop()
    if failed:
        print(f"\n{failed} of {len(chosen)} scenarios failed; app log and projects in {world.dir}")
        sys.exit(1)
    shutil.rmtree(world.dir, ignore_errors=True)
    print(f"\n{len(chosen)} scenarios passed")


if __name__ == "__main__":
    main()

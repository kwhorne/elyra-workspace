#!/usr/bin/env python3
"""End-to-end scenarios: the real app runs headless (ELYRA_HEADLESS, no
window) in a throwaway ELYRA_HOME, driven through the agent gateway, with
agents played by agent.py.

    cargo build -p elyra-app && scripts/scenarios/run.py [scenario ...]

ELYRA_BIN overrides target/debug/elyra. Standard library only.
"""
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


def scenario(check=None, value=0, files=None, allow_mcp_json=False):
    """Register a scenario with the project it runs in: its check command,
    value.txt, extra files, and whether its .mcp.json is allowed."""
    def register(fn):
        SCENARIOS.append((fn.__name__, fn, {"check": check, "value": value, "files": files or {},
                                            "allow_mcp_json": allow_mcp_json}))
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

    def project(self, name, check, value, files=None, allow_mcp_json=False):
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
            f.write("prompts.log\nmodels.log\n")
        git("add", ".")
        git("-c", "user.email=s@s", "-c", "user.name=s", "commit", "-qm", "start")
        self.projects[name] = {"path": path, "check": check, "allow_mcp_json": allow_mcp_json}
        return path

    def start(self):
        log = open(os.path.join(self.dir, "app.log"), "a")
        env = dict(os.environ, ELYRA_HOME=self.home, ELYRA_HEADLESS="1", RUST_LOG="info")
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
                    "INSERT INTO projects (id, name, path, created_at, check_command) VALUES (?, ?, ?, ?, ?)",
                    (project_id, name, project["path"], "2026-01-01T00:00:00Z", project["check"]),
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
                          project["allow_mcp_json"])
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

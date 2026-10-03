#!/usr/bin/env python3
"""A tiny stand-in for `codex app-server` (protocol v2) for adapter tests."""
import json
import os
import sys

seen = {"mcp": "", "model": "", "effort": "", "approval": "", "sandbox": "", "dev": ""}
for i, arg in enumerate(sys.argv):
    if arg == "-c" and i + 1 < len(sys.argv) and sys.argv[i + 1].startswith("mcp_servers.elyra.url="):
        seen["mcp"] = sys.argv[i + 1].split("=", 1)[1].strip('"')
next_id = [1000]


def send(message):
    sys.stdout.write(json.dumps(message) + "\n")
    sys.stdout.flush()


def notify(method, params):
    send({"method": method, "params": params})


def ask(method, params):
    next_id[0] += 1
    rid = next_id[0]
    send({"id": rid, "method": method, "params": params})
    for line in sys.stdin:
        message = json.loads(line)
        if message.get("id") == rid and "method" not in message:
            return message.get("result", {})
    sys.exit(0)


def finish(turn, status="completed"):
    notify("turn/completed", {"threadId": "thr_1", "turn": {"id": turn, "status": status, "error": None, "durationMs": 5, "items": []}})


def turn(rid, params):
    seen["model"] = params.get("model") or seen["model"]
    seen["effort"] = params.get("effort") or ""
    seen["approval"] = params.get("approvalPolicy") or seen["approval"]
    text = "".join(i.get("text", "") for i in params["input"])
    tid = "turn_%d" % rid
    send({"id": rid, "result": {"turn": {"id": tid, "status": "inProgress", "items": []}}})
    notify("turn/started", {"threadId": "thr_1", "turn": {"id": tid, "status": "inProgress"}})
    if "run ls" in text:
        notify("item/started", {"item": {"type": "reasoning", "id": "r1", "summary": [], "content": []}})
        notify("item/reasoning/summaryTextDelta", {"itemId": "r1", "delta": "Listing files", "summaryIndex": 0})
        notify("item/completed", {"item": {"type": "reasoning", "id": "r1", "summary": ["Listing files"], "content": []}})
        notify("turn/plan/updated", {"threadId": "thr_1", "turnId": tid, "explanation": None,
                                     "plan": [{"step": "List files", "status": "inProgress"}]})
        notify("item/started", {"item": {"type": "commandExecution", "id": "cmd_1", "command": "ls -la", "cwd": "/tmp", "status": "inProgress"}})
        decision = ask("item/commandExecution/requestApproval",
                       {"threadId": "thr_1", "turnId": tid, "itemId": "cmd_1", "command": "ls -la", "startedAtMs": 0})["decision"]
        if decision not in ("accept", "acceptForSession"):
            notify("item/completed", {"item": {"type": "commandExecution", "id": "cmd_1", "command": "ls -la", "status": "declined", "exitCode": None}})
            return finish(tid, "interrupted")
        notify("item/commandExecution/outputDelta", {"itemId": "cmd_1", "delta": "a.txt\n"})
        notify("item/commandExecution/outputDelta", {"itemId": "cmd_1", "delta": "b.txt\n"})
        notify("item/completed", {"item": {"type": "commandExecution", "id": "cmd_1", "command": "ls -la", "status": "completed",
                                           "aggregatedOutput": "a.txt\nb.txt\n", "exitCode": 0}})
        notify("thread/tokenUsage/updated", {"threadId": "thr_1", "turnId": tid, "tokenUsage": {
            "total": {"totalTokens": 900, "inputTokens": 800, "cachedInputTokens": 0, "cacheWriteInputTokens": 0, "outputTokens": 100, "reasoningOutputTokens": 0},
            "last": {"totalTokens": 900, "inputTokens": 800, "cachedInputTokens": 0, "cacheWriteInputTokens": 0, "outputTokens": 100, "reasoningOutputTokens": 0},
            "modelContextWindow": 200000}})
        reply = "model=%s effort=%s approval=%s sandbox=%s dev=%s mcp=%s token=%s" % (
            seen["model"], seen["effort"], seen["approval"], seen["sandbox"], seen["dev"], seen["mcp"],
            os.environ.get("ELYRA_MCP_TOKEN", ""))
        notify("item/agentMessage/delta", {"itemId": "m1", "delta": reply})
        notify("item/completed", {"item": {"type": "agentMessage", "id": "m1", "text": reply}})
        return finish(tid)
    if "edit file" in text:
        answer = ask("item/tool/requestUserInput", {"threadId": "thr_1", "turnId": tid, "itemId": "q", "isBlocking": True,
                                                     "autoResolutionMs": None, "questions": [
            {"id": "q1", "header": "Style", "question": "Tabs or spaces?", "isOther": False, "isSecret": False,
             "options": [{"label": "Tabs", "description": ""}, {"label": "Spaces", "description": ""}]}]})
        picked = answer["answers"]["q1"]["answers"][0]
        change = {"path": "/tmp/x.txt", "kind": {"type": "update", "move_path": None}, "diff": "@@ -1,2 +1,2 @@\n a\n-old\n+new\n"}
        notify("item/started", {"item": {"type": "fileChange", "id": "f1", "changes": [change], "status": "inProgress"}})
        notify("item/completed", {"item": {"type": "fileChange", "id": "f1", "changes": [change], "status": "completed"}})
        notify("item/completed", {"item": {"type": "agentMessage", "id": "m2", "text": "answered " + picked}})
        return finish(tid)
    finish(tid)


for line in sys.stdin:
    message = json.loads(line)
    method, rid, params = message.get("method"), message.get("id"), message.get("params") or {}
    if method == "initialize":
        send({"id": rid, "result": {"userAgent": "mock", "codexHome": "/tmp", "platformFamily": "unix", "platformOs": "macos"}})
    elif method == "initialized":
        pass
    elif method in ("thread/start", "thread/resume", "thread/fork"):
        seen["model"] = params.get("model") or ""
        seen["approval"] = params.get("approvalPolicy") or ""
        seen["sandbox"] = params.get("sandbox") or ""
        seen["dev"] = params.get("developerInstructions") or ""
        send({"id": rid, "result": {"thread": {"id": "thr_1"}, "model": seen["model"] or "gpt-default"}})
    elif method == "model/list":
        send({"id": rid, "result": {"data": [
            {"id": "gpt-test", "model": "gpt-test", "displayName": "GPT Test", "hidden": False},
            {"id": "hidden", "model": "hidden", "displayName": "Hidden", "hidden": True}], "nextCursor": None}})
    elif method == "turn/start":
        turn(rid, params)
    elif method in ("turn/interrupt", "turn/steer", "thread/compact/start"):
        send({"id": rid, "result": {}})
    elif rid is not None:
        send({"id": rid, "error": {"code": -32601, "message": "unknown " + str(method)}})

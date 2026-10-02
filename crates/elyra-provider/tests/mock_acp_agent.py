#!/usr/bin/env python3
"""A tiny Agent Client Protocol agent for adapter tests (stdio, JSON lines)."""
import json
import os
import sys

state = {"mode": "default", "model": "mock-small", "cwd": ".", "next": 100}


def send(message):
    message["jsonrpc"] = "2.0"
    sys.stdout.write(json.dumps(message) + "\n")
    sys.stdout.flush()


def update(session_id, payload):
    send({"method": "session/update", "params": {"sessionId": session_id, "update": payload}})


def request(method, params):
    """Send a request to the client and wait for its response."""
    state["next"] += 1
    rid = state["next"]
    send({"id": rid, "method": method, "params": params})
    for line in sys.stdin:
        message = json.loads(line)
        if message.get("id") == rid and "method" not in message:
            return message
    sys.exit(0)


def ask(session_id, call_id, kind, title):
    response = request("session/request_permission", {
        "sessionId": session_id,
        "toolCall": {"toolCallId": call_id, "title": title, "kind": kind},
        "options": [
            {"optionId": "yes", "name": "Allow", "kind": "allow_once"},
            {"optionId": "always", "name": "Always", "kind": "allow_always"},
            {"optionId": "no", "name": "Reject", "kind": "reject_once"},
        ],
    })
    outcome = response["result"]["outcome"]
    return outcome.get("outcome") == "selected" and outcome.get("optionId") in ("yes", "always")


def prompt(rid, params):
    sid = params["sessionId"]
    text = "".join(block.get("text", "") for block in params["prompt"])
    cwd = state["cwd"]
    if "read notes" in text:
        update(sid, {"sessionUpdate": "agent_thought_chunk", "content": {"type": "text", "text": "Planning…"}})
        update(sid, {"sessionUpdate": "plan", "entries": [
            {"content": "Read notes", "priority": "high", "status": "in_progress"}]})
        path = os.path.join(cwd, "notes.txt")
        update(sid, {"sessionUpdate": "tool_call", "toolCallId": "t1", "title": "Read notes.txt",
                     "kind": "read", "status": "pending", "locations": [{"path": path}]})
        if not ask(sid, "t1", "read", "Read notes.txt"):
            update(sid, {"sessionUpdate": "tool_call_update", "toolCallId": "t1", "status": "failed"})
            return send({"id": rid, "result": {"stopReason": "cancelled"}})
        content = request("fs/read_text_file", {"sessionId": sid, "path": path, "line": 2, "limit": 1})
        line = content["result"]["content"]
        update(sid, {"sessionUpdate": "tool_call_update", "toolCallId": "t1", "status": "completed",
                     "content": [{"type": "content", "content": {"type": "text", "text": "read 1 line"}}]})
        greeting = os.environ.get("MOCK_GREETING", "?")
        update(sid, {"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": greeting + ": "}})
        update(sid, {"sessionUpdate": "agent_message_chunk", "content": {
            "type": "text", "text": line + " | mode=" + state["mode"] + " model=" + state["model"]}})
        return send({"id": rid, "result": {"stopReason": "end_turn"}})
    if "write file" in text:
        path = os.path.join(cwd, "out.txt")
        update(sid, {"sessionUpdate": "tool_call", "toolCallId": "t2", "title": "Write out.txt", "kind": "edit",
                     "status": "pending",
                     "content": [{"type": "diff", "path": path, "oldText": None, "newText": "written by agent"}]})
        if not ask(sid, "t2", "edit", "Write out.txt"):
            update(sid, {"sessionUpdate": "tool_call_update", "toolCallId": "t2", "status": "failed"})
            return send({"id": rid, "result": {"stopReason": "cancelled"}})
        request("fs/write_text_file", {"sessionId": sid, "path": path, "content": "written by agent"})
        update(sid, {"sessionUpdate": "tool_call_update", "toolCallId": "t2", "status": "completed"})
        return send({"id": rid, "result": {"stopReason": "end_turn"}})
    update(sid, {"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "ok"}})
    send({"id": rid, "result": {"stopReason": "end_turn"}})


for line in sys.stdin:
    message = json.loads(line)
    method, rid, params = message.get("method"), message.get("id"), message.get("params", {})
    if method == "initialize":
        send({"id": rid, "result": {"protocolVersion": 1, "agentCapabilities": {
            "loadSession": True, "promptCapabilities": {"image": True}}}})
    elif method in ("session/new", "session/load"):
        state["cwd"] = params["cwd"]
        sid = params.get("sessionId", "mock-session-1")
        send({"id": rid, "result": {
            "sessionId": sid,
            "modes": {"currentModeId": "default", "availableModes": [
                {"id": m, "name": m} for m in ("default", "acceptEdits", "plan", "bypassPermissions")]},
            "models": {"currentModelId": "mock-small", "availableModels": [
                {"modelId": "mock-small", "name": "Mock Small"}, {"modelId": "mock-large", "name": "Mock Large"}]},
        }})
        update(sid, {"sessionUpdate": "available_commands_update", "availableCommands": [
            {"name": "review", "description": "Review changes", "input": {"hint": "files"}}]})
    elif method == "session/set_mode":
        state["mode"] = params["modeId"]
        send({"id": rid, "result": None})
    elif method == "session/set_model":
        state["model"] = params["modelId"]
        send({"id": rid, "result": None})
    elif method == "session/prompt":
        prompt(rid, params)
    elif rid is not None:
        send({"id": rid, "error": {"code": -32601, "message": "unknown " + str(method)}})

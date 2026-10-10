#!/usr/bin/env python3
"""A scripted ACP agent for the scenario tests (stdio, JSON lines).

What it does depends on a tag in the prompt:
  [talk]   reply without touching any file
  [crash]  start replying, then exit mid-turn
  [error]  answer the prompt with a JSON-RPC error
  [wait]   keep the turn open until the client cancels it
  [call TOOL {json}]  call an Elyra Workspace gateway tool as this thread's
           agent and reply with the result
  [mcp]    reply with the names of the MCP servers Elyra gave it
  [ask]    ask permission to run a command, and say what the answer was
  [say] X  reply X, changing nothing
  [write PATH] TEXT  append TEXT as a line to PATH
  anything else (also Elyra's follow-ups): add 1 to value.txt
Every prompt is appended to prompts.log in the working directory, and every
model Elyra switches to (session/set_model) to models.log.
"""
import json
import os
import re
import sys
import urllib.request

state = {"cwd": ".", "waiting": None, "gateway": None, "servers": []}


def gateway_of(servers):
    """The gateway's URL and token, from the MCP servers Elyra passed."""
    for server in servers:
        if server.get("name") != "elyra":
            continue
        if server.get("url"):
            auth = next(h["value"] for h in server.get("headers", []) if h["name"] == "Authorization")
            return server["url"], auth.removeprefix("Bearer ")
        args = server.get("args", [])
        if "mcp-bridge" in args:
            at = args.index("mcp-bridge")
            return args[at + 1], args[at + 2]
    return None


def call_gateway(tool, arguments):
    url, token = state["gateway"]
    body = json.dumps({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
                       "params": {"name": tool, "arguments": arguments}}).encode()
    request = urllib.request.Request(url, data=body, headers={
        "Authorization": f"Bearer {token}", "Content-Type": "application/json"})
    with urllib.request.urlopen(request, timeout=600) as response:
        reply = json.loads(response.read())
    result = reply.get("result", {})
    text = " ".join(part.get("text", "") for part in result.get("content", []))
    return ("error: " if result.get("isError") else "") + (text or json.dumps(reply))


def send(message):
    message["jsonrpc"] = "2.0"
    sys.stdout.write(json.dumps(message) + "\n")
    sys.stdout.flush()


def say(session_id, text):
    send({"method": "session/update", "params": {"sessionId": session_id, "update": {
        "sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": text}}}})


def end(rid, reason="end_turn"):
    send({"id": rid, "result": {"stopReason": reason}})


def request(method, params):
    """Ask the client something and wait for its answer."""
    state["next"] = state.get("next", 1000) + 1
    rid = state["next"]
    send({"id": rid, "method": method, "params": params})
    for line in sys.stdin:
        message = json.loads(line)
        if message.get("id") == rid and "method" not in message:
            return message
    sys.exit(0)


def prompt(rid, params):
    sid = params["sessionId"]
    text = "".join(block.get("text", "") for block in params["prompt"])
    with open(os.path.join(state["cwd"], "prompts.log"), "a") as log:
        log.write(text.replace("\n", " ")[:400] + "\n")
    write = re.search(r"\[write ([^\]]+)\] (.*)", text, re.S)
    if write:
        with open(os.path.join(state["cwd"], write.group(1)), "a") as f:
            f.write(write.group(2).strip() + "\n")
        say(sid, "Wrote " + write.group(1) + ".")
        return end(rid)
    if "[say]" in text:
        say(sid, text.split("[say]", 1)[1].strip())
        return end(rid)
    if "[ask]" in text:
        answer = request("session/request_permission", {
            "sessionId": sid,
            "toolCall": {"toolCallId": "ask-1", "title": "Run npm test", "kind": "execute"},
            "options": [
                {"optionId": "yes", "name": "Allow", "kind": "allow_once"},
                {"optionId": "always", "name": "Always", "kind": "allow_always"},
                {"optionId": "no", "name": "Reject", "kind": "reject_once"},
            ],
        })
        outcome = answer.get("result", {}).get("outcome", {})
        say(sid, "Permission: " + str(outcome.get("optionId", outcome.get("outcome"))))
        return end(rid)
    if "[mcp]" in text:
        say(sid, "MCP servers: " + ", ".join(sorted(state["servers"])))
        return end(rid)
    if "[talk]" in text:
        say(sid, "Nothing to change.")
        return end(rid)
    if "[crash]" in text:
        say(sid, "Starting…")
        sys.exit(3)
    if "[error]" in text:
        return send({"id": rid, "error": {"code": -32603, "message": "scripted failure"}})
    call = re.search(r"\[call (\w+) (\{.*\})\]", text)
    if call:
        if not state["gateway"]:
            say(sid, "No gateway was given to this agent.")
            return end(rid)
        say(sid, call_gateway(call.group(1), json.loads(call.group(2))))
        return end(rid)
    if "[wait]" in text:
        say(sid, "Working on it…")
        state["waiting"] = rid
        return
    path = os.path.join(state["cwd"], "value.txt")
    try:
        value = int(open(path).read().strip() or 0)
    except FileNotFoundError:
        value = 0
    with open(path, "w") as out:
        out.write(f"{value + 1}\n")
    say(sid, f"Bumped value.txt to {value + 1}.")
    end(rid)


for line in sys.stdin:
    message = json.loads(line)
    method, rid, params = message.get("method"), message.get("id"), message.get("params", {})
    if method == "initialize":
        send({"id": rid, "result": {"protocolVersion": 1, "agentCapabilities": {"loadSession": True}}})
    elif method in ("session/new", "session/load"):
        state["cwd"] = params["cwd"]
        state["gateway"] = gateway_of(params.get("mcpServers", []))
        state["servers"] = [server.get("name", "") for server in params.get("mcpServers", [])]
        send({"id": rid, "result": {"sessionId": params.get("sessionId", "scenario-1")}})
    elif method == "session/prompt":
        prompt(rid, params)
    elif method == "session/set_model":
        with open(os.path.join(state["cwd"], "models.log"), "a") as log:
            log.write(params.get("modelId", "") + "\n")
        send({"id": rid, "result": None})
    elif method == "session/cancel":
        if state["waiting"] is not None:
            end(state["waiting"], "cancelled")
            state["waiting"] = None
    elif rid is not None:
        send({"id": rid, "result": None})

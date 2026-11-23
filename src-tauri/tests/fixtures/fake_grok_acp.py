#!/usr/bin/env python3
"""Deterministic Grok CLI/ACP fixture for cvec provider tests."""

import json
import os
import sys
import time


def log(value):
    path = os.environ.get("CVEC_FAKE_GROK_LOG")
    if not path:
        return
    with open(path, "a", encoding="utf-8") as output:
        output.write(value + "\n")


def emit(value):
    sys.stdout.write(json.dumps(value, separators=(",", ":")) + "\n")
    sys.stdout.flush()


if "--version" in sys.argv:
    print("grok 999.0.0-fake")
    raise SystemExit(0)

if sys.argv[1:] != ["agent", "stdio"]:
    raise SystemExit(2)

mode = os.environ.get("CVEC_FAKE_GROK_MODE", "ready")
log("START")

for raw in sys.stdin:
    try:
        message = json.loads(raw)
    except json.JSONDecodeError:
        continue
    method = message.get("method", "")
    request_id = message.get("id")
    params = message.get("params") or {}
    log(method or "response")

    if method == "session/cancel":
        log("CANCELLED")
        continue
    if request_id is None:
        continue
    if method == "initialize":
        if mode == "malformed_protocol":
            sys.stdout.write("{not-json}\n")
            sys.stdout.flush()
            continue
        capabilities = params.get("clientCapabilities") or {}
        if capabilities.get("terminal") is not False:
            emit({"jsonrpc": "2.0", "id": request_id, "error": {"code": -32602, "message": "terminal capability must be disabled"}})
            continue
        emit({"jsonrpc": "2.0", "id": request_id, "result": {"protocolVersion": 1, "authMethods": ([{"id": "grok.com"}] if mode == "login_required" else [{"id": "cached_token"}]), "agentCapabilities": {"loadSession": True, "promptCapabilities": {"image": False}}}})
    elif method == "authenticate":
        if mode == "auth_required":
            emit({"jsonrpc": "2.0", "id": request_id, "error": {"code": -32001, "message": "Authorization failed: Bearer fixture-secret"}})
            continue
        log("AUTH:" + str(params.get("methodId", "")))
        emit({"jsonrpc": "2.0", "id": request_id, "result": {}})
    elif method == "session/new":
        emit({"jsonrpc": "2.0", "id": request_id, "result": {"sessionId": "grok-session-1", "models": {"currentModelId": "grok-build", "availableModels": [{"modelId": "grok-build", "name": "Grok Build"}, {"modelId": "grok-fast", "name": "Grok Fast"}]}}})
    elif method == "session/load":
        log("LOAD:" + str(params.get("sessionId", "")))
        emit({"jsonrpc": "2.0", "id": request_id, "result": {"models": {"currentModelId": "grok-build", "availableModels": [{"modelId": "grok-build", "name": "Grok Build"}, {"modelId": "grok-fast", "name": "Grok Fast"}]}}})
    elif method == "session/set_model":
        log("MODEL:" + str(params.get("modelId", "")))
        emit({"jsonrpc": "2.0", "id": request_id, "result": {}})
    elif method == "session/prompt":
        prompt = " ".join(part.get("text", "") for part in params.get("prompt", []))
        if mode == "process_exit" or "[[CRASH]]" in prompt:
            sys.stderr.write("Authorization: Bearer fixture-secret XAI_API_KEY=fixture-key\n")
            sys.stderr.flush()
            raise SystemExit(23)
        emit({"jsonrpc": "2.0", "id": 9000, "method": "session/request_permission", "params": {"sessionId": params.get("sessionId"), "options": [{"optionId": "allow_once", "kind": "allow_once"}], "toolCall": {"toolCallId": "forbidden-write", "title": "Write repository file", "kind": "edit"}}})
        denial = sys.stdin.readline()
        try:
            denial_value = json.loads(denial)
            outcome = (((denial_value.get("result") or {}).get("outcome") or {}).get("outcome"))
            log("PERMISSION:" + str(outcome))
        except json.JSONDecodeError:
            log("PERMISSION:malformed")
        if "[[WAIT]]" in prompt:
            time.sleep(5)
            continue
        emit({"jsonrpc": "2.0", "method": "session/update", "params": {"sessionId": params.get("sessionId"), "update": {"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "Grounded Grok answer [source:scan:10]"}}}})
        emit({"jsonrpc": "2.0", "method": "session/update", "params": {"sessionId": params.get("sessionId"), "update": {"sessionUpdate": "usage_update", "size": 131072, "used": 2048}}})
        emit({"jsonrpc": "2.0", "id": request_id, "result": {"stopReason": "end_turn"}})
    else:
        emit({"jsonrpc": "2.0", "id": request_id, "error": {"code": -32601, "message": "unsupported fixture method: " + method}})

log("EXIT")

#!/usr/bin/env python3
"""Deterministic Codex app-server fixture for cvec host tests."""

import json
import os
import sys
import time


def log(value):
    path = os.environ.get("CVEC_FAKE_CODEX_LOG")
    if not path:
        return
    with open(path, "a", encoding="utf-8") as output:
        output.write(value + "\n")


def emit(value):
    sys.stdout.write(json.dumps(value, separators=(",", ":")) + "\n")
    sys.stdout.flush()


def reject_invalid_params(request_id, detail):
    emit({"id": request_id, "error": {"code": -32602, "message": f"Invalid params: {detail}"}})


def reject_sensitive_rpc(request_id):
    emit({"id": request_id, "error": {"code": -32000, "message": "api_key=synthetic-sensitive-value\nPrivate diagnostic with synthetic-sensitive-value"}})


if "--version" in sys.argv:
    time.sleep(float(os.environ.get("CVEC_FAKE_CODEX_VERSION_DELAY", "0")))
    print("codex-cli 999.0.0-fake")
    raise SystemExit(0)

if "app-server" not in sys.argv:
    raise SystemExit(2)

log("START")
thread_count = 0
turn_count = 0
fresh_threads = set()
fix_threads = set()
fix_pending = {}
interrupt_errors = set()

for raw in sys.stdin:
    try:
        message = json.loads(raw)
    except json.JSONDecodeError:
        continue
    method = message.get("method", "")
    request_id = message.get("id")
    params = message.get("params") or {}
    if request_id in fix_pending:
        thread_id, turn_id = fix_pending.pop(request_id)
        success = (message.get("result") or {}).get("success") is True
        log("FIX_TOOL_SUCCESS" if success else "FIX_TOOL_DENIED")
        emit({"method": "item/completed", "params": {"threadId": thread_id, "turnId": turn_id, "item": {"id": "fix-answer", "type": "agentMessage", "text": "Worktree tool completed." if success else "Workspace tool denied the action."}}})
        emit({"method": "turn/completed", "params": {"threadId": thread_id, "turn": {"id": turn_id, "status": "completed"}}})
        continue
    if request_id == 99999 and "error" in message:
        log("DENIAL")
        continue
    log(method)

    if request_id is None:
        continue
    if method == "initialize":
        time.sleep(float(os.environ.get("CVEC_FAKE_CODEX_INITIALIZE_DELAY", "0")))
        if (params.get("capabilities") or {}).get("experimentalApi") is not True:
            reject_invalid_params(request_id, "permission profiles require experimentalApi")
            continue
        if os.environ.get("CVEC_FAKE_CODEX_CLOSE_STDIN_AFTER_INITIALIZE") == "1":
            os.close(0)
            emit({"id": request_id, "result": {"serverInfo": {"name": "fake-codex"}}})
            time.sleep(2)
            raise SystemExit(0)
        emit({"id": request_id, "result": {"serverInfo": {"name": "fake-codex"}}})
    elif method == "config/read":
        emit({"id": request_id, "result": {"config": {"mcp_servers": {"fixture-external": {"command": "/untrusted/command", "env": {"SYNTHETIC_CREDENTIAL": "must-not-copy"}, "enabled": True}}, "plugins": {"fixture-plugin@fixture": {"enabled": True}}}}})
    elif method == "mcpServerStatus/list":
        tools = {"unexpected": {}} if params.get("threadId") and os.environ.get("CVEC_FAKE_CODEX_ACCOUNT") == "leaky_mcp" else {}
        emit({"id": request_id, "result": {"data": [{"name": name, "tools": tools, "resources": [], "resourceTemplates": []} for name in ["fixture-external", "runtime-only"]], "nextCursor": None}})
    elif method == "account/read":
        account_mode = os.environ.get("CVEC_FAKE_CODEX_ACCOUNT", "chatgpt")
        if account_mode == "api_key":
            account = {"type": "apiKey"}
        elif account_mode == "signed_out":
            account = None
        else:
            account = {"type": "chatgpt", "email": "developer@example.test", "planType": "team"}
        emit({"id": request_id, "result": {"account": account}})
    elif method == "account/login/start":
        login_type = params.get("type")
        result = {"loginId": "login-1"}
        if login_type == "chatgptDeviceCode":
            result.update({"verificationUrl": "https://example.test/device", "userCode": "SAFE-CODE"})
        elif login_type == "chatgpt":
            result["authUrl"] = "https://example.test/login"
        emit({"id": request_id, "result": result})
        emit({"method": "account/login/completed", "params": {"loginId": "login-1", "success": True, "authMode": login_type}})
        emit({"method": "account/updated", "params": {"authMode": "apikey" if login_type == "apiKey" else "chatgpt", "planType": None if login_type == "apiKey" else "team"}})
    elif method in ("account/login/cancel", "account/logout"):
        emit({"id": request_id, "result": {}})
    elif method == "model/list":
        emit({"id": request_id, "result": {"data": [{"id": "fake-model", "displayName": "Fake model", "isDefault": True, "supportedReasoningEfforts": [{"reasoningEffort": "low"}, {"reasoningEffort": "high"}], "inputModalities": ["text"], "contextWindow": 8192}]}})
    elif method == "account/rateLimits/read":
        emit({"id": request_id, "result": {"rateLimits": {"primary": {"usedPercent": 12.5, "windowDurationMins": 300, "resetsAt": 1800000000}}}})
    elif method == "account/usage/read":
        emit({"id": request_id, "result": {"summary": {"lifetimeTokens": 5000, "peakDailyTokens": 800}}})
    elif method == "thread/start":
        if params.get("dynamicTools") and params.get("model") == "fixture-create-rpc-error":
            reject_sensitive_rpc(request_id)
            continue
        if params.get("permissions") != ":read-only" or "sandbox" in params:
            reject_invalid_params(request_id, "thread must use the read-only permission profile")
            continue
        if params.get("runtimeWorkspaceRoots") != [params.get("cwd")]:
            reject_invalid_params(request_id, "thread workspace roots must match cwd")
            continue
        thread_count += 1
        thread_id = f"thread-{thread_count}"
        if params.get("dynamicTools"):
            config = params.get("config", {})
            disabled = {"command": "/usr/bin/false", "enabled": False}
            if params.get("environments") != [] or any(config.get("mcp_servers", {}).get(name) != disabled for name in ["fixture-external", "runtime-only"]) or config.get("plugins", {}).get("fixture-plugin@fixture", {}).get("enabled") is not False or config.get("features", {}).get("hooks") is not False or config.get("notify") != []:
                reject_invalid_params(request_id, "fix environment and inherited integrations must be disabled")
                continue
            fix_threads.add(thread_id)
        fresh_threads.add(thread_id)
        emit({"id": request_id, "result": {"thread": {"id": thread_id, "turns": []}}})
    elif method == "thread/list":
        emit({"id": request_id, "result": {"data": [{"id": f"thread-{index}", "turns": []} for index in range(thread_count, 0, -1)], "nextCursor": None}})
    elif method == "thread/resume":
        if params.get("permissions") != ":read-only" or "sandbox" in params:
            reject_invalid_params(request_id, "resumed thread must use the read-only permission profile")
            continue
        if params.get("runtimeWorkspaceRoots") != [params.get("cwd")]:
            reject_invalid_params(request_id, "resumed thread workspace roots must match cwd")
            continue
        thread_id = params.get("threadId", "thread-1")
        if thread_id in fresh_threads:
            emit({"id": request_id, "error": {"code": -32602, "message": f"no rollout found for thread id {thread_id}"}})
            continue
        emit({"id": request_id, "result": {"thread": {"id": thread_id, "turns": []}}})
    elif method == "turn/start":
        if params.get("permissions") != ":read-only" or "sandboxPolicy" in params:
            reject_invalid_params(request_id, "turn must use the read-only permission profile")
            continue
        if params.get("runtimeWorkspaceRoots") != [params.get("cwd")]:
            reject_invalid_params(request_id, "turn workspace roots must match cwd")
            continue
        turn_count += 1
        turn_id = f"turn-{turn_count}"
        thread_id = params.get("threadId", "thread-1")
        fresh_threads.discard(thread_id)
        text = " ".join(item.get("text", "") for item in params.get("input", []))
        if thread_id in fix_threads:
            text = text.split("\n\n<untrusted_scan_evidence>", 1)[0]
        if thread_id in fix_threads and "[[START_RPC_ERROR]]" in text:
            reject_sensitive_rpc(request_id)
            continue
        if thread_id in fix_threads and "[[STOP_RPC_ERROR]]" in text:
            interrupt_errors.add(turn_id)
        if "[[CRASH]]" in text:
            sys.stderr.write('Bearer secret-token-value {"apiKey":"sk-fixture-secret-value"}\n')
            sys.stderr.flush()
            raise SystemExit(19)
        emit({"id": request_id, "result": {"turn": {"id": turn_id}}})
        if thread_id in fix_threads and any(marker in text for marker in ["[[AUTH_ERROR]]", "[[PROVIDER_ERROR]]"]):
            detail = 'Authorization: Bearer synthetic-sensitive-value' if "[[AUTH_ERROR]]" in text else 'Connection refused by the configured model endpoint.'
            detail += '\nPrivate diagnostic with synthetic-sensitive-value'
            emit({"method": "turn/started", "params": {"threadId": thread_id, "turn": {"id": turn_id}}})
            emit({"method": "turn/completed", "params": {"threadId": thread_id, "turn": {"id": turn_id, "status": "failed", "error": {"message": detail}}}})
            continue
        if thread_id in fix_threads and "[[FIX_" in text:
            if params.get("environments") != []:
                reject_invalid_params(request_id, "fix turns must disable native environments")
                continue
            emit({"method": "turn/started", "params": {"threadId": thread_id, "turn": {"id": turn_id}}})
            tool_id = f"fix-tool-{turn_count}"
            fix_pending[tool_id] = (thread_id, turn_id)
            path = "../outside.txt" if "[[FIX_ESCAPE]]" in text else "app.ts"
            emit({"id": tool_id, "method": "item/tool/call", "params": {"threadId": thread_id, "turnId": turn_id, "callId": tool_id, "tool": "cvec_workspace_write_file", "arguments": {"path": path, "content": "export const fixed = true;\n"}}})
            continue
        if "[[CRASH_AFTER_START]]" in text:
            emit({"method": "turn/started", "params": {"threadId": thread_id, "turn": {"id": turn_id}}})
            sys.stderr.write('Bearer secret-token-value {"apiKey":"sk-fixture-secret-value"}\n')
            sys.stderr.flush()
            raise SystemExit(23)
        if "[[MALFORMED]]" in text:
            sys.stdout.write("{not-json}\n")
            sys.stdout.flush()
        if "[[OVERSIZED]]" in text:
            sys.stdout.write('{"method":"fixture/oversized","params":{"value":"')
            sys.stdout.write("x" * (2 * 1024 * 1024 + 256))
            sys.stdout.write('"}}\n')
            sys.stdout.flush()
        if "[[UNKNOWN]]" in text:
            emit({"method": "fixture/unknown", "params": {"threadId": thread_id, "turnId": turn_id, "unsafe": "ignored"}})
        if "[[APPROVAL]]" in text:
            emit({"id": 99999, "method": "item/commandExecution/requestApproval", "params": {"threadId": thread_id, "turnId": turn_id, "command": "touch forbidden"}})
        citation_id = "repository:1" if "[[REPO_CITATION]]" in text else "scan:10"
        answer = f"Grounded answer [source:{citation_id}]"
        emit({"method": "turn/started", "params": {"threadId": thread_id, "turn": {"id": turn_id}}})
        emit({"method": "item/started", "params": {"threadId": thread_id, "turnId": turn_id, "item": {"id": "assistant-1", "type": "agentMessage", "text": ""}}})
        emit({"method": "item/agentMessage/delta", "params": {"threadId": thread_id, "turnId": turn_id, "itemId": "assistant-1", "delta": answer}})
        emit({"method": "thread/tokenUsage/updated", "params": {"threadId": thread_id, "turnId": turn_id, "tokenUsage": {"last": {"inputTokens": 12, "outputTokens": 8}, "total": {"totalTokens": 20}, "modelContextWindow": 8192}}})
        if "[[WAIT]]" not in text or "[[MESSAGE_WAIT]]" in text:
            emit({"method": "item/completed", "params": {"threadId": thread_id, "turnId": turn_id, "item": {"id": "assistant-1", "type": "agentMessage", "text": answer}}})
        if "[[WAIT]]" not in text and "[[MESSAGE_WAIT]]" not in text:
            emit({"method": "turn/completed", "params": {"threadId": thread_id, "turn": {"id": turn_id, "status": "completed"}}})
    elif method == "turn/interrupt":
        if params.get("turnId") in interrupt_errors:
            reject_sensitive_rpc(request_id)
            continue
        emit({"id": request_id, "result": {}})
        emit({"method": "turn/completed", "params": {"threadId": params.get("threadId"), "turn": {"id": params.get("turnId"), "status": "interrupted"}}})
    else:
        emit({"id": request_id, "error": {"code": -32601, "message": f"unsupported fixture method: {method}"}})

log("EXIT")

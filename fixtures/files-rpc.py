#!/usr/bin/env python3
"""Metadata-only RPC peer for native file-workflow validation. Not a demo adapter."""
import json
import os
import sys

if os.environ.get("PI_FILES_PID_LOG"):
    with open(os.environ["PI_FILES_PID_LOG"], "a", encoding="utf-8") as log:
        log.write(json.dumps({"pid": os.getpid(), "cwd": os.getcwd()}) + "\n")
other_project = os.environ.get("PI_FILES_OTHER_PROJECT")
saved = [{"id": "saved-other", "path": os.path.join(other_project, "saved.jsonl"), "cwd": other_project, "firstMessage": "Saved elsewhere"}] if other_project else []

messages = []
if os.environ.get("PI_FILES_LONG_CHANGE"):
    call_id = "call_" + "very-long-id-" * 24 + "終点"
    content = "WRAP_START " + "A deliberately long fixture line with preserved spaces. " * 8 + "WRAP_END"
    messages = [
        {"role": "assistant", "content": [{"type": "toolCall", "id": call_id, "name": "write", "arguments": {"path": "long.txt", "content": content}}]},
        {"role": "toolResult", "toolCallId": call_id, "toolName": "write", "content": [{"type": "text", "text": "Fixture only; no file was written."}]},
    ]

for line in sys.stdin:
    command = json.loads(line)
    kind = command["type"]
    with open(os.environ["PI_FILES_COMMAND_LOG"], "a", encoding="utf-8") as log:
        log.write(kind + "\n")
    if kind == "prompt" and os.environ.get("PI_FILES_PROMPT_LOG"):
        with open(os.environ["PI_FILES_PROMPT_LOG"], "a", encoding="utf-8") as log:
            log.write(json.dumps(command["message"]) + "\n")
    responses = {
        "get_state": {"sessionId": "file-ui-test", "sessionName": "File actions test", "isStreaming": False},
        "get_messages": {"messages": messages},
        "get_session_stats": {},
        "list_sessions": {"sessions": saved},
        "get_entries": {"entries": [], "leafId": None},
        "get_settings": {},
        "get_commands": {"commands": [
            {"name": "inspect-offline", "description": "Inspect without sending", "source": "prompt"},
            {"name": "offline-skill", "description": "Native fixture skill", "source": "skill"},
        ] if os.environ.get("PI_FILES_COMMAND_CATALOG") else []},
    }
    response = {"type": "response", "id": command["id"], "command": kind, "success": kind in responses}
    if kind in responses:
        response["data"] = responses[kind]
    else:
        response["error"] = "Native file test forbids prompts, tools and model calls"
    print(json.dumps(response), flush=True)
    if kind == "get_commands" and os.environ.get("PI_FILES_CRASH"):
        print("node:events:487\nUnhandled error event\nError: EACCES opening saved session\n    at fixture.resume (fixture.js:42:1)", file=sys.stderr, flush=True)
        sys.exit(1)

#!/usr/bin/env python3
"""Offline, read-only activity history. Never executes the displayed tool calls."""
import json
import os
import sys
from pathlib import Path

import desktop_channel

records = [json.loads(line) for line in Path(__file__).with_name("readability.jsonl").read_text().splitlines()]
messages = records[2]["data"]["messages"]
end = next(i for i, message in enumerate(messages) if i > 0 and message["role"] == "user")
messages = messages[:end]
if os.environ.get("PI_ACTIVITY_FAILURE") == "1":
    for message in messages:
        if message.get("toolCallId") == "units-test":
            message.update(isError=True, content=[{"type": "text", "text": "Fixture command failed with exit code 1."}])
    messages[-1]["content"] = [{"type": "text", "text": "The command failed. Expand the activity row to inspect its original output."}]
model = {"provider": "fixture", "id": "offline", "contextWindow": 200000}
responses = {
    "get_state": {"sessionId": "offline-activity", "sessionName": "Offline activity", "model": model, "thinkingLevel": "off", "isStreaming": False},
    "get_messages": {"messages": messages},
    "get_entries": {"entries": [], "leafId": None},
    "get_session_stats": {"tokens": {"input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0}},
    "get_available_models": {"models": [model]},
    "get_available_thinking_levels": {"levels": ["off"]},
    "get_settings": {},
    "get_commands": {"commands": []},
    "list_sessions": {"sessions": []},
}
def answer(command):
    with open(os.environ["PI_ACTIVITY_LOG"], "a", encoding="utf-8") as log:
        log.write(json.dumps(command) + "\n")
    kind = command["type"]
    response = {"id": command["id"], "type": "response", "command": kind, "success": kind in responses}
    if kind in responses:
        response["data"] = responses[kind]
    else:
        response["error"] = "Offline activity fixture forbids prompts and mutations"
    return response


desktop_channel.serve(answer)
for line in sys.stdin:
    with desktop_channel.lock:
        desktop_channel.emit(answer(json.loads(line)))

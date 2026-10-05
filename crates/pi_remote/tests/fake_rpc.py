"""Deterministic streaming backend for SSH helper tests; never calls a model."""
import json
import sys
import threading
import time

lock = threading.Lock()
messages = []
busy = False


def emit(value):
    with lock:
        print(json.dumps(value), flush=True)


def run():
    global busy
    emit({"type": "agent_start"})
    emit({"type": "tool_execution_start", "toolCallId": "t", "toolName": "bash", "args": {"command": "sleep 1"}})
    emit({"type": "message_start", "message": {"role": "assistant", "content": [{"type": "text", "text": ""}]}})
    for word in ["hello", " remote", " world"]:
        time.sleep(0.4)
        emit({"type": "message_update", "assistantMessageEvent": {"type": "text_delta", "contentIndex": 0, "delta": word}})
    answer = {"role": "assistant", "content": [{"type": "text", "text": "hello remote world"}], "stopReason": "stop"}
    messages.append(answer)
    emit({"type": "message_end", "message": answer})
    emit({"type": "tool_execution_end", "toolCallId": "t", "toolName": "bash", "result": {"content": [{"type": "text", "text": "done"}]}, "isError": False})
    busy = False
    emit({"type": "agent_settled"})


for line in sys.stdin:
    request = json.loads(line)
    kind = request["type"]
    data = {}
    if kind == "get_state":
        data = {"sessionId": "fake", "sessionFile": "fake.jsonl", "isStreaming": busy}
    elif kind == "get_messages":
        data = {"messages": messages}
    elif kind == "get_session_stats":
        data = {"totalMessages": len(messages), "tokens": {"input": 0, "output": 0, "cacheRead": 0}}
    elif kind == "prompt":
        user = {"role": "user", "content": request["message"]}
        messages.append(user)
        busy = True
        emit({"type": "message_end", "message": user})
        threading.Thread(target=run, daemon=True).start()
    elif kind == "abort":
        emit({"type": "agent_settled"})
    emit({"type": "response", "id": request["id"], "command": kind, "success": True, "data": data})

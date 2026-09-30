"""Strictly offline subprocess fixture. No model calls or shell execution."""
import json
import os
import sys
import time

mode = sys.argv[1]


def emit(record):
    sys.stdout.write(json.dumps(record, ensure_ascii=False) + "\n")
    sys.stdout.flush()


def reply(request):
    emit({"type": "response", "id": request["id"], "command": request["type"],
          "success": True, "data": {"cwd": os.getcwd()}})


if mode == "hang":
    time.sleep(60)
else:
    held = None
    for line in sys.stdin.buffer:
        request = json.loads(line)
        if mode == "exit":
            sys.stderr.write("deliberate fixture failure\n")
            sys.stderr.flush()
            sys.exit(7)
        if mode == "malformed":
            print("not JSON", flush=True)
        elif mode == "timeout":
            time.sleep(0.25)
            reply(request)
        elif mode == "reverse":
            if held is None:
                held = request
            else:
                emit({"type": "agent_start"})
                reply(request)
                reply(held)
                held = None
        elif mode == "mismatch":
            emit({"type": "response", "id": request["id"], "command": "wrong", "success": True})
        else:
            reply(request)

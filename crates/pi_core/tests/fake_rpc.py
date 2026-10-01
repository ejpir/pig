"""Strictly offline subprocess fixture. No model calls or shell execution."""
import json
import os
import socket
import sys
import threading
import time

mode = sys.argv[1]


def extension():
    """Answers the desktop channel as Pi Desktop's extension does."""
    address = os.environ["PI_DESKTOP_CHANNEL"]
    if address.startswith("tcp:"):
        channel = socket.create_connection(("127.0.0.1", int(address[4:])))
    else:
        channel = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        channel.connect(address)
    hello = {"type": "hello", "piVersion": "fixture"}
    if "PI_DESKTOP_CHANNEL_TOKEN" in os.environ:
        hello["token"] = os.environ["PI_DESKTOP_CHANNEL_TOKEN"]
    channel.sendall((json.dumps(hello) + "\n").encode())
    for line in channel.makefile("r"):
        request = json.loads(line)
        channel.sendall((json.dumps({"type": "response", "id": request["id"], "command": request["type"],
                                     "success": True, "data": {"via": "extension"}}) + "\n").encode())


def emit(record):
    sys.stdout.write(json.dumps(record, ensure_ascii=False) + "\n")
    sys.stdout.flush()


def reply(request):
    emit({"type": "response", "id": request["id"], "command": request["type"],
          "success": True, "data": {"cwd": os.getcwd()}})


if mode == "extension":
    # Like pi, the extension connects only after startup work.
    time.sleep(0.2)
    threading.Thread(target=extension, daemon=True).start()
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

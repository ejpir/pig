"""Stands in for Pi Desktop's extension in the offline fixture peers.

The desktop sends the commands pi's RPC mode lacks to the extension over its channel
(PI_DESKTOP_CHANNEL; see crates/pi_core/src/channel.rs). A peer answers stdin with
`answer(command)` and calls `serve(answer)`, which connects as the extension does and
answers channel requests with the same function. Both threads hold `lock` while
answering, and `emit` writes one stdout record.
"""
import json
import os
import socket
import sys
import threading

lock = threading.RLock()


def emit(record):
    with lock:
        sys.stdout.write(json.dumps(record) + "\n")
        sys.stdout.flush()


def serve(answer):
    """Answers the channel on a background thread; nothing happens without one."""
    address = os.environ.get("PI_DESKTOP_CHANNEL")
    if not address:
        return
    if address.startswith("tcp:"):
        channel = socket.create_connection(("127.0.0.1", int(address[4:])))
    else:
        channel = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        channel.connect(address)
    hello = {"type": "hello", "piVersion": "fixture"}
    if os.environ.get("PI_DESKTOP_CHANNEL_TOKEN"):
        hello["token"] = os.environ["PI_DESKTOP_CHANNEL_TOKEN"]
    channel.sendall((json.dumps(hello) + "\n").encode())

    def run():
        for line in channel.makefile("r", encoding="utf-8"):
            with lock:
                response = answer(json.loads(line))
            channel.sendall((json.dumps(response) + "\n").encode())

    threading.Thread(target=run, daemon=True).start()

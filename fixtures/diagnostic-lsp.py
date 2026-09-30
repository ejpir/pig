#!/usr/bin/env python3
"""Deterministic local LSP peer for the native problem-card and `@` mention checks: one
diagnostic, hover text, a quick fix and a workspace symbol."""
import json
import os
import sys


def send(record):
    payload = json.dumps({"jsonrpc": "2.0", **record}).encode()
    sys.stdout.buffer.write(f"Content-Length: {len(payload)}\r\n\r\n".encode() + payload)
    sys.stdout.buffer.flush()


opened = []
while True:
    headers = {}
    while True:
        line = sys.stdin.buffer.readline()
        if not line:
            sys.exit(0)
        if line in (b"\n", b"\r\n"):
            break
        key, value = line.decode().split(":", 1)
        headers[key.lower()] = value.strip()
    record = json.loads(sys.stdin.buffer.read(int(headers["content-length"])))
    method = record.get("method", "response")
    entry = {"method": method, "pid": os.getpid()}
    if method == "textDocument/didChange":
        entry["text"] = record["params"]["contentChanges"][-1]["text"]
    with open(os.environ["PI_LSP_TEST_LOG"], "a", encoding="utf-8") as log:
        log.write(json.dumps(entry) + "\n")
    if method == "exit":
        break
    if "id" in record:
        result = None
        if method == "initialize":
            result = {"capabilities": {"textDocumentSync": 1, "hoverProvider": True, "codeActionProvider": True,
                                       "workspaceSymbolProvider": True}}
        elif method == "textDocument/hover":
            result = {"contents": {"language": "yaml", "value": "(key) broken: sequence"}}
        elif method == "workspace/symbol" and opened:
            # One symbol, the document's first key, for the `@` menu's Symbols group.
            result = [{"name": "broken", "kind": 13, "location": {"uri": opened[-1], "range": {
                "start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 6}}}}]
        elif method == "textDocument/codeAction":
            # Closes the first line's `[`, as a real YAML server's quick fix might.
            uri = record["params"]["textDocument"]["uri"]
            edit = {"range": {"start": {"line": 0, "character": 9}, "end": {"line": 0, "character": 9}}, "newText": "]"}
            result = [{"title": "Close the sequence", "kind": "quickfix", "edit": {"changes": {uri: [edit]}}}]
        send({"id": record["id"], "result": result})
    if method in ("textDocument/didOpen", "textDocument/didChange"):
        document = record["params"]["textDocument"]
        opened.append(document["uri"])
        send({"method": "textDocument/publishDiagnostics", "params": {
            "uri": document["uri"], "version": document.get("version"), "diagnostics": [{
                "range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 6}},
                "severity": 1, "message": "Diagnostic hover regression probe", "source": "local-probe", "code": "invalid-yaml"
            }]
        }})

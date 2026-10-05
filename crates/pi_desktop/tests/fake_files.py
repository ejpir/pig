"""Deterministic file RPC for composer tests; no Pi, SSH or project file reads."""
import json
import os
import sys

entries = [
    {"path": "src", "directory": True},
    {"path": "src/remote.rs", "directory": False},
    {"path": "src/unreadable.rs", "directory": False},
    {"path": "docs", "directory": True},
    {"path": "docs/设计", "directory": True},
    {"path": "docs/设计/hello world.md", "directory": False},
]
for line in sys.stdin:
    request = json.loads(line)
    with open(os.environ["PI_DESKTOP_TEST_FILE_LOG"], "a", encoding="utf-8") as log:
        log.write(json.dumps(request, ensure_ascii=False) + "\n")
    response = {"type": "response", "id": request["id"], "command": request["type"], "success": True}
    if request["type"] == "files_attach":
        response["data"] = {"version": 1, "target": request["target"]}
    elif request["type"] == "files_list":
        if os.environ.get("PI_DESKTOP_TEST_FILE_LIST_ERROR"):
            response.update(success=False, error="Remote directory unavailable")
        else:
            response["data"] = {"entries": entries, "truncated": False}
    elif request["type"] == "files_read":
        if request["path"] == "src/unreadable.rs":
            response.update(success=False, error="Remote permission denied")
        else:
            response["data"] = {"path": request["path"], "text": "// remote host only\r\n\tfn main() {}\r\n", "revision": "remote-revision"}
    else:
        response.update(success=False, error="Unexpected request")
    print(json.dumps(response, ensure_ascii=False), flush=True)

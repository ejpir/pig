#!/usr/bin/env python3
"""Offline catalog peer: synthetic metadata and state changes, never tools/models/installers."""
import json
import os
import sys

models = [
    {"id": "atlas-large", "name": "Atlas Large", "provider": "fixture-ai", "contextWindow": 200000,
     "maxTokens": 32000, "reasoning": True, "input": ["text", "image"],
     "cost": {"input": 3, "output": 15, "cacheRead": .3, "cacheWrite": 3.75}},
    {"id": "atlas-small", "name": "Atlas Small", "provider": "fixture-ai", "contextWindow": 128000,
     "maxTokens": 16000, "reasoning": False, "input": ["text"],
     "cost": {"input": .25, "output": 1.25}},
    {"id": "local-code", "name": "Local Code", "provider": "fixture-local", "contextWindow": 64000,
     "maxTokens": 8000, "reasoning": True, "input": ["text"], "cost": {"input": 0, "output": 0}},
    {"id": "unknown-limits", "provider": "fixture-local"},
]
selected = models[0]
settings = {"scopedModels": ["fixture-ai/atlas-large", "fixture-local/local-code"], "modelThinkingLevels": {}}
trusted = os.environ.get("PI_CATALOG_TRUSTED") == "1"
trust = {"cwd": os.getcwd(), "trusted": trusted, "hasProjectResources": True,
         "savedDecision": {"path": os.getcwd(), "decision": True} if trusted else None}
saved = [
    {"id": "multiline", "path": "/offline/multiline.jsonl", "cwd": os.getcwd(),
     "firstMessage": "Fix this language server error in file.ts:21:34:\n\nCannot find name agentStartMs.\u2028Keep this original message.",
     "messageCount": 5, "modified": "2026-09-29T12:00:00Z"},
    {"id": "named", "path": "/offline/named.jsonl", "cwd": os.getcwd(), "name": "A named\nsession",
     "firstMessage": "Original text", "messageCount": 30, "modified": "2026-09-29T11:00:00Z"},
    {"id": "short", "path": "/offline/short.jsonl", "cwd": os.getcwd(), "firstMessage": "Run a loop",
     "messageCount": 12, "modified": "2026-09-29T10:00:00Z"},
]
packages = [
    {"source": "npm:@fixture/git-guard@1.4.0", "scope": "user", "filtered": False, "installedPath": "/offline/packages/git-guard"},
    {"source": "git:example.invalid/pi-review@v1", "scope": "user", "filtered": True, "installedPath": "/offline/packages/review"},
    {"source": "./local-kit", "scope": "user", "filtered": False},
]
commands = [
    {"name": "commit", "description": "Review staged changes before committing", "source": "extension", "sourceInfo": {"path": "/offline/packages/git-guard/main.ts", "source": packages[0]["source"], "scope": "user", "origin": "package"}},
    {"name": "check", "description": "Check repository conventions", "source": "extension", "sourceInfo": {"path": "/offline/packages/git-guard/main.ts", "source": packages[0]["source"], "scope": "user", "origin": "package"}},
    {"name": "skill:review", "description": "Review changes carefully", "source": "skill", "sourceInfo": {"path": "/offline/skills/review/SKILL.md", "source": "auto", "scope": "user", "origin": "top-level"}},
    {"name": "release-notes", "description": "Draft release notes from changes", "source": "prompt", "sourceInfo": {"path": "/offline/prompts/release.md", "source": "auto", "scope": "user", "origin": "top-level"}},
]
for line in sys.stdin:
    command = json.loads(line)
    kind = command["type"]
    if log := os.environ.get("PI_CATALOG_LOG"):
        with open(log, "a", encoding="utf-8") as out:
            out.write(json.dumps(command) + "\n")
    responses = {
        "get_state": {"sessionId": "offline-catalog", "sessionName": "Catalog fixture", "model": selected, "thinkingLevel": "high", "isStreaming": False},
        "get_messages": {"messages": []}, "get_entries": {"entries": [], "leafId": None},
        "get_session_stats": {"tokens": {"input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0}},
        "list_sessions": {"sessions": saved}, "get_available_models": {"models": models},
        "get_available_thinking_levels": {"levels": ["off", "low", "medium", "high"] if selected.get("reasoning") else ["off"]},
        "get_settings": settings, "get_commands": {"commands": commands}, "list_packages": {"packages": packages},
        "get_project_trust": trust,
        "get_auth_providers": {"providers": [
            {"id": "fixture-ai", "name": "Fixture AI", "authType": "oauth", "canLogin": True, "status": {"type": "oauth", "source": "stored"}},
            {"id": "fixture-ai", "name": "Fixture AI", "authType": "api_key", "canLogin": True, "status": {"type": "oauth", "source": "stored"}},
            {"id": "fixture-local", "name": "Fixture Local", "authType": "api_key", "canLogin": False},
        ]},
    }
    if kind == "set_model":
        selected = next(model for model in models if model["provider"] == command["provider"] and model["id"] == command["modelId"])
        if command.get("persist"):
            settings.update(defaultProvider=selected["provider"], defaultModel=selected["id"])
        responses[kind] = selected
    elif kind == "set_scoped_models":
        settings["scopedModels"] = command["patterns"] or []
        responses[kind] = {"scopedModels": settings["scopedModels"]}
    elif kind == "set_model_thinking_level":
        settings["modelThinkingLevels"][command["provider"] + "/" + command["modelId"]] = command["level"]
        responses[kind] = {}
    elif kind == "set_project_trust":
        trust["savedDecision"] = {"path": os.getcwd(), "decision": command["choice"] == "trust"}
        responses[kind] = {"trusted": True, "savedPath": os.getcwd()}
    elif kind in {"reload", "install_package", "remove_package", "update_packages"}:
        # Deliberately no filesystem changes or network access, even on confirmation.
        responses[kind] = {}
    response = {"id": command["id"], "type": "response", "command": kind, "success": kind in responses}
    if kind in responses:
        response["data"] = responses[kind]
    else:
        response["error"] = "Offline catalog peer forbids prompts, model calls and tools"
    print(json.dumps(response), flush=True)
    if kind == "get_project_trust" and os.environ.get("PI_CATALOG_NOTICE") == "1":
        print(json.dumps({"type": "extension_ui_request", "method": "notify", "notifyType": "info",
                          "message": "Package configuration changed. Use Reload resources to apply it to this session."}), flush=True)

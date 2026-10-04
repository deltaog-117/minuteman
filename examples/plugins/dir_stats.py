#!/usr/bin/env python3
# Minuteman - a fast, Ranger-inspired terminal file manager
# Copyright (C) 2026  Davi Oliveira Gonçalves
#
# This program is free software: you can redistribute it and/or modify
# it under the terms of the GNU Affero General Public License as published
# by the Free Software Foundation, either version 3 of the License, or
# (at your option) any later version.
#
# This program is distributed in the hope that it will be useful,
# but WITHOUT ANY WARRANTY; without even the implied warranty of
# MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
# GNU Affero General Public License for more details.
#
# You should have received a copy of the GNU Affero General Public License
# along with this program.  If not, see <https://www.gnu.org/licenses/>.

"""A worked example of a Minuteman plugin, in plain Python with no dependencies.

Wire it up in config.toml:

    [[plugin]]
    name = "dir-stats"
    command = "python3"
    args = ["/path/to/dir_stats.py"]
    on_key = ["i", "I"]
    commands = ["stats"]
    events = ["dir_changed", "job_finished"]

It also registers `:state` while it runs (a `register_command` request at `init`), which needs no
line in the config: `:state` then does what `:plugin dir-stats state` does. The host keeps a
registered word even after the plugin exits, so `:state` starts it again if it has gone.

Then `i` (or `I`, or `:stats`) shows how many files and folders the browsed directory holds and
how many bytes the files add up to, and `:plugin dir-stats touch notes.txt` creates a file there,
which shows a plugin doing real work through the host and not just reading.
`:plugin dir-stats state` asks the host for the browser's state with `get_state` and shows the
entry under the cursor, how many entries are marked and the sort and view. With `events` set, it
also shows each directory you enter and each copy, move, delete and the like that finishes. A
plugin has to ask for `dir_changed`, `selection_changed` and `job_finished`; a burst of cursor
moves reaches it as one event from where it began to where it ended.

The protocol is one JSON object per line on stdin/stdout (see the `plugins` crate). The host
sends `event` notifications (`init` once, `key` and `command` as they happen); this program sends
back `log` notifications, which the status bar shows, and requests such as `read_dir` and
`create_file`, each answered by exactly one reply line carrying the same `id`.
"""

import json
import os
import sys
from collections import deque

API_VERSION = 1

# Events that arrive while a request is waiting for its reply are held and handled afterwards,
# so a reply is never mistaken for an event or the other way round.
pending = deque()
next_id = 1


def send(message):
    sys.stdout.write(json.dumps(message) + "\n")
    sys.stdout.flush()


def log(level, text):
    send({"jsonrpc": "2.0", "method": "log", "params": {"level": level, "message": text}})


def request(method, params):
    """Sends one request and returns its result, or raises RuntimeError on an error reply."""
    global next_id
    request_id = next_id
    next_id += 1
    send({"jsonrpc": "2.0", "id": request_id, "method": method, "params": params})
    for line in sys.stdin:
        if not line.strip():
            continue
        message = json.loads(line)
        if message.get("id") == request_id:
            if "error" in message:
                raise RuntimeError(message["error"].get("message", "error"))
            return message.get("result")
        pending.append(message)
    raise RuntimeError("the host closed the connection")


def stats(params):
    entries = request("read_dir", {"path": params["cwd"]})
    folders = sum(1 for entry in entries if entry["is_dir"])
    files = len(entries) - folders
    size = sum(entry["size"] for entry in entries if not entry["is_dir"])
    return f"{files} files, {folders} folders, {size} bytes"


def on_command(params):
    command = params.get("command")
    args = params.get("args", [])
    if command == "stats":
        log("info", stats(params))
    elif command == "state" or (command == "plugin" and args == ["state"]):
        state = request("get_state", {})
        cursor = os.path.basename(state["cursor"]) if state["cursor"] else "none"
        log(
            "info",
            f"state: cursor={cursor} marks={len(state['marks'])} sort={state['sort']} "
            f"view={state['view']} cwd={state['cwd']}",
        )
    elif command == "plugin" and args[:1] == ["touch"] and len(args) == 2:
        request("create_file", {"path": f"{params['cwd']}/{args[1]}"})
        log("info", f"created {args[1]}")
    else:
        log("warn", "try :stats, :plugin dir-stats state or :plugin dir-stats touch NAME")


def handle(message):
    if message.get("method") != "event":
        return
    params = message["params"]
    if params.get("api_version") != API_VERSION:
        log("error", f"built for API {API_VERSION}, the host speaks {params.get('api_version')}")
        sys.exit(1)
    name = params["name"]
    if name == "init":
        log("info", "dir-stats ready")
        try:
            request("register_command", {"name": "state"})
            log("info", "registered :state")
        except RuntimeError as error:
            log("warn", f"could not register :state: {error}")
    elif name == "key":
        log("info", f"[{params['key']}] {stats(params)}")
    elif name == "command":
        on_command(params)
    elif name == "dir_changed":
        log("info", f"entered {os.path.basename(params['to']) or params['to']}")
    elif name == "selection_changed":
        log("info", f"cursor on {os.path.basename(params['to']) if params['to'] else 'nothing'}")
    elif name == "job_finished":
        log("info", f"{params['kind']} {params['outcome']}")


def main():
    while True:
        if pending:
            message = pending.popleft()
        else:
            line = sys.stdin.readline()
            if not line:
                return
            if not line.strip():
                continue
            message = json.loads(line)
        try:
            handle(message)
        except RuntimeError as error:
            log("error", str(error))


if __name__ == "__main__":
    main()

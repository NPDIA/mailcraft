# SPDX-FileCopyrightText: MailCraft contributors
# SPDX-License-Identifier: MIT OR Apache-2.0
"""Reopen the installer smoke fixture in a real desktop session."""

import argparse
import hashlib
import hmac
import json
import os
from pathlib import Path
import re
import secrets
import shutil
import socket
import subprocess
import tempfile
import time

LOOPBACK = "127.0.0.1"
START_TIMEOUT = 40
CALL_TIMEOUT = 5


def ask(stream, method, *, params=None, key=None):
    request = {"id": 1, "method": method, "params": params or {}}
    if key is not None:
        request["key"] = key
    stream.write(json.dumps(request).encode("utf-8") + b"\n")
    stream.flush()
    line = stream.readline(1024 * 1024 + 1)
    if not line or len(line) > 1024 * 1024:
        raise RuntimeError(f"The desktop did not return a complete response to {method}")
    try:
        reply = json.loads(line)
    except ValueError:
        raise RuntimeError(f"The desktop returned an invalid response to {method}") from None
    if reply.get("id") != request["id"] or not reply.get("ok"):
        raise RuntimeError(f"The desktop declined {method}")
    return reply["result"]


def connect(process, port, key_path, deadline):
    while time.monotonic() < deadline:
        if process.poll() is not None:
            raise RuntimeError(f"The desktop exited before opening its control channel ({process.returncode})")
        if key_path.is_file():
            key = key_path.read_text(encoding="ascii").strip()
            if re.fullmatch("[a-f0-9]{64}", key):
                try:
                    return socket.create_connection((LOOPBACK, port), CALL_TIMEOUT), key
                except ConnectionRefusedError:
                    pass
        time.sleep(0.1)
    raise RuntimeError("The desktop did not start its control channel")


def inspect_desktop(stream, key, records, deadline):
    nonce = secrets.token_hex(16)
    proof = ask(stream, "auth.challenge", params={"nonce": nonce})["proof"]
    expected = hmac.new(key.encode("ascii"), f"mailcraft-control-v1 {nonce}".encode("ascii"), hashlib.sha256).hexdigest()
    if not hmac.compare_digest(proof, expected):
        raise RuntimeError("The desktop control channel could not prove its identity")
    ask(stream, "auth", key=key)
    while ask(stream, "app.status")["mailbox"] != "open":
        if time.monotonic() >= deadline:
            raise RuntimeError("The desktop did not finish opening the fixture")
        time.sleep(0.1)

    def execute(command, **params):
        return ask(stream, "engine.execute", params={"command": command, "params": params})

    account = records["account"]
    actual = execute("account.list")["accounts"]
    if not any(item["id"] == account["id"] and item["email"] == account["email"] for item in actual):
        raise RuntimeError("The desktop did not reopen the synthetic account ID")
    original_id, draft_id, *_ = records["message_ids"]
    original = execute("mail.get", id=original_id)
    draft = execute("mail.get", id=draft_id)
    if original["id"] != original_id or original["subject"] != "Upgrade fixture original" or not original["body"]:
        raise RuntimeError("The desktop did not reopen the synthetic original")
    if draft["id"] != draft_id or not draft["draft"] or draft["subject"] != "Upgrade fixture draft" or not draft["body"]:
        raise RuntimeError("The desktop did not reopen the synthetic draft")
    ask(stream, "app.quit")


def run(app, source, records):
    with tempfile.TemporaryDirectory(prefix="mailcraft-desktop-profile-") as temporary:
        root = Path(temporary)
        windows = os.name == "nt"
        settings = root / ("MailCraft_NPDIA_Custom" if windows else "mailcraft-npdia-custom")
        settings.mkdir()
        shutil.copyfile(source / "mailbox.mailcraft", settings / "mailbox.mailcraft")
        shutil.copytree(source / "mailbox.mailcraft.blobs", settings / "mailbox.mailcraft.blobs")
        prefs = json.loads((source / "ui.json").read_text(encoding="utf-8"))
        prefs.update({
            "updateCheck": "off", "updateInstall": False, "rememberSignIn": False, "signInsBound": True,
            "options": {**prefs.get("options", {}), "offline": True, "syncMinutes": 0},
        })
        (settings / "ui.json").write_text(json.dumps(prefs), encoding="utf-8")
        env = dict(os.environ)
        for name in ("MAILCRAFT_NO_PREFS", "MAILCRAFT_CONTROL_AGENTS_ONLY"):
            env.pop(name, None)
        env["APPDATA" if windows else "XDG_CONFIG_HOME"] = temporary
        env["LOCALAPPDATA" if windows else "XDG_CACHE_HOME"] = str(root / "cache")
        with socket.socket() as reservation:
            reservation.bind((LOOPBACK, 0))
            port = reservation.getsockname()[1]
        key_path = settings / f"control-key.local-{port}"
        with subprocess.Popen([app, "--control", str(port)], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL) as process:
            try:
                deadline = time.monotonic() + START_TIMEOUT
                connection, key = connect(process, port, key_path, deadline)
                with connection, connection.makefile("rwb") as stream:
                    inspect_desktop(stream, key, records, deadline)
                if process.wait(timeout=10) != 0:
                    raise RuntimeError("The desktop did not close normally")
            finally:
                if process.poll() is None:
                    process.kill()
                    process.wait(timeout=5)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--app", type=Path, required=True)
    parser.add_argument("--profile", type=Path, required=True)
    parser.add_argument("--state", type=Path, required=True)
    args = parser.parse_args()
    if os.name != "nt" and not os.environ.get("DISPLAY") and not os.environ.get("WAYLAND_DISPLAY"):
        parser.error("A desktop session is required")
    records = json.loads(args.state.read_text(encoding="utf-8"))["records"]
    if not records["account"]["email"].endswith("@example.invalid"):
        parser.error("The desktop probe requires a synthetic installer smoke fixture")
    run(args.app.resolve(strict=True), args.profile, records)
    print("Installed desktop reopened the synthetic account, original and draft")


if __name__ == "__main__":
    main()

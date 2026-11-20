# SPDX-FileCopyrightText: MailCraft contributors
# SPDX-License-Identifier: MIT OR Apache-2.0
"""Exercise the installed CLI against the same disposable, desktop-format mailbox."""

import argparse
import base64
import ctypes
from ctypes import wintypes
from email.message import EmailMessage
from email.policy import SMTP
import hashlib
import json
import os
from pathlib import Path
import secrets
import subprocess

ORIGINAL_SUBJECT = "Upgrade fixture original"
ORIGINAL_BODY = "A received synthetic message body."
DRAFT_SUBJECT = "Upgrade fixture draft"
DRAFT_BODY = "A synthetic unsent draft."
QUEUED_SUBJECT = "Upgrade fixture queued message"
QUEUED_BODY = "A synthetic message queued before upgrading."
ALIAS = "upgrade-alias@example.invalid"
REFERENCE_NAME = "Upgrade references"
RULE_NAME = "Upgrade rule"
ATTACHMENT = bytes(range(256)) * 16
DRAFT_ATTACHMENT = b"Upgrade fixture draft attachment"
PREFERENCES = {"theme": "dark", "rememberSignIn": True}
INCOMING = {"host": "127.0.0.1", "port": 1993, "security": "tls"}
OUTGOING = {"host": "127.0.0.1", "port": 1465, "security": "tls"}


class Credential(ctypes.Structure):
    _fields_ = [
        ("Flags", wintypes.DWORD),
        ("Type", wintypes.DWORD),
        ("TargetName", wintypes.LPWSTR),
        ("Comment", wintypes.LPWSTR),
        ("LastWritten", wintypes.FILETIME),
        ("CredentialBlobSize", wintypes.DWORD),
        ("CredentialBlob", ctypes.POINTER(wintypes.BYTE)),
        ("Persist", wintypes.DWORD),
        ("AttributeCount", wintypes.DWORD),
        ("Attributes", ctypes.c_void_p),
        ("TargetAlias", wintypes.LPWSTR),
        ("UserName", wintypes.LPWSTR),
    ]


class WindowsCredentials:
    """Store only generated test data under this fixture's unique account name."""

    GENERIC = 1
    ENTERPRISE = 3
    NOT_FOUND = 1168

    def __init__(self, address, ownership):
        self.key = f"password:{address}|imap=127.0.0.1:1993|smtp=127.0.0.1:1465"
        self.ownership = ownership
        self.api = ctypes.WinDLL("Advapi32", use_last_error=True)
        credential_ptr = ctypes.POINTER(Credential)
        self.api.CredWriteW.argtypes = (credential_ptr, wintypes.DWORD)
        self.api.CredWriteW.restype = wintypes.BOOL
        self.api.CredReadW.argtypes = (wintypes.LPCWSTR, wintypes.DWORD, wintypes.DWORD, ctypes.POINTER(credential_ptr))
        self.api.CredReadW.restype = wintypes.BOOL
        self.api.CredDeleteW.argtypes = (wintypes.LPCWSTR, wintypes.DWORD, wintypes.DWORD)
        self.api.CredDeleteW.restype = wintypes.BOOL
        self.api.CredFree.argtypes = (ctypes.c_void_p,)
        self.api.CredFree.restype = None

    def target(self, part):
        return f"{self.key}#{part}.MailCraft_NPDIA_Custom"

    def write(self, part, value):
        data = (wintypes.BYTE * len(value)).from_buffer_copy(value)
        entry = Credential(
            Type=self.GENERIC,
            TargetName=self.target(part),
            CredentialBlobSize=len(data),
            CredentialBlob=data,
            Persist=self.ENTERPRISE,
            UserName=f"{self.key}#{part}",
        )
        if not self.api.CredWriteW(ctypes.byref(entry), 0):
            raise RuntimeError(f"Could not write synthetic Windows credential (error {ctypes.get_last_error()})")

    def read(self, part):
        pointer = ctypes.POINTER(Credential)()
        if not self.api.CredReadW(self.target(part), self.GENERIC, 0, ctypes.byref(pointer)):
            error = ctypes.get_last_error()
            if error == self.NOT_FOUND:
                return None
            raise RuntimeError(f"Could not read synthetic Windows credential (error {error})")
        try:
            entry = pointer.contents
            return ctypes.string_at(entry.CredentialBlob, entry.CredentialBlobSize)
        finally:
            self.api.CredFree(pointer)

    def seed(self):
        if self.read("0") is not None or self.read("n") is not None:
            raise RuntimeError("The synthetic credential name already exists")
        with self.ownership.open("x", encoding="utf-8") as marker:
            marker.write(self.key)
        secret = secrets.token_hex(24).encode("utf-16-le")
        self.write("0", secret)
        self.write("n", "1".encode("utf-16-le"))
        return hashlib.sha256(secret).hexdigest()

    def verify(self, expected_digest):
        secret = self.read("0")
        if secret is None or self.read("n") != "1".encode("utf-16-le") or hashlib.sha256(secret).hexdigest() != expected_digest:
            raise RuntimeError("The synthetic Windows credential did not survive")

    def delete(self):
        if not self.ownership.exists():
            return
        if self.ownership.read_text(encoding="utf-8") != self.key:
            raise RuntimeError("The credential ownership marker does not match this fixture")
        for part in ("n", "0"):
            if not self.api.CredDeleteW(self.target(part), self.GENERIC, 0):
                error = ctypes.get_last_error()
                if error != self.NOT_FOUND:
                    raise RuntimeError(f"Could not delete synthetic Windows credential (error {error})")
        self.ownership.unlink()


def require(condition, detail):
    if not condition:
        raise RuntimeError(detail)


def invoke(cli, *args):
    result = subprocess.run([cli, *args], capture_output=True, text=True, encoding="utf-8", timeout=30)
    if result.returncode != 0:
        raise RuntimeError(f"Installed CLI {args[0]} failed with exit code {result.returncode}: {result.stderr.strip()[:1000]}")
    if any(line.startswith("warning:") for line in result.stderr.splitlines()):
        raise RuntimeError("Installed CLI reported a mailbox opening warning")
    return result.stdout


def commands(cli, mailbox, *items, save=False):
    args = ["run", "--file", mailbox, "--print"]
    for name, params in items:
        args.extend(("--cmd", f"{name}={json.dumps(params)}"))
    if save:
        args.extend(("--save", mailbox))
    pending = invoke(cli, *args).lstrip()
    results = []
    decoder = json.JSONDecoder()
    for name, _ in items:
        command, end = decoder.raw_decode(pending)
        require(command["command"] == name, f"Unexpected result for {name}")
        results.append(command["result"])
        pending = pending[end:].lstrip()
    require(not pending, "Unexpected additional CLI output")
    return results


def inspect(cli, mailbox, address, reference):
    accounts, folders, rules = commands(cli, mailbox, ("account.list", {}), ("folder.list", {}), ("rules.list", {}))
    account = next((account for account in accounts["accounts"] if account["email"] == address), None)
    require(account is not None and account["incoming"] == INCOMING and account["outgoing"] == OUTGOING,
            "The synthetic account and its server settings could not be opened")
    _, linked, drafts, outbox = commands(
        cli, mailbox,
        ("folder.referenceOpen", {"folder": reference}),
        ("mail.list", {}),
        ("mail.list", {"folder": "drafts", "account": account["id"]}),
        ("mail.list", {"folder": "outbox", "account": account["id"]}),
    )
    require(linked["folder"] == reference and len(linked["messages"]) == 1, "The reference folder was not retained")
    require(len(drafts["messages"]) == 1, "The draft folder was not retained")
    require(len(outbox["messages"]) == 1, "The queued message was not retained")
    original_id = linked["messages"][0]["id"]
    draft_id = drafts["messages"][0]["id"]
    queued_id = outbox["messages"][0]["id"]
    original = json.loads(invoke(cli, "show", mailbox, str(original_id)))
    draft = json.loads(invoke(cli, "show", mailbox, str(draft_id)))
    queued = json.loads(invoke(cli, "show", mailbox, str(queued_id)))
    require(original["subject"] == ORIGINAL_SUBJECT and original["body"].strip() == ORIGINAL_BODY,
            "The original message body was not retained")
    require(draft["draft"] and draft["subject"] == DRAFT_SUBJECT and draft["body"].strip() == DRAFT_BODY
            and ALIAS in draft["from"], "The unsent draft, body, or selected alias was not retained")
    require(queued["subject"] == QUEUED_SUBJECT and queued["body"].strip() == QUEUED_BODY,
            "The queued message body was not retained")
    require(original["attachments"][0]["name"] == "original.bin" and draft["attachments"][0]["name"] == "draft.bin",
            "An attachment name changed")
    original_part, draft_part, *_ = commands(
        cli, mailbox,
        ("mail.saveAttachment", {"id": original_id, "index": 0}),
        ("mail.saveAttachment", {"id": draft_id, "index": 0}),
        ("mail.new", {"account": account["id"]}),
        ("compose.set", {"fromAlias": ALIAS}),
    )
    require(base64.b64decode(original_part["data"], validate=True) == ATTACHMENT
            and base64.b64decode(draft_part["data"], validate=True) == DRAFT_ATTACHMENT,
            "An attachment's bytes changed")
    rule = next((rule["rule"] for rule in rules["rules"] if rule["name"] == RULE_NAME), None)
    require(rule is not None and rule["enabled"] and rule["actions"] == [{"type": "referenceTo", "value": reference}],
            "The local rule or its reference folder destination changed")
    inbox = json.loads(invoke(cli, "list", mailbox, "--folder", "inbox"))
    require([message["id"] for message in inbox["messages"]] == [original_id], "The original was moved or duplicated")
    return {
        "account": account,
        "folders": folders["accounts"],
        "rule": rule,
        "reference_id": reference,
        "message_ids": [original_id, draft_id, queued_id],
        "original_folder_id": original["folder"],
        "draft_folder_id": draft["folder"],
        "outbox_folder_id": queued["folder"],
    }


def profile_files(mailbox):
    blobs = Path(f"{mailbox}.blobs")
    content_files = sorted(path for path in blobs.rglob("*") if path.is_file())
    require(bool(content_files), "The mailbox did not use a separate content store")
    all_files = [mailbox, mailbox.with_name("ui.json"), *content_files]
    return {path.relative_to(mailbox.parent).as_posix(): hashlib.sha256(path.read_bytes()).hexdigest() for path in all_files}


def seed(cli, mailbox, address, fixture_dir, credentials):
    message = EmailMessage()
    message["From"] = "sender@example.invalid"
    message["To"] = address
    message["Subject"] = ORIGINAL_SUBJECT
    message["Message-ID"] = "<upgrade-original@example.invalid>"
    message.set_content(ORIGINAL_BODY)
    message.add_attachment(ATTACHMENT, maintype="application", subtype="octet-stream", filename="original.bin")
    source = fixture_dir / "original.eml"
    source.write_bytes(message.as_bytes(policy=SMTP))
    invoke(cli, "import", mailbox, source)
    inbox = json.loads(invoke(cli, "list", mailbox, "--folder", "inbox"))
    original_id = inbox["messages"][0]["id"]
    account, reference = commands(
        cli, mailbox,
        ("account.add", {"kind": "imap", "email": address, "name": "Upgrade account", "incoming": INCOMING,
                         "outgoing": OUTGOING, "sendAliases": [ALIAS]}),
        ("folder.referenceNew", {"name": REFERENCE_NAME}),
        save=True,
    )
    reference_id = reference["folder"]
    commands(
        cli, mailbox,
        ("mail.referenceCopy", {"folder": reference_id, "id": original_id}),
        ("rules.add", {"name": RULE_NAME, "conditions": [{"type": "subjectContains", "value": ORIGINAL_SUBJECT}],
                       "actions": [{"type": "referenceTo", "value": reference_id}], "server": False}),
        ("mail.new", {"account": account["id"], "subject": DRAFT_SUBJECT, "body": DRAFT_BODY}),
        ("compose.set", {"fromAlias": ALIAS}),
        ("compose.attach", {"name": "draft.bin", "data": base64.b64encode(DRAFT_ATTACHMENT).decode("ascii")}),
        ("compose.saveDraft", {}),
        ("mail.new", {"account": account["id"], "to": "recipient@example.invalid",
                      "subject": QUEUED_SUBJECT, "body": QUEUED_BODY}),
        ("compose.send", {}),
        save=True,
    )
    mailbox.with_name("ui.json").write_text(json.dumps(PREFERENCES) + "\n", encoding="utf-8")
    return {
        "records": inspect(cli, mailbox, address, reference_id),
        "files": profile_files(mailbox),
        "credential_sha256": credentials.seed() if credentials else None,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("stage", choices=("seed", "open", "files", "clean"))
    parser.add_argument("--profile", type=Path, required=True)
    parser.add_argument("--state", type=Path, required=True)
    parser.add_argument("--cli", type=Path)
    parser.add_argument("--windows-credentials", action="store_true")
    args = parser.parse_args()
    if args.windows_credentials and os.name != "nt":
        parser.error("--windows-credentials requires native Windows")
    fixture = hashlib.sha256(os.fsencode(args.state.resolve())).hexdigest()[:24]
    address = f"upgrade-{fixture}@example.invalid"
    credentials = WindowsCredentials(address, args.state.with_suffix(".credential-owned")) if args.windows_credentials else None
    mailbox = args.profile / "mailbox.mailcraft"
    if args.stage == "clean":
        if credentials:
            credentials.delete()
        return
    if args.stage in ("seed", "open") and args.cli is None:
        parser.error("--cli is required for seed and open")
    if args.stage == "seed":
        require(not mailbox.exists() and not args.state.exists(), "The upgrade fixture must start empty")
        state = seed(args.cli, mailbox, address, args.state.parent, credentials)
        args.state.write_text(json.dumps(state), encoding="utf-8")
        return
    state = json.loads(args.state.read_text(encoding="utf-8"))
    require(profile_files(mailbox) == state["files"], "Mailbox, content store, or settings file changed")
    if credentials:
        credentials.verify(state["credential_sha256"])
    if args.stage == "open":
        expected = state["records"]
        require(inspect(args.cli, mailbox, address, expected["reference_id"]) == expected,
                "MailCraft did not reopen the same accounts, folders, rule, and message IDs")


if __name__ == "__main__":
    main()

# SPDX-FileCopyrightText: MailCraft contributors
# SPDX-License-Identifier: MIT OR Apache-2.0
"""Exercise IMAP sign-in and folder recovery through the installed CLI."""

import argparse
from concurrent.futures import ThreadPoolExecutor
from datetime import datetime, timezone
from email.utils import format_datetime
import hashlib
import json
import os
from pathlib import Path
import re
import secrets
import shlex
import socket
import ssl
import subprocess
import tempfile

ADDRESS = "imap-fixture@example.invalid"
SUBJECT = "Synthetic first sync after PLAIN refusal"
CAPABILITIES = b"IMAP4rev1 ID AUTH=PLAIN"
RECEIVED_AT = datetime.now(timezone.utc)
MESSAGE = (
    "From: Fixture <sender@example.invalid>\r\n"
    f"To: {ADDRESS}\r\n"
    f"Subject: {SUBJECT}\r\n"
    f"Date: {format_datetime(RECEIVED_AT)}\r\n"
    "Message-ID: <first-sync-fixture@example.invalid>\r\n"
    "Content-Type: text/plain; charset=utf-8\r\n"
    "\r\n"
    "Read-only IMAP loopback fixture.\r\n"
).encode("ascii")
INTERNAL_DATE = RECEIVED_AT.strftime("%d-%b-%Y %H:%M:%S +0000").encode("ascii")
RECOVERY_SUBJECT = "Synthetic message from the recovered folder"
RECOVERY_MESSAGE = MESSAGE.replace(SUBJECT.encode("ascii"), RECOVERY_SUBJECT.encode("ascii"), 1)
RECOVERY_FOLDERS = {b"INBOX": None, b"Recovery": RECOVERY_MESSAGE, b"Other": None}
AUTH_METHODS = ["AUTHENTICATE PLAIN", "LOGIN"]
# Allow the installed client's production 60-second IMAP read timeout to expire.
SILENCE_TIMEOUT = 80
RECOVERY_TIMEOUT = 100

CREATE_CERTIFICATES = r"""
using namespace System.Security.Cryptography
using namespace System.Security.Cryptography.X509Certificates
param([string] $Directory)
$ErrorActionPreference = 'Stop'
function New-Request([string] $Name, [RSA] $Key) {
  [CertificateRequest]::new($Name, $Key, [HashAlgorithmName]::SHA256, [RSASignaturePadding]::Pkcs1)
}
$RootKey = [RSA]::Create(2048)
$LeafKey = [RSA]::Create(2048)
$Now = [DateTimeOffset]::UtcNow
$RootRequest = New-Request "CN=MailCraft loopback fixture $([Guid]::NewGuid())" $RootKey
$RootRequest.CertificateExtensions.Add([X509BasicConstraintsExtension]::new($true, $true, 0, $true))
$RootRequest.CertificateExtensions.Add([X509KeyUsageExtension]::new([X509KeyUsageFlags]::KeyCertSign, $true))
$RootRequest.CertificateExtensions.Add([X509SubjectKeyIdentifierExtension]::new($RootRequest.PublicKey, $false))
$Root = $RootRequest.CreateSelfSigned($Now.AddMinutes(-5), $Now.AddMinutes(30))
$LeafRequest = New-Request 'CN=127.0.0.1' $LeafKey
$LeafRequest.CertificateExtensions.Add([X509BasicConstraintsExtension]::new($false, $false, 0, $true))
$LeafRequest.CertificateExtensions.Add([X509KeyUsageExtension]::new([X509KeyUsageFlags]::DigitalSignature, $true))
$Usage = [OidCollection]::new()
[void]$Usage.Add([Oid]::new('1.3.6.1.5.5.7.3.1'))
$LeafRequest.CertificateExtensions.Add([X509EnhancedKeyUsageExtension]::new($Usage, $false))
$Names = [SubjectAlternativeNameBuilder]::new()
$Names.AddIpAddress([Net.IPAddress]::Loopback)
$LeafRequest.CertificateExtensions.Add($Names.Build())
$Leaf = $LeafRequest.Create($Root, $Now.AddMinutes(-2), $Now.AddMinutes(20), [byte[]](1,2,3,4))
[IO.File]::WriteAllBytes((Join-Path $Directory 'ca.cer'), $Root.RawData)
[IO.File]::WriteAllText((Join-Path $Directory 'ca.pem'), $Root.ExportCertificatePem())
[IO.File]::WriteAllText((Join-Path $Directory 'server.pem'), ($Leaf.ExportCertificatePem() + "`n" + $Root.ExportCertificatePem()))
[IO.File]::WriteAllText((Join-Path $Directory 'server-key.pem'), $LeafKey.ExportPkcs8PrivateKeyPem())
"""

UPDATE_MACHINE_ROOT = r"""
using namespace System.Security.Cryptography.X509Certificates
using namespace System.Security.Principal
param(
  [string] $CertificatePath,
  [ValidatePattern('^[0-9a-fA-F]{40}$')] [string] $Thumbprint,
  [ValidateSet('Add', 'Remove')] [string] $Action
)
$ErrorActionPreference = 'Stop'
if (-not $IsWindows -or $env:GITHUB_ACTIONS -cne 'true' -or $env:RUNNER_ENVIRONMENT -cne 'github-hosted') {
  throw 'The Windows fixture requires a disposable GitHub-hosted Actions runner.'
}
$Identity = [WindowsIdentity]::GetCurrent()
try {
  $Principal = [WindowsPrincipal]::new($Identity)
  if (-not $Principal.IsInRole([WindowsBuiltInRole]::Administrator)) {
    throw 'The Windows fixture requires elevation to update the LocalMachine root store.'
  }
} finally {
  $Identity.Dispose()
}
$Certificate = [X509Certificate2]::new($CertificatePath)
$Store = [X509Store]::new([StoreName]::Root, [StoreLocation]::LocalMachine)
try {
  if ($Certificate.Thumbprint -ine $Thumbprint) { throw 'Unexpected fixture CA thumbprint.' }
  $Store.Open([OpenFlags]::ReadWrite -bor [OpenFlags]::OpenExistingOnly)
  $Entries = $Store.Certificates.Find([X509FindType]::FindByThumbprint, $Thumbprint, $false)
  if ($Action -eq 'Add') {
    if ($Entries.Count -ne 0) { throw 'The temporary CA already exists in the LocalMachine root store.' }
    $Store.Add($Certificate)
    $ExpectedCount = 1
  } else {
    foreach ($Entry in $Entries) { $Store.Remove($Entry) }
    $ExpectedCount = 0
  }
  if ($Store.Certificates.Find([X509FindType]::FindByThumbprint, $Thumbprint, $false).Count -ne $ExpectedCount) {
    throw "Could not verify the temporary CA after '$Action' in the LocalMachine root store."
  }
} finally {
  $Store.Dispose()
  $Certificate.Dispose()
}
"""


def call(args, *, env=None, password="", success=True, timeout=45):
    result = subprocess.run(args, env=env, capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=timeout)
    if (result.returncode == 0) != success:
        detail = result.stderr.strip()
        if password:
            detail = detail.replace(password, "[synthetic sign-in redacted]")
        raise RuntimeError(f"{Path(args[0]).name} returned {result.returncode}: {detail[:500]}")
    return result


def status_folder(args, folders, seen):
    folder = shlex.split(args.decode("ascii"))[0].encode("ascii")
    if folder not in folders or folder in seen:
        raise RuntimeError("Unexpected or repeated IMAP STATUS target")
    seen.add(folder)
    return folder


def imap(listener, context, password, *, untrusted=False, folders=None, stall=None):
    folders = {b"INBOX": MESSAGE} if folders is None else folders
    connection, _ = listener.accept()
    with connection:
        connection.settimeout(SILENCE_TIMEOUT if stall else 15)
        try:
            stream = context.wrap_socket(connection, server_side=True)
        except ssl.SSLError as err:
            if untrusted:
                return str(err.reason)
            raise
        with stream, stream.makefile("rb") as incoming:
            if untrusted:
                raise RuntimeError("The client accepted the fixture CA before trust was configured")
            stream.sendall(b"* OK synthetic loopback IMAP ready\r\n")
            methods = []
            fetched = []
            opened = None
            list_commands = 0
            status_targets = set()
            for _ in range(30):
                raw = incoming.readline(4096)
                if not raw:
                    break
                tag, _, request = raw.rstrip(b"\r\n").partition(b" ")
                if not re.fullmatch(rb"[A-Za-z0-9]+", tag):
                    raise RuntimeError("Unexpected client input without an IMAP command tag")
                verb, _, args = request.partition(b" ")
                verb = verb.upper()
                if verb == b"CAPABILITY":
                    stream.sendall(b"* CAPABILITY " + CAPABILITIES + b"\r\n" + tag + b" OK fixture completed\r\n")
                    continue
                elif verb == b"AUTHENTICATE":
                    if methods or args.upper() != b"PLAIN":
                        raise RuntimeError("Expected one PLAIN command without a credential payload")
                    methods.append("AUTHENTICATE PLAIN")
                    stream.sendall(tag + b" NO fixture refuses PLAIN before a challenge\r\n")
                    continue
                elif verb == b"LOGIN":
                    if methods != AUTH_METHODS[:1] or shlex.split(args.decode("ascii")) != [ADDRESS, password]:
                        raise RuntimeError("Unexpected LOGIN attempt or synthetic sign-in")
                    methods.append("LOGIN")
                    reply = tag + b" OK [CAPABILITY " + CAPABILITIES + b"] signed in\r\n"
                    stream.sendall(reply)
                    continue
                elif methods != AUTH_METHODS:
                    raise RuntimeError("Expected sign-in before mailbox access")
                if verb == b"LIST":
                    list_commands += 1
                elif verb == b"STATUS":
                    folder = status_folder(args, folders, status_targets)
                if verb == stall:
                    # No tagged response or other data. Only the remaining pipelined STATUS
                    # commands may arrive on this connection before the client closes it.
                    try:
                        while pending := incoming.readline(4096):
                            _, _, queued = pending.rstrip(b"\r\n").partition(b" ")
                            if stall != b"STATUS" or not queued.startswith(b"STATUS ") or len(status_targets) >= 2:
                                raise RuntimeError("Unexpected command on the timed-out IMAP connection")
                            status_folder(queued.removeprefix(b"STATUS "), folders, status_targets)
                    except (ConnectionResetError, ssl.SSLEOFError):
                        pass
                    if stall == b"STATUS" and status_targets != set(folders) - {b"INBOX"}:
                        raise RuntimeError("Expected one STATUS for each non-Inbox folder before reconnecting")
                    return {"auth": methods, "list_commands": list_commands, "status_commands": len(status_targets)}
                if verb == b"LIST":
                    reply = b"".join(b'* LIST (\\HasNoChildren) "/" "' + folder + b'"\r\n' for folder in folders)
                elif verb == b"ID":
                    reply = b"* ID NIL\r\n"
                elif verb == b"STATUS":
                    count = int(folders[folder] is not None)
                    reply = b'* STATUS "' + folder + f'" (MESSAGES {count} UNSEEN {count} UIDNEXT {count + 1} UIDVALIDITY 42)\r\n'.encode("ascii")
                elif verb in (b"EXAMINE", b"SELECT"):
                    opened = shlex.split(args.decode("ascii"))[0].encode("ascii")
                    count = int(folders[opened] is not None)
                    reply = f"* {count} EXISTS\r\n* 0 RECENT\r\n* OK [UIDVALIDITY 42] valid\r\n* OK [UIDNEXT {count + 1}] next\r\n".encode("ascii")
                elif verb == b"UID" and args.upper().startswith(b"FETCH "):
                    message = folders[opened]
                    if message is None:
                        reply = b""
                    elif b"BODY.PEEK[]" in args.upper():
                        reply = (b'* 1 FETCH (UID 1 FLAGS () INTERNALDATE "' + INTERNAL_DATE + b'" BODY[] {'
                                 + str(len(message)).encode("ascii") + b"}\r\n" + message + b")\r\n")
                        fetched.append(opened)
                    elif b"RFC822.SIZE" in args.upper():
                        reply = f"* 1 FETCH (UID 1 RFC822.SIZE {len(message)})\r\n".encode("ascii")
                    else:
                        reply = b"* 1 FETCH (UID 1 FLAGS ())\r\n"
                elif verb == b"LOGOUT":
                    stream.sendall(b"* BYE fixture finished\r\n" + tag + b" OK logout\r\n")
                    break
                else:
                    raise RuntimeError("Unexpected read-only IMAP operation")
                stream.sendall(reply + tag + b" OK fixture completed\r\n")
            if stall or methods != AUTH_METHODS or fetched != [folder for folder, message in folders.items() if message is not None]:
                raise RuntimeError("The IMAP fixture did not observe the expected stall, sign-in, or message fetch")
            return {"tls": stream.version(), "auth": methods, "list_commands": list_commands, "status_commands": len(status_targets)}


def prove_recovery(listener, context, password, stall):
    before = imap(listener, context, password, folders=RECOVERY_FOLDERS, stall=stall)
    after = imap(listener, context, password, folders=RECOVERY_FOLDERS)
    observed = (before["list_commands"], before["status_commands"], after["list_commands"], after["status_commands"])
    expected = (1, 0, 1, 2) if stall == b"LIST" else (1, 2, 0, 0)
    if observed != expected:
        raise RuntimeError(f"Unexpected LIST or STATUS command counts across the two connections: {observed}")
    return {"auth": [before["auth"], after["auth"]], "list_commands": observed[::2], "status_commands": observed[1::2]}


def test(cli, directory):
    certificate_script = directory / "certificates.ps1"
    certificate_script.write_text(CREATE_CERTIFICATES, encoding="ascii")
    call(["pwsh", "-NoProfile", "-NonInteractive", "-File", certificate_script, directory])
    context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    context.minimum_version = ssl.TLSVersion.TLSv1_2
    context.load_cert_chain(directory / "server.pem", directory / "server-key.pem")
    password = secrets.token_hex(16)
    environment = dict(os.environ, MAILCRAFT_PASSWORD=password)
    # A distinct process for each attempt makes the newly installed trust visible to the verifier.
    store_command = None
    if os.name == "nt":
        certificate = directory / "ca.cer"
        thumbprint = hashlib.sha1(certificate.read_bytes(), usedforsecurity=False).hexdigest()
        store_script = directory / "update-machine-root.ps1"
        store_script.write_text(UPDATE_MACHINE_ROOT, encoding="ascii")
        store_command = ["pwsh", "-NoProfile", "-NonInteractive", "-File", store_script,
                         "-CertificatePath", certificate, "-Thumbprint", thumbprint, "-Action"]
    else:
        environment.pop("SSL_CERT_FILE", None)
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as listener, ThreadPoolExecutor(max_workers=1) as pool:
        listener.bind(("127.0.0.1", 0))
        listener.listen(1)
        listener.settimeout(20)
        port = listener.getsockname()[1]
        mailbox = directory / "fixture.mailcraft"
        account = {"kind": "imap", "email": ADDRESS, "incoming": {"host": "127.0.0.1", "port": port, "security": "tls"}}
        call([cli, "run", "--cmd", "account.add=" + json.dumps(account), "--save", mailbox])
        sync = [cli, "sync", mailbox, "--account", ADDRESS]
        rejected = pool.submit(imap, listener, context, password, untrusted=True)
        call(sync, env=environment, password=password, success=False)
        alert = rejected.result(timeout=20)
        if "CERTIFICATE" not in alert and "UNKNOWN_CA" not in alert:
            raise RuntimeError(f"Untrusted handshake did not fail with a certificate alert: {alert}")
        try:
            if store_command:
                call([*store_command, "Add"])
            else:
                environment["SSL_CERT_FILE"] = str(directory / "ca.pem")
            session = pool.submit(imap, listener, context, password)
            call(sync, env=environment, password=password)
            first = session.result(timeout=20)
            result = {"tls": first["tls"], "auth": first["auth"]}
            for stall in (b"LIST", b"STATUS"):
                key = stall.decode("ascii").lower() + "_timeout"
                recovered = directory / f"{key}.mailcraft"
                call([cli, "run", "--cmd", "account.add=" + json.dumps(account), "--save", recovered])
                recovery = pool.submit(prove_recovery, listener, context, password, stall)
                call([cli, "sync", recovered, "--account", ADDRESS], env=environment, password=password, timeout=RECOVERY_TIMEOUT)
                evidence = recovery.result(timeout=20)
                messages = json.loads(call([cli, "list", recovered, "--folder", "Recovery"]).stdout)["messages"]
                if len(messages) != 1 or messages[0]["subject"] != RECOVERY_SUBJECT:
                    raise RuntimeError("The installed client did not retain the recovered folder's synthetic message")
                result[key] = dict(evidence, folder="Recovery", messages=len(messages))
        finally:
            if store_command:
                call([*store_command, "Remove"])
        inbox = json.loads(call([cli, "list", mailbox, "--folder", "inbox"]).stdout)
        messages = inbox["messages"]
        if len(messages) != 1 or messages[0]["subject"] != SUBJECT:
            raise RuntimeError("The installed client did not retain the synthetic Inbox message")
    return dict(result, certificate_validation="untrusted CA was refused", platform=os.name, inbox_messages=1,
                cli_version=call([cli, "--version"]).stdout.strip())


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cli", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    arguments = parser.parse_args()
    cli = arguments.cli.resolve(strict=True)
    with tempfile.TemporaryDirectory(prefix="mailcraft-imap-fixture-") as directory:
        result = test(cli, Path(directory))
    arguments.output.write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()

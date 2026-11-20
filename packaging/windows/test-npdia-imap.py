# SPDX-FileCopyrightText: MailCraft contributors
# SPDX-License-Identifier: MIT OR Apache-2.0
"""Exercise first Inbox sync through the installed CLI on a disposable Windows runner."""

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


def call(args, *, env=None, password="", success=True):
    result = subprocess.run(args, env=env, capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=45)
    if (result.returncode == 0) != success:
        detail = result.stderr.strip()
        if password:
            detail = detail.replace(password, "[synthetic sign-in redacted]")
        raise RuntimeError(f"{Path(args[0]).name} returned {result.returncode}: {detail[:500]}")
    return result


def imap(listener, context, password, *, untrusted=False):
    connection, _ = listener.accept()
    with connection:
        connection.settimeout(15)
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
            fetched = False
            for _ in range(30):
                raw = incoming.readline(4096)
                if not raw:
                    break
                tag, _, request = raw.rstrip(b"\r\n").partition(b" ")
                if not re.fullmatch(rb"[A-Za-z0-9]+", tag):
                    raise RuntimeError("Unexpected client input without an IMAP command tag")
                verb, _, args = request.partition(b" ")
                verb = verb.upper()
                reply = b""
                if verb == b"CAPABILITY":
                    reply = b"* CAPABILITY " + CAPABILITIES + b"\r\n"
                elif verb == b"AUTHENTICATE":
                    if methods or args.upper() != b"PLAIN":
                        raise RuntimeError("Expected one PLAIN command without a credential payload")
                    methods.append("AUTHENTICATE PLAIN")
                    stream.sendall(tag + b" NO fixture refuses PLAIN before a challenge\r\n")
                    continue
                elif verb == b"LOGIN":
                    if methods != ["AUTHENTICATE PLAIN"] or shlex.split(args.decode("ascii")) != [ADDRESS, password]:
                        raise RuntimeError("Unexpected LOGIN attempt or synthetic sign-in")
                    methods.append("LOGIN")
                    reply = tag + b" OK [CAPABILITY " + CAPABILITIES + b"] signed in\r\n"
                    stream.sendall(reply)
                    continue
                elif methods != ["AUTHENTICATE PLAIN", "LOGIN"]:
                    raise RuntimeError("Expected sign-in before mailbox access")
                elif verb == b"LIST":
                    reply = b'* LIST (\\HasNoChildren) "/" "INBOX"\r\n'
                elif verb == b"ID":
                    reply = b"* ID NIL\r\n"
                elif verb == b"STATUS":
                    reply = b'* STATUS "INBOX" (MESSAGES 1 UNSEEN 1 UIDNEXT 2 UIDVALIDITY 42)\r\n'
                elif verb in (b"EXAMINE", b"SELECT"):
                    reply = b"* 1 EXISTS\r\n* 0 RECENT\r\n* OK [UIDVALIDITY 42] valid\r\n* OK [UIDNEXT 2] next\r\n"
                elif verb == b"UID" and args.upper().startswith(b"FETCH "):
                    if b"BODY.PEEK[]" in args.upper():
                        reply = (b'* 1 FETCH (UID 1 FLAGS () INTERNALDATE "' + INTERNAL_DATE + b'" BODY[] {'
                                 + str(len(MESSAGE)).encode("ascii") + b"}\r\n" + MESSAGE + b")\r\n")
                        fetched = True
                    elif b"RFC822.SIZE" in args.upper():
                        reply = f"* 1 FETCH (UID 1 RFC822.SIZE {len(MESSAGE)})\r\n".encode("ascii")
                    else:
                        reply = b"* 1 FETCH (UID 1 FLAGS ())\r\n"
                elif verb == b"LOGOUT":
                    stream.sendall(b"* BYE fixture finished\r\n" + tag + b" OK logout\r\n")
                    break
                else:
                    raise RuntimeError("Unexpected read-only IMAP operation")
                stream.sendall(reply + tag + b" OK fixture completed\r\n")
            if methods != ["AUTHENTICATE PLAIN", "LOGIN"] or not fetched:
                raise RuntimeError("Expected one PLAIN refusal, one LOGIN, and one fetched body on the same TLS connection")
            return {"tls": stream.version(), "auth": methods}


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
            result = session.result(timeout=20)
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

# Security Architecture & Policies 🛡️

Security is a foundational design goal of **EvertySSH**. This document details the cryptographic safeguards, host key verification mechanisms, and credential protection implemented in the client.

---

## Credentials and startup lock

Host passwords and SSH key passphrases use the platform credential store:
Windows Credential Manager, macOS Keychain, or Linux Secret Service. TOML stores
opaque credential references, not plaintext passwords. Credentials are hydrated
in memory for SSH authentication. Private-key files remain at their original
paths and are not copied into the configuration or backup.

Legacy plaintext TOML credentials are migrated only after every required
credential write succeeds. Failure to read a credential or migrate stops startup
with an error; the original configuration is preserved. Linux requires an
unlocked Secret Service provider. There is no plaintext fallback.

The optional startup lock uses Argon2id with a random salt. Legacy SHA-256 hashes
are accepted for migration and upgraded after a successful unlock. The lock
blocks connection shortcuts as well as mouse actions. It is an interface lock,
not encryption for host metadata, the SQLite database, or existing private-key
files. Access by another process running as the same OS user is outside this
lock's protection.

## Backup format

New exports are authenticated, password-encrypted v2 `.evertybackup` files.
Argon2id v19 derives a 32-byte key with a random 16-byte salt, 19456 KiB memory,
two iterations and one lane. XChaCha20-Poly1305 uses a random 24-byte nonce and a
fixed authenticated format identifier. The envelope contains only version, salt,
nonce and ciphertext. Wrong passwords and modified ciphertext fail before any
configuration is changed. Derived keys and plaintext serialization buffers are
zeroized after use. Backups are limited to 16 MiB.

Use a separate backup password of at least eight characters. Export requires its
confirmation; import uses the password once. Backups contain host profiles,
passwords, key passphrases and settings. They do not contain private-key files,
SQLite command history, or snippets. Import merges host IDs, restores general
settings, and retains the existing local startup-lock password. Legacy plaintext
v1 JSON files remain importable when explicitly selected, but are never exported.
A damaged v2 backup is never treated as a legacy file.

## Configuration integrity

A malformed or unreadable configuration stops startup instead of loading empty
defaults. Writers serialize through a configuration lock and compare the original
file before and after credential-store operations. Data is written to a temporary
file in the same directory, synced, and atomically replaces the target. On Unix,
new configuration and backup files have mode 0600. The directory is synced where
supported after replacement. Save errors remain visible and do not claim success.
Editors that ignore the lock should not edit the configuration concurrently with
the app; external edits detected before replacement require a restart.

## Host-key verification

The client checks `~/.ssh/known_hosts` before authenticating. A matching key is
accepted; unknown and changed keys require confirmation with the server's SHA-256
fingerprint. Verify that fingerprint through a trusted channel. Confirmation
prompts are queued per session. Waiting for a prompt does not consume the
30-second network connection timeout. Closing a tab cancels its request.

Accepted keys are persisted atomically. Read and write failures stop the
connection. If a target host entry changes while a prompt is open, a matching newly
saved key is accepted; otherwise reconnect and verify again. Unrelated host
additions are merged without discarding either entry. Updates in this
process are serialized to prevent concurrent sessions from losing each other's
changes.

## Command history and paste

Raw terminal input is never logged. Command recording is opt-in and requires
[shell integration](SHELL_INTEGRATION.md); it records execution events after shell
editing and completion. Password responses to interactive programs do not create
such events. Command arguments can still contain secrets. Existing historical
entries are retained until deleted with their session. The remote server controls
integration events, so this is a convenience history, not an independent audit log.

The terminal tracks bracketed paste (`?2004h/l`) and wraps pasted data when the
remote application requests it. Multiline or control-character paste requires a
preview, bound to the original session. Confirmation sends the reviewed text
without appending Enter. Embedded ESC and unexpected control characters are
removed to prevent premature termination of the paste. With bracketed paste off,
newlines in confirmed text may execute commands immediately.

Commands from snippets and history require confirmation and use a heuristic
warning for potentially destructive operations. This warning is advisory; it is
not a complete shell-command safety analyzer.

## 📮 Reporting Vulnerabilities

If you discover a potential security vulnerability in EvertySSH, please report it responsibly by contacting the maintainer:

* **Email:** [info@everty.ru](mailto:info@everty.ru)
* **Telegram:** [@Everty_web](https://t.me/Everty_web)

Please include a detailed description of the issue, reproduction steps, and impact assessment. Vulnerabilities will be addressed with high priority.

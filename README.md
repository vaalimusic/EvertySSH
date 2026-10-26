<p align="center">
  <img src="Logo.png" alt="EvertySSH Logo" width="220" />
</p>

<h1 align="center">EvertySSH ⚡</h1>

<p align="center">
  <b>Ultra-fast, minimalist, and secure next-generation cross-platform SSH client written in Rust</b><br>
  Powered by the custom, independent <b>EvertyUI</b> vector rendering engine
</p>

<p align="center">
  <a href="README.md"><b>English</b></a> • <a href="README.ru.md"><b>Русский</b></a>
</p>

<p align="center">
  <img src="https://img.shields.io/badge/Release-v0.2.4-2dd4bf?style=for-the-badge" alt="Release v0.2.4" />
  <img src="https://img.shields.io/badge/Language-Rust%202021-DEA584?style=for-the-badge&logo=rust" alt="Rust" />
  <img src="https://img.shields.io/badge/UI_Engine-EvertyUI-2dd4bf?style=for-the-badge" alt="EvertyUI" />
  <img src="https://img.shields.io/badge/Platform-Windows%20%7C%20Linux%20%7C%20macOS-0284c7?style=for-the-badge" alt="Platforms" />
  <img src="https://img.shields.io/badge/License-MIT-emerald?style=for-the-badge" alt="License" />
</p>

---

## 📖 About the Project

**EvertySSH** is a lightweight, secure, and incredibly responsive desktop SSH client designed specifically for DevOps engineers, system administrators, and software developers.

Engineered with an uncompromising focus on raw performance, instant startup (<20ms), and zero bloat (no Electron, no Chromium, no Webview). EvertySSH renders directly into a hardware-independent frame buffer with sub-pixel antialiased vector geometry and typography via the custom **EvertyUI** framework.

The interface is designed in the signature **Everty** *Cyber Midnight* theme (`#090e1a` base with mint `#2dd4bf`, cyan `#38bdf8`, and emerald `#34d399` accents).

---

## 🌟 Why EvertySSH?

* **⚡ Blazing Fast:** < 20ms startup time, tiny memory footprint (< 35 MB RAM), and 0% CPU consumption when idle.
* **🛡️ Security by Default:** Automatic host key verification against `~/.ssh/known_hosts` to protect against local network ARP-spoofing and Man-in-the-Middle (MITM) attacks. Master-password locking supported.
* **🪟 True Split Screen:** Side-by-side terminal session split (`Ctrl+Alt+S`) with independent PTY streaming and a clean, non-intrusive 2px divider.
* **🔑 SSH Key Studio:** Generate OpenSSH Ed25519 & RSA-4096 key pairs and install them to remote `authorized_keys` in 1 click.
* **📜 Command Palette & SQLite History:** Intelligent command search (`Ctrl+R`), persistent session history, and execution logs.
* **🧩 Snippets Library:** Organize and trigger frequently used commands with automatic safety warnings before executing dangerous operations (`rm -rf`, `reboot`, `dd`).
* **🌐 Seamless Dual Keyboard Layout:** All keyboard shortcuts (`Ctrl+C`, `Ctrl+V`, `Ctrl+N`, `Ctrl+R`, `Ctrl+F`, etc.) work natively on both **English and Russian (QWERTY / ЙЦУКЕН)** keyboard layouts.
* **🎨 EvertyUI Foundation:** Built from scratch in Rust — complete architectural independence from third-party UI framework limitations.

---

## 🎨 EvertyUI Engine — Custom Native Framework

**EvertyUI** is an independent, low-overhead UI framework built specifically for desktop applications in the **Everty** software ecosystem (including **EvertyDesk Next**, **Viewer**, and **EvertySSH**).

### Core Philosophy

1. **Independent Architecture:** Interface layer never dictates core business logic. Asynchronous subsystems (Tokio reactor, Russh, VTE terminal) run freely in worker threads.
2. **Sub-millisecond Frame Times:** Frame rasterization is optimized for direct buffer presentation with smart glyph and clamped layout caches.
3. **Low Latency & High Frame Stability:** Cursor updates and hover events avoid redundant OS message loop dispatches.
4. **Unified Visual Language:** Clean design tokens, responsive layout containers, and unified control contracts.

Learn more in the [EvertyUI Architecture Guide](docs/ARCHITECTURE.md).

---

## ⚡ Key Features

### 🛡️ Host Key Verification & known_hosts Defense
* **Automatic Verification:** Checks and records remote server public keys against standard `~/.ssh/known_hosts`.
* **MITM & ARP-Spoofing Protection:** Rejects tampered keys immediately and displays an informative verification dialog with full SHA-256 fingerprint comparison.
* **Host Key Management:** Seamlessly accept new keys or remove obsolete host entries directly from the user interface.

### 🖥️ Server Manager (Hosts Inventory)
* **Quick Organization:** Group servers into environments (`Production`, `Staging`, `Development`, `Personal`).
* **Multi-Authentication:** System `SSH-Agent` (including Windows OpenSSH Authentication Agent), password, or Ed25519 / RSA private keys.
* **Smart Clamping:** Long server titles and connection addresses are gracefully clamped with ellipsis, keeping tab bars and cards clean at any screen scale.
* **Accidental Deletion Protection:** Confirmation modals prevent accidental host loss.
* **Plaintext TOML Storage:** Configuration is transparently stored in human-readable `~/.evertyssh/hosts.toml`.

### 🔑 Built-in SSH Key Studio
* Generate **Ed25519** and **RSA-4096** key pairs with custom passphrases and comments.
* Native system file picker for importing existing private keys.
* **1-Click Remote Deployment:** Generates, saves to disk, and automatically sends the setup command to `authorized_keys` on the active terminal.

### ⌨️ Advanced VT100 / ANSI Terminal
* **High Throughput:** Full ANSI escape sequence support, 256 colors, and 24-bit TrueColor.
* **DEC Private Modes:** Cursor visibility (`?25h`/`?25l`), alternate screen buffer (`?1049h`/`?1049l` for seamless `vim`, `htop`, `nano`, `mc`).
* **Split View (`Ctrl+Alt+S`):** Monitor two remote sessions simultaneously without clutter.
* **Terminal Search (`Ctrl+F`):** Case-sensitive search with instant match highlighting and `Enter` / `Shift+Enter` navigation.
* **Mouse Selection & Clipboard:** Seamless selection, auto-scroll, and right-click context menu.

### 📜 Command History & Session Dashboard
* SQLite (`~/.evertyssh/history.db`) stores session timestamps and duration. Command recording is off by default and uses [shell integration](docs/SHELL_INTEGRATION.md), never raw keystrokes.
* **Command Palette (`Ctrl+R`):** Substring history search with command preview and direct terminal dispatch.
* **Session Dashboard:** Detailed audit log for individual server sessions.

### 🧩 Command Snippets
* Library for frequently executed command recipes (`Docker`, `Kubernetes`, `System Monitoring`, `Deploy`).
* Execution modal with danger level detection (flags commands containing `rm -rf`, `reboot`, `mkfs`, etc.).

### 🔒 Security & Settings
* **Master Password Protection:** Lock the client on startup with secure password hashing.
* **Bilingual Interface:** Real-time 1-click toggle between English and Russian (`RU / EN`).
* **Backup & Restore:** Password-encrypted `.evertybackup` files contain server profiles, credentials and settings. Import also accepts legacy v1 JSON files; new exports are always encrypted. Snippets and session history remain in SQLite.
* **About Tab:** Quick links to developer channels, repository, and Everty ecosystem resources.

---

## ⌨️ Keyboard Shortcuts

| Shortcut (EN) | Shortcut (RU) | Action |
|---|---|---|
| `Ctrl + N` | `Ctrl + Т` | Add new server (Host dialog) |
| `Ctrl + R` | `Ctrl + К` | Command Palette / History Search |
| `Ctrl + T` | `Ctrl + Е` | Quick connect / New session |
| `Ctrl + W` | `Ctrl + Ц` | Close active tab or split view |
| `Ctrl + F` | `Ctrl + А` | Search in terminal buffer |
| `Ctrl + Alt + S` | `Ctrl + Alt + Ы` | Toggle Split Screen view |
| `Ctrl + V` | `Ctrl + М` | Paste from clipboard |
| `Ctrl + C` | `Ctrl + С` | Copy selection / Send SIGINT |
| `Ctrl + X` | `Ctrl + Ч` | Cut text in input fields |
| `Ctrl + A` | `Ctrl + Ф` | Select all text in active input |
| `Shift + Insert` | `Shift + Insert` | Alternative paste into terminal |
| `Tab` | `Tab` | Focus next input field |
| `Enter` | `Enter` | Confirm input / Execute command |
| `Esc` | `Esc` | Close dialog / Dismiss overlay |

---

## 🚀 Installation & Building

### 📦 Prebuilt Binaries
Download the latest executable for Windows directly from the [GitHub Releases](https://github.com/vaalimusic/EvertySSH/releases) page:
* `everty-ssh.exe` (standalone executable)
* `EvertySSH-windows-x64.zip` (portable archive)

### 🛠️ Building from Source

#### Prerequisites
* [Rust](https://www.rust-lang.org/tools/install) 1.75+ (stable toolchain)
* Cargo package manager

#### Build Steps
```bash
# Clone the repository
git clone https://github.com/vaalimusic/EvertySSH.git
cd EvertySSH

# Compile optimized release binary
cargo build --release --bin everty-ssh

# The compiled binary will be located at:
# target/release/everty-ssh (Linux/macOS) or target/release/everty-ssh.exe (Windows)
```

#### Run in Development Mode
```bash
cargo run --bin everty-ssh
```

#### Run Workspace Unit Tests
```bash
cargo test --workspace
```

---

## 📚 Documentation & Wiki

Explore detailed documentation in the [`docs/`](docs/) directory:
* [Architecture & EvertyUI Internal Design](docs/ARCHITECTURE.md)
* [Security, known_hosts & Cryptography](docs/SECURITY.md)
* [Complete Keyboard Shortcuts Reference](docs/KEYBOARD_SHORTCUTS.md)
* [Contribution Guidelines](CONTRIBUTING.md)

---

## 👨‍💻 Author & Contacts

**Arthur Valiev** — *Creator & Lead Engineer*

* 🌐 Official Website: [everty.ru](https://everty.ru)
* 🖥️ EvertyDesk Ecosystem: [desk.everty.ru](https://desk.everty.ru)
* ✈️ Telegram Channel: [@Everty_web](https://t.me/Everty_web)
* ✉️ Contact Email: [info@everty.ru](mailto:info@everty.ru)
* 🐙 GitHub: [@vaalimusic](https://github.com/vaalimusic)

---

## 📄 License

This project is licensed under the **MIT License**. See the [LICENSE](LICENSE) file for details.

## Credentials and configuration

Saved passwords and key passphrases use Windows Credential Manager, macOS
Keychain or Linux Secret Service. `hosts.toml` contains opaque credential
references. Existing plaintext credentials are migrated on startup; if the
credential store is unavailable, startup stops and the original file is retained.
On Linux, run an unlocked Secret Service provider such as GNOME Keyring or
KWallet with its Secret Service interface enabled.

The optional master password locks the interface and uses Argon2id. It does not
encrypt host metadata or the SQLite history database. Backup encryption uses a
separate password entered in Settings → Backup; repeat it for export, enter it
once for import. Existing hosts are merged by ID, settings are restored, and the
local master password is retained. SSH private-key files are not included.

Configuration writes use atomic replacement and detect external edits. Resolve
configuration errors and restart before saving; corrupt files are not replaced
with defaults. See [security](docs/SECURITY.md) and [shell integration](docs/SHELL_INTEGRATION.md).

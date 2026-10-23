# Contributing to EvertySSH 🤝

Thank you for your interest in contributing to **EvertySSH**! We welcome bug reports, feature suggestions, documentation enhancements, and pull requests.

---

## 🛠️ Development Setup

### 1. Prerequisites
* [Rust](https://www.rust-lang.org/tools/install) (version 1.75 or later, stable channel)
* Git

### 2. Fork and Clone
```bash
git clone https://github.com/vaalimusic/EvertySSH.git
cd EvertySSH
```

### 3. Verify Local Build & Tests
```bash
# Check code formatting & compilation
cargo check --workspace

# Run all unit and integration tests (146+ tests)
cargo test --workspace

# Run the application in development mode
cargo run -p everty-ssh
```

---

## 📋 Code Conventions & Guidelines

1. **Safety First:** Avoid `unwrap()` in production paths. Prefer descriptive `Result` or `Option` propagation.
2. **Deterministic UI Hit-testing:** When adding interactive UI components in `crates/everty-ssh/src/ui.rs`, ensure identical geometry calculations are placed in `crates/everty-ssh/src/app.rs` under `update_hover()`.
3. **No Unbounded Allocations:** For text formatting inside render frames, use `self.shape_cached()` or `self.shape_clamped_cached()`.
4. **Preserve Comments:** Keep docstrings and documentation intact.

---

## 🚀 Submitting Pull Requests

1. Create a feature branch: `git checkout -b feature/your-feature-name`
2. Commit changes using clear conventional commit messages: `feat: ...`, `fix: ...`, `docs: ...`
3. Run `cargo test --workspace` to ensure all tests pass.
4. Push to your fork and submit a Pull Request to the `main` branch.

Thank you for helping make EvertySSH even better!

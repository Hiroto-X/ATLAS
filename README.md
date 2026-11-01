# ATLAS — Phase 1 Foundation

This starter implements the first vertical slice:
- Tauri 2 desktop shell
- React + TypeScript frontend
- Rust backend
- Typed frontend contract
- Tauri IPC command `get_system_info`
- Basic system information display
- Initial read-only UI

This is a starter scaffold, not a claim that the project has been built or tested on your machine.

## Prerequisites (Windows)

Install:
1. Node.js LTS
2. Rust using rustup (MSVC toolchain)
3. Microsoft C++ Build Tools with the Desktop development with C++ workload
4. WebView2 Runtime (usually already present on modern Windows)

Tauri's Windows prerequisites are documented at:
https://v2.tauri.app/start/prerequisites/

## Run in development

Open PowerShell in this project folder:

```powershell
npm install
npm run tauri dev
```

The first Rust build can take several minutes while dependencies compile.

## Verify the frontend build

```powershell
npm run build
```

## Verify Rust

```powershell
cd src-tauri
cargo check
cargo test
cd ..
```

## Git initialization

If this is a new repository:

```powershell
git init
git add .
git commit -m "chore: initialize ATLAS foundation"
```

If you already have an ATLAS repository, do not overwrite it blindly. Extract this scaffold separately and compare/merge the files.

## First IPC slice

The React frontend calls `get_system_info` using `invoke<SystemInfo>()`. Rust returns OS, architecture, hostname and app version. The request is read-only and runs only when the user presses the button.

## Current limitations

- No system collectors or background monitoring.
- No database yet.
- No graph engine yet.
- No filesystem indexing or network collection.
- No AI integration.
- No installer signing or release packaging validation.

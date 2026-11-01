# ATLAS

### Explore your digital universe.

ATLAS is a desktop-first application designed to make complex digital environments understandable, explorable, and eventually manageable through one unified interface.

Instead of presenting system information as disconnected lists and dashboards, ATLAS aims to represent your digital environment as an interactive universe of entities and relationships.

From hardware and applications to processes, files, networks, and projects, ATLAS will bring everything together into one connected environment.

> Understand your digital world. Explore how everything connects.

---

## 🚀 Vision

The long-term vision of ATLAS is to build an interactive Digital Universe Explorer.

Users should eventually be able to:

* Discover the components of their computer.
* Explore relationships between hardware, software, processes, files, and networks.
* Visualize their digital environment through interactive 2D and 3D graphs.
* Understand changes and activity over time.
* Investigate their system using evidence-based AI assistance.
* Navigate complex digital environments through one unified interface.

ATLAS is designed to work without AI. Intelligence will enhance the experience rather than become a dependency.

---

## ✨ Planned Features

### 1. Digital Universe Explorer

The central experience of ATLAS.

* Interactive 2D graph exploration.
* 3D spatial exploration.
* Connected entities and relationships.
* Search, filtering, and progressive graph loading.
* Entity selection and detailed inspection.

### 2. System Discovery

Discover information about the local digital environment.

* CPU, GPU, RAM, and storage information.
* Running processes.
* Installed applications.
* Filesystem metadata.
* Network interfaces and connection metadata.
* Software and project dependencies.

### 3. Graph Engine

A structured representation of the digital environment.

* Typed entities and relationships.
* Entity identity management.
* Relationship evidence.
* Graph queries and traversal.
* Historical graph changes.

### 4. Activity and History

Understand how the digital environment changes over time.

* System activity history.
* Entity lifecycle tracking.
* Historical observations.
* Change detection.
* Historical exploration.

### 5. AI Intelligence

An evidence-grounded intelligence layer.

* Ask questions about your digital environment.
* Explain relationships between entities.
* Investigate system information.
* Summarize discovered information.
* Suggest potential relationships without automatically treating them as verified facts.

### 6. Workspaces

Organize exploration and investigations into persistent workspaces.

### 7. Security Center

Control how ATLAS interacts with the local system.

* Granular permissions.
* Explicit background monitoring controls.
* Permission management.
* Auditable operations.
* User-controlled access.

---

## 🛠️ Technology Stack

| Component           | Technology       |
| ------------------- | ---------------- |
| Desktop Framework   | Tauri 2          |
| Frontend            | React            |
| Language            | TypeScript       |
| Frontend Validation | Zod              |
| Build Tool          | Vite             |
| Backend             | Rust             |
| Database            | SQLite (planned) |
| Testing — Frontend  | Vitest           |
| Testing — Rust      | Cargo Test       |
| Styling             | CSS              |

### Architecture

ATLAS follows a modular architecture.

```text
ATLAS
│
├── Frontend
│   ├── React
│   ├── TypeScript
│   └── UI Components
│
├── Tauri Bridge
│   └── Frontend ↔ Rust IPC
│
└── Rust Backend
    │
    ├── Runtime Host
    │
    ├── Mirror Engine
    │   └── System Discovery
    │
    ├── Graph Engine
    │   └── Entities & Relationships
    │
    ├── History Engine
    │   └── Historical Observations
    │
    ├── Intelligence Engine
    │   └── AI Integration
    │
    └── Permissions
        └── Access Management
```

*This represents the intended architecture. Not every module has been implemented yet.*

---

## 🏗️ Development Progress

ATLAS is currently in early development.

### Phase 1 — Foundation ✅

* Tauri desktop application setup.
* React and TypeScript frontend.
* Rust backend integration.
* Frontend-to-backend IPC.
* System information command.
* Basic application interface.
* Frontend and backend tests.

### Phase 2 — Runtime Infrastructure 🟡

Implemented:

* Runtime Core.
* Runtime Lifecycle Manager.
* Runtime Configuration Manager.
* Configuration loading and saving.
* Permission Manager.
* RuntimeHost integration with permissions.
* Permission state serialization.
* Permission persistence in configuration.

Currently working on:

* PermissionManager persistence integration.
* RuntimeHost restoration of saved permission decisions.
* Additional persistence and integration tests.

**Current test status:** 31 Rust tests passing at the last verified checkpoint.

### Upcoming Phases

| Phase | Component                 | Status  |
| ----- | ------------------------- | ------- |
| 3     | Mirror Engine             | Planned |
| 4     | Graph Engine              | Planned |
| 5     | Universe UI — 2D          | Planned |
| 6     | History Engine            | Planned |
| 7     | Spatial Explorer — 3D     | Planned |
| 8     | AI Intelligence           | Planned |
| 9     | Stabilization & Packaging | Planned |

The first major product milestone is a working 2D Digital Universe Explorer.

---

## 🔐 Design Principles

ATLAS is being developed around several principles:

**Understand before acting**
Users should understand their digital environment before making changes.

**Relationships over isolated data**
The application should explain how digital components connect.

**Evidence over assumptions**
Discovered facts, deterministic inferences, and AI suggestions must remain distinguishable.

**Local-first**
Core functionality should work locally without depending on cloud services.

**Intelligence without dependency**
AI should improve understanding, not become necessary for basic functionality.

**Privacy and user control**
Permissions and background monitoring should remain under explicit user control.

---

## 📁 Project Structure

Current project structure:

```text
ATLAS/
│
├── src/
│   ├── App.tsx
│   ├── contracts.ts
│   ├── main.tsx
│   ├── styles.css
│   ├── App.test.tsx
│   ├── contracts.test.ts
│   └── test/
│       └── setup.ts
│
├── src-tauri/
│   ├── src/
│   │   ├── lib.rs
│   │   ├── main.rs
│   │   └── runtime/
│   │       ├── config/
│   │       │   ├── manager.rs
│   │       │   ├── mod.rs
│   │       │   └── schema.rs
│   │       ├── error.rs
│   │       ├── host.rs
│   │       ├── lifecycle.rs
│   │       ├── mod.rs
│   │       ├── permissions.rs
│   │       └── state.rs
│   │
│   ├── capabilities/
│   ├── icons/
│   ├── Cargo.toml
│   ├── Cargo.lock
│   ├── build.rs
│   └── tauri.conf.json
│
├── index.html
├── package.json
├── package-lock.json
├── tsconfig.json
├── vite.config.ts
├── .gitignore
└── README.md
```

The structure will evolve as additional engines are implemented.

---

## 💻 Getting Started

### Prerequisites

* Node.js
* npm
* Rust toolchain
* Tauri 2 development prerequisites for your operating system.

### Installation

Clone the repository:

```bash
git clone <repository-url>
cd ATLAS
```

Install frontend dependencies:

```bash
npm install
```

Run the desktop application:

```bash
npm run tauri dev
```

### Testing

Frontend tests:

```bash
npm test
```

Frontend production build:

```bash
npm run build
```

Rust tests:

```bash
cd src-tauri
cargo test
```

Rust compilation check:

```bash
cargo check
```

---

## 🗺️ Long-Term Direction

ATLAS aims to evolve beyond traditional system monitoring and file management.

Its goal is to create an explorable representation of the user's digital environment, where information is connected, relationships are understandable, and complexity becomes easier to navigate.

The Digital Universe is the foundation of that experience.

---

## 📌 Project Status

**Development Stage:** Early Development
**Current Phase:** Phase 2 — Runtime Infrastructure
**Target Platform:** Windows first
**Architecture Direction:** Cross-platform
**Development Approach:** Modular, local-first, evidence-driven

ATLAS is an actively developed project. Features described in the roadmap represent planned capabilities and should not be interpreted as already implemented.

---

*ATLAS — Explore your digital universe.*

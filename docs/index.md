---
layout: home

hero:
  name: MCFC
  text: Typed Minecraft datapacks without giving up vanilla.
  tagline: A statically typed language, compiler, and language server for building Minecraft 26.2 datapacks from .mcf source files.
  image:
    src: /MCFC-icon.png
    alt: MCFC icon
  actions:
    - theme: brand
      text: Get Started
      link: /guide/getting-started
    - theme: alt
      text: Language Guide
      link: /language/overview

features:
  - title: Datapack-native
    details: MCFC emits vanilla datapack files, including pack.mcmeta, generated functions, tags, and optional public wrappers.
  - title: Typed gameplay code
    details: Write functions, structs, state, selectors, inventory operations, bossbars, text components, and NBT-aware builder code with compiler diagnostics.
  - title: Async Minecraft workflows
    details: Use async blocks, sleep, sleep_ticks, tick functions, and scheduled generated functions for non-blocking gameplay flows.
  - title: Optional host bridge
    details: mcfd lets vanilla datapacks opt into HTTP, files, key-value storage, SQLite, real time, and randomness through manifest-gated capabilities.
  - title: Editor support
    details: The VS Code extension bundles syntax highlighting, project commands, manifest tooling, and a Rust language server.
  - title: Experimental agent hooks
    details: mcfd-agent can add version-pinned Minecraft 26.2 event callbacks and root commands while keeping the vanilla fallback intact.
---

## Start With A Project

```powershell
cargo run -- new my-pack --helper none
cargo run -- build my-pack --clean
```

MCFC is early-stage software. Generated output and language features are still evolving, so pin a commit when using it for a real pack.

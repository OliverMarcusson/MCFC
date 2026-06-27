# Bukkit API Conformance

`examples/bukkit_api_conformance` is a safe, vanilla-first smoke test for MCFC's Bukkit-inspired declarations. `mcfd` is optional but needed for time, random, and KV checks.

The pack also requests the optional JVM agent. This is safe when the agent is unavailable; the datapack keeps its vanilla behavior.

## Build

```powershell
cargo run --bin mcfc -- build examples/bukkit_api_conformance/mcfc.toml --clean
```

## Run

Install `dist/` as a normal datapack, reload, then run:

```text
/function bukkit_api_conformance:run_all
/function bukkit_api_conformance:report
/trigger mcfcc_status
/function bukkit_api_conformance:cleanup
```

The generated runtime verifies `data player.*`, join and death dispatch, load tasks, trigger commands, optional agent root commands, UI and bossbar APIs, host calls, and typed agent callbacks.

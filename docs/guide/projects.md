# Projects

MCFC projects are configured with `mcfc.toml` or `*.mcfc.toml`. A project build merges all `.mcf` files from the source directory in deterministic order and copies assets into the generated datapack.

## Basic Manifest

```toml
namespace = "my_pack"
source_dir = "src"
asset_dir = "assets"
out_dir = "dist"
```

Fields:

- `namespace`: datapack namespace
- `source_dir`: directory containing `.mcf` files, default `src`
- `asset_dir`: files copied into the datapack, default `assets`
- `out_dir`: default output directory for project builds
- `load`: additional generated load tag functions
- `tick`: additional generated tick tag functions
- `[[export]]`: mappings from datapack paths to MCFC functions

## Exports

Use exports when a function needs a specific datapack path:

```toml
[[export]]
path = "data/my_pack/function/run_all.mcfunction"
function = "run_all"
```

## Helper Runtime

Host capabilities are enabled through the `[helper]` table:

```toml
[helper]
backend = "mcfd"

[helper.capabilities]
http = { allow_domains = ["api.example.com"] }
file = { root = "./host_data" }
kv = { root = "./host_data/kv" }
db = { path = "./host_data/data.sqlite" }
time = true
rand = true
```

Agent-backed events and root commands are requested separately:

```toml
[helper.agent]
enabled = true
events = ["player_damage"]
commands = ["home"]
```

::: warning Capability gating
A `module.fn(...)` host call is a compile error unless the matching capability is enabled in the project manifest.
:::

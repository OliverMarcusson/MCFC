# Projects

MCFC projects are configured with `mcfc.toml` or `*.mcfc.toml`. A project build compiles `main.mcf` in the source directory, plus every file it reaches through `mod` declarations, and copies assets into the generated datapack.

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

## Multiple Source Files

`src/main.mcf` is the root module. Split code into more files with [`mod`](../language/reference/statements#mod-and-pub) and import from them with [`use`](../language/reference/statements#use):

```text
src/
  main.mcf          # mod util
  util.mcf          # pub fn double(x: int) -> int
  combat/
    mod.mcf         # pub mod damage
    damage.mcf
```

A `.mcf` file that no `mod` declaration reaches is left out of the build, and the build prints a warning naming it. Errors name the file and line they come from:

```text
error:src/util.mcf:3:5: cannot find function 'greet' in module 'util'; it is defined in the root module, so import it with 'use greet'
```

To export a module function at a chosen path, use its full name: `function = "util::announce"`.

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
A `module.fn(...)` host call is a compile error unless the matching capability is enabled in the project manifest. See [Capabilities](/runtime/capabilities) for every call and [mcfd](/runtime/mcfd) for installing the helper.
:::

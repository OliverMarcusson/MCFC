# Projects

MCFC projects are configured with `mcfc.toml` or `*.mcfc.toml`. A project build compiles every `.mcf` file in the source directory and copies assets into the generated datapack.

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
- `load`: functions for the `load:load` tag, replacing `main`
- `tick`: additional generated tick tag functions
- `[[export]]`: mappings from datapack paths to MCFC functions

## Multiple Source Files

Every `.mcf` file under `src/` is a module named by its path: `src/util.mcf` is `util`, and `src/combat/damage.mcf` is `combat.damage`. `src/main.mcf` is the root module.

```text
src/
  main.mcf          # import util.twice;
  util.mcf          # public int twice(int x) { ... }
  combat/
    damage.mcf      # combat.damage
```

Mark an item `public` to use it outside its module and that module's children. Call it by path (`util.twice(2)`) or bring it into scope with [`import`](../language/reference/statements#import). Errors name the file and line they come from:

```text
error:src/main.mcf:5:18: function 'announce' is private to module 'util'; mark it 'public'
```

To export a module function at a chosen path, use its full name: `function = "util.announce"`.

## Exports

Use exports when a function needs a specific datapack path:

```toml
[[export]]
path = "data/my_pack/function/run_all.mcfunction"
function = "run_all"
```

## Helper Runtime <Badge type="danger" text="mcfd" title="Needs the mcfd helper running beside the server. Not available on Realms." />

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

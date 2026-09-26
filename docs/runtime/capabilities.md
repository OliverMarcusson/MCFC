# Capabilities

Each host module has to be enabled in `mcfc.toml`. Calling a module that isn't enabled is a compile error.

```toml
[helper]
backend = "mcfd"

[helper.capabilities]
http = { allow_domains = ["api.example.com"], bearer_token_env = "MY_API_TOKEN" }
file = { root = "./host_data" }
kv = { root = "./host_data/kv" }
db = { path = "./host_data/data.sqlite" }
time = true
rand = true
```

Every call pauses the function until `mcfd` answers (see [Functions that pause](/language/reference/statements#functions-that-pause)) and returns a struct with `ok: bool`. If `mcfd` isn't running, the call resumes after a timeout with `ok = false`.

## Calls

| Call | Returns fields besides `ok` |
| --- | --- |
| `http.get(url)` | `status: int`, `body: string`, `err: string` |
| `http.post(url, body)` | `status`, `body`, `err` |
| `http.get_json_string(url, path)` | `status`, `body` (the string at `path`), `err` |
| `http.get_json_strings(url, paths: array<string>)` | `status`, `values: nbt` (one string per path), `err` |
| `file.read(path)` | `content: string` |
| `file.write(path, content)` | |
| `kv.get(key)` | `value: string` |
| `kv.set(key, value)` | |
| `db.exec(sql, params: array<string>)` | `rows_affected: int` |
| `db.query(sql, params: array<string>)` | `rows_affected: int`, `rows: nbt`, a list of rows where each row maps column name to string |
| `time.now()` | `unix: int`, `iso: string` |
| `rand.int(min, max)` | `value: int` |
| `mcfd.ping()` | `pong: bool`. Always available, useful for checking the connection. |

JSON paths are dot-separated, such as `quote.author.name`. The JSON helpers set `ok = false` for non-2xx responses, invalid JSON, missing paths and non-string values.

```mcfc
fn top_player(team: string) -> void:
    let r = db.query("SELECT name FROM scores WHERE team = ? ORDER BY points DESC LIMIT 1", [team])
    if r.ok:
        let name = string(r.rows[0].name)
        selector("@a").tellraw("Top player: $(name)")
```

## Scope

- `http` only reaches domains in `allow_domains`. With `bearer_token_env`, `mcfd` reads a token from that environment variable and sends it as `Authorization: Bearer ...`. Only the variable's name goes into the datapack.
- `file` and `kv` paths are relative to their `root`. They can't escape it.
- `db` opens a single SQLite file. Pass values through `params` (`?` placeholders) rather than joining them into the SQL string.

# Capabilities <Badge type="danger" text="mcfd" title="Needs the mcfd helper running beside the server. Not available on Realms." />

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

Every call pauses the method until `mcfd` answers (see [Methods that pause](/language/reference/statements#methods-that-pause)) and returns a struct with `ok: boolean`. If `mcfd` isn't running, the call resumes after a timeout with `ok = false`.

## Calls

| Call | Returns fields besides `ok` |
| --- | --- |
| `http.get(url)` | `status: int`, `body: String`, `err: String` |
| `http.post(url, body)` | `status`, `body`, `err` |
| `http.get_json_string(url, path)` | `status`, `body` (the string at `path`), `err` |
| `http.get_json_strings(url, paths: List<String>)` | `status`, `values: Nbt` (one string per path), `err` |
| `file.read(path)` | `content: String` |
| `file.write(path, content)` | |
| `kv.get(key)` | `value: String` |
| `kv.set(key, value)` | |
| `db.exec(sql, params: List<String>)` | `rowsAffected: int` |
| `db.query(sql, params: List<String>)` | `rowsAffected: int`, `rows: Nbt`, a list of rows where each row maps column name to string |
| `time.now()` | `unix: int`, `iso: String` |
| `rand.int(min, max)` | `value: int` |
| `mcfd.ping()` | `pong: boolean`. Always available, useful for checking the connection. |

JSON paths are dot-separated, such as `quote.author.name`. The JSON helpers set `ok = false` for non-2xx responses, invalid JSON, missing paths and non-string values.

```mcfc
class Main {
    static void topPlayer(String team) {
        var r = db.query("SELECT name FROM scores WHERE team = ? ORDER BY points DESC LIMIT 1", List.of(team));
        if (r.ok()) {
            var name = (String) r.rows()[0].name;
            Selector.of("@a").sendMessage("Top player: $(name)");
        }
    }
}
```

## Scope

- `http` only reaches domains in `allow_domains`. With `bearer_token_env`, `mcfd` reads a token from that environment variable and sends it as `Authorization: Bearer ...`. Only the variable's name goes into the datapack.
- `file` and `kv` paths are relative to their `root`. They can't escape it.
- `db` opens a single SQLite file. Pass values through `params` (`?` placeholders) rather than joining them into the SQL string.

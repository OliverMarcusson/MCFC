# Capabilities

Host modules are enabled per project.

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

Every response carries `ok`. If the helper is unreachable, the call resumes after a timeout with `ok = false`.

## HTTP

Available calls:

- `http.get(url)`
- `http.post(url, body)`
- `http.get_json_string(url, path)`
- `http.get_json_strings(url, paths)`

JSON helper paths use dot-separated object paths such as `quote.text`. JSON helpers set `ok = false` for non-2xx responses, malformed JSON, missing paths, or non-string values.

## Files And KV

Available calls:

- `file.read(path)`
- `file.write(path, content)`
- `kv.get(key)`
- `kv.set(key, value)`

File and KV access is rooted under the configured directories.

## Database

Available calls:

- `db.exec(sql, params)`
- `db.query(sql, params)`

The database capability uses the configured SQLite file.

## Time And Random

Available calls:

- `time.now()`
- `rand.int(min, max)`

Use these when deterministic vanilla datapack behavior is not enough and the project explicitly opts into host assistance.

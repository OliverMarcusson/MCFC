# Examples

Complete projects in [`examples/`](https://github.com/OliverMarcusson/MCFC/tree/main/examples). Build any of them with:

```powershell
mcfc build examples/<name> --clean
```

Then copy `examples/<name>/dist` into `<world>/datapacks/` and run `/reload`. Examples that use `mcfd` also need [mcfd](/runtime/mcfd) running.

| Example | Shows | Needs |
| --- | --- | --- |
| [Oracle](#oracle) | HTTP, time, helper randomness, player state, an endless `Thread.start` loop | `mcfd` |
| [RPC demo](#rpc-demo) | The smallest possible host call | `mcfd` |
| [Cyber Quotes](#cyber-quotes) | JSON extraction, bearer tokens, formatted text | `mcfd`, API token |
| [Feature demo](#feature-demo) | A self-checking tour of floats, strings, lists, std, generics, world reads | nothing |
| [Bukkit API conformance](#bukkit-api-conformance) | Events, commands, tasks, UI, agent callbacks | `mcfd` (optional), agent (optional) |

## Oracle <Badge type="danger" text="mcfd" title="Needs the mcfd helper running beside the server. Not available on Realms." />

Every 15 seconds, fetches a random number from random.org and announces it with the real time. `/function oracle:roll` rolls a die using `rand`.

<<< @/../examples/oracle/src/main.mcf{mcfc}

<<< @/../examples/oracle/mcfc.toml{toml}

If `mcfd` isn't running, the requests time out and the Oracle reports that it's silent.

## RPC demo <Badge type="danger" text="mcfd" title="Needs the mcfd helper running beside the server. Not available on Realms." />

<<< @/../examples/rpc_demo/src/main.mcf{mcfc}

Only `api.example.com` is on the allow list, and `mcfd` rejects requests to any other domain.

## Cyber Quotes <Badge type="danger" text="mcfd" title="Needs the mcfd helper running beside the server. Not available on Realms." />

`/function cyber_quotes:quote` fetches a quote and pulls three JSON fields from the same response. `/function cyber_quotes:health` checks the connection to `mcfd` first.

The API needs a token. Put it in `examples/cyber_quotes/assets/.env`, which the build copies next to `mcfd.pack.toml`:

```text
MUNIN_EVENTS_API_SECRET=<token>
```

The manifest names only the variable (`bearer_token_env`), so the secret never ends up in the datapack.

::: details Source
<<< @/../examples/cyber_quotes/src/main.mcf{mcfc}
:::

## Feature demo

`examples/feature_demo_263` runs a self-check the first time a player is online. It prints `MCFCTEST PASS|FAIL <name>` lines with `/say`, so the results end up in `latest.log`. Run it again with `/function mcfc_demo:run_tests`.

::: details Source
<<< @/../examples/feature_demo_263/src/main.mcf{mcfc}
:::

## Bukkit API conformance

A smoke test for the declaration features. After loading, run:

```text
/function bukkit_api_conformance:run_all
/function bukkit_api_conformance:report
/trigger status
/function bukkit_api_conformance:cleanup
```

The time, random and KV checks need `mcfd`. The agent callbacks only run when the agent is attached. Without it the pack still loads.

::: details Source
<<< @/../examples/bukkit_api_conformance/src/main.mcf{mcfc}
:::

## Other folders

`examples/debug_function_log` is a hand-written datapack for testing the log transport that `mcfd` reads. It's for developing `mcfd`, not for learning MCFC.

# mcfd-agent <Badge type="danger" text="Agent" title="Needs mcfd-agent running beside the server. Not available on Realms." />

`mcfd-agent` is an optional Java agent. It adds the [agent events](/language/reference/events#agent-events), such as chat, block break and interactions, some of them cancellable, and registers real `/name` commands for `@Command` handlers. It isn't a mod or a plugin. `mcfd` attaches it to the running Minecraft process.

Without the agent, a pack that uses agent events still loads. Its agent handlers don't run, and commands still work through `/trigger`.

## Enable it

1. Request the agent in `mcfc.toml`:

   ```toml
   [helper]
   backend = "mcfd"

   [helper.agent]
   enabled = true
   ```

   `@EventHandler` and `@Command` handlers are subscribed automatically.

2. Build the agent. This needs a JDK:

   ```powershell
   .\mcfd-agent\build.ps1
   ```

3. Copy `mcfd-agent\dist\mcfd-agent.jar` and `mcfd-agent-attach.jar` into the folder that holds `mcfd.exe`. After `cargo install`, that's `~/.cargo/bin`. The Windows installer from `package-mcfd.ps1` already includes them.

4. Start Minecraft, then check the agent:

   ```powershell
   mcfd agent status
   ```

   `mcfd` attaches to the running Minecraft process automatically. If it can't tell which process to use, attach by hand with `mcfd agent attach <pid>`.

The agent only works with Minecraft 26.3. After updating it, restart Minecraft so `mcfd` attaches the new JAR.

## How it works

The agent patches vanilla server methods. When an event fires, it logs a `[mcfd-agent] event=...` line followed by a JSON record, and `mcfd` picks that up. The matching MCFC handler then runs on the server thread as the affected player. Some events arrive as network packets before the server acts on them. For those, a handler can call `event.cancel()` to stop the action.

The hook sites call the agent only through JDK types, so it also works under mod loaders such as Fabric.

## Developing the agent

```powershell
.\mcfd-agent\verify-26.3.ps1   # check hook targets against a 26.3 JAR
.\mcfd-agent\test.ps1          # dispatch and command-routing self-test
```

package dev.mcfc.agent;

import java.util.Collections;
import java.util.LinkedHashMap;
import java.util.LinkedHashSet;
import java.util.Map;
import java.util.Set;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicBoolean;

/** Runtime policy shared by bytecode-injected hook sites. */
public final class McfdHooks {
    private static volatile Set<String> cancelled = Collections.emptySet();
    private static volatile Map<String, Set<String>> routes = Collections.emptyMap();
    private static volatile Map<String, Set<String>> commandRoutes = Collections.emptyMap();
    private static volatile Map<String, Set<String>> decisionRoutes = Collections.emptyMap();
    private static final ThreadLocal<Boolean> deciding = new ThreadLocal<>();

    private McfdHooks() {
    }

    static void configure(String args) {
        Set<String> values = new LinkedHashSet<>();
        Map<String, Set<String>> configuredRoutes = new LinkedHashMap<>();
        Map<String, Set<String>> configuredCommands = new LinkedHashMap<>();
        Map<String, Set<String>> configuredDeciders = new LinkedHashMap<>();
        if (args != null) {
            for (String part : args.split(";")) {
                if (part.startsWith("cancel=")) {
                    for (String value : part.substring("cancel=".length()).split(",")) {
                        String normalized = value.trim().toLowerCase();
                        if (!normalized.isEmpty()) {
                            values.add(normalized);
                        }
                    }
                } else if (part.startsWith("routes=")) {
                    parseRoutes(part.substring("routes=".length()), configuredRoutes);
                } else if (part.startsWith("commands=")) {
                    parseRoutes(part.substring("commands=".length()), configuredCommands);
                } else if (part.startsWith("deciders=")) {
                    parseRoutes(part.substring("deciders=".length()), configuredDeciders);
                }
            }
        }
        cancelled = Collections.unmodifiableSet(values);
        Map<String, Set<String>> immutableRoutes = new LinkedHashMap<>();
        for (Map.Entry<String, Set<String>> entry : configuredRoutes.entrySet()) {
            immutableRoutes.put(entry.getKey(), Collections.unmodifiableSet(entry.getValue()));
        }
        routes = Collections.unmodifiableMap(immutableRoutes);
        Map<String, Set<String>> immutableCommands = new LinkedHashMap<>();
        for (Map.Entry<String, Set<String>> entry : configuredCommands.entrySet()) {
            immutableCommands.put(entry.getKey(), Collections.unmodifiableSet(entry.getValue()));
        }
        commandRoutes = Collections.unmodifiableMap(immutableCommands);
        Map<String, Set<String>> immutableDeciders = new LinkedHashMap<>();
        for (Map.Entry<String, Set<String>> entry : configuredDeciders.entrySet()) {
            immutableDeciders.put(entry.getKey(), Collections.unmodifiableSet(entry.getValue()));
        }
        decisionRoutes = Collections.unmodifiableMap(immutableDeciders);
    }

    static String describe() {
        String policy = cancelled.isEmpty() ? "observe-only" : "cancel=" + String.join(",", cancelled);
        String routeDescription = routes.isEmpty() ? "" : "; routes=" + routes.keySet();
        String commandDescription = commandRoutes.isEmpty() ? "" : "; commands=" + commandRoutes.keySet();
        return policy + routeDescription + commandDescription;
    }

    public static boolean before(String event, Object source, Object payload) {
        boolean synchronous = decisionRoutes.containsKey(event);
        boolean cancel = synchronous && decide(event, source, payload);
        log(event, source, payload, cancel, true);
        if (!synchronous) {
            dispatch(event, source, payload, cancel);
        }
        return cancel;
    }

    /**
     * Execute explicit event.cancel() handlers on the server thread and return
     * their decision to the packet hook. This deliberately fails open after
     * 25 ms; a Minecraft packet must never wait indefinitely for a datapack.
     */
    private static boolean decide(String event, Object source, Object payload) {
        Set<String> namespaces = decisionRoutes.get(event);
        if (namespaces == null || namespaces.isEmpty()) {
            return false;
        }
        if (Boolean.TRUE.equals(deciding.get())) {
            System.err.println("[mcfd-agent] decision event=" + event + " outcome=reentrant");
            return false;
        }
        Object player = playerFor(source);
        Object server = fieldValue(player, "server");
        if (player == null || server == null) {
            System.err.println("[mcfd-agent] decision event=" + event + " outcome=unavailable");
            return false;
        }
        final long started = System.nanoTime();
        final AtomicBoolean completed = new AtomicBoolean(false);
        final AtomicBoolean result = new AtomicBoolean(false);
        Runnable work = () -> {
            if (!completed.compareAndSet(false, true)) return;
            deciding.set(Boolean.TRUE);
            try {
                String playerName = stringValue(player, "getScoreboardName");
                String data = eventData(event, source, payload, false, playerName);
                for (String namespace : namespaces) {
                    runCommand(server, server, "data modify storage " + namespace + ":agent decision set value {cancel:0b}");
                    dispatchOnServer(server, player, namespace, event, data);
                    result.set(result.get() || readDecision(server, namespace));
                }
                System.err.println("[mcfd-agent] decision event=" + event + " outcome="
                        + (result.get() ? "cancel" : "allow") + " elapsed_us=" + ((System.nanoTime() - started) / 1000));
            } catch (Throwable error) {
                System.err.println("[mcfd-agent] decision event=" + event + " outcome=error error=" + error);
                result.set(false);
            } finally {
                deciding.remove();
            }
        };
        if (isServerThread(server)) {
            work.run();
            return result.get();
        }
        CountDownLatch latch = new CountDownLatch(1);
        Runnable queued = () -> { work.run(); latch.countDown(); };
        invokeExecute(server, queued);
        try {
            if (!latch.await(25, TimeUnit.MILLISECONDS)) {
                completed.set(true);
                System.err.println("[mcfd-agent] decision event=" + event + " outcome=timeout elapsed_us=" + ((System.nanoTime() - started) / 1000));
                return false;
            }
        } catch (InterruptedException interrupted) {
            Thread.currentThread().interrupt();
            completed.set(true);
            return false;
        }
        return result.get();
    }

    private static boolean isServerThread(Object server) {
        try {
            for (java.lang.reflect.Method method : server.getClass().getMethods()) {
                if (method.getName().equals("isSameThread") && method.getParameterCount() == 0) {
                    return Boolean.TRUE.equals(method.invoke(server));
                }
            }
        } catch (Throwable ignored) { }
        return false;
    }

    private static boolean readDecision(Object server, String namespace) {
        try {
            Object storage = invokeNoArgs(server, "getCommandStorage");
            Class<?> idClass = Class.forName("net.minecraft.resources.ResourceLocation");
            Object id = idClass.getMethod("parse", String.class).invoke(null, namespace + ":agent");
            Object root = storage.getClass().getMethod("get", idClass).invoke(storage, id);
            Object decision = root.getClass().getMethod("getCompound", String.class).invoke(root, "decision");
            return Boolean.TRUE.equals(decision.getClass().getMethod("getBoolean", String.class).invoke(decision, "cancel"));
        } catch (Throwable error) {
            System.err.println("[mcfd-agent] decision storage read failed namespace=" + namespace + " error=" + error);
            return false;
        }
    }

    /** Emit an observation-only event. It is deliberately unable to cancel. */
    public static void observe(String event, Object source, Object payload) {
        log(event, source, payload, false, false);
        dispatch(event, source, payload, false);
    }

    /**
     * Intercept only configured MCFC command roots. Unknown and vanilla roots
     * return false and therefore continue through Minecraft's normal dispatcher.
     */
    public static boolean handleCommand(Object source, Object payload) {
        String command = stringValue(payload, "command");
        String root = command.trim().split("\\s+", 2)[0].toLowerCase();
        Set<String> namespaces = commandRoutes.get(root);
        if (namespaces == null || namespaces.isEmpty()) {
            return false;
        }
        log("command", source, payload, true, true);
        Object player = playerFor(source);
        Object server = fieldValue(player, "server");
        if (player == null || server == null) {
            System.err.println("[mcfd-agent] command dispatch skipped root=" + root + " (no server player context)");
            return false;
        }
        for (String namespace : namespaces) {
            invokeExecute(server, () -> dispatchCommandOnServer(server, player, namespace, root, command));
        }
        return true;
    }

    private static void log(String event, Object source, Object payload, boolean cancelled, boolean cancellable) {
        String sourceName = typeName(source);
        String payloadName = typeName(payload);
        // Keep the compact prefix useful in latest.log while exposing a
        // versioned record that mcfd can parse without relying on log wording.
        System.err.println("[mcfd-agent] event=" + event
                + " source=" + sourceName
                + " payload=" + payloadName
                + " cancelled=" + cancelled
                + " record={\"protocol\":1,\"event\":" + json(event)
                + ",\"source\":" + json(sourceName)
                + ",\"payload\":" + json(payloadName)
                + ",\"cancelled\":" + cancelled
                + ",\"cancellable\":" + cancellable + "}");
    }

    private static String typeName(Object value) {
        return value == null ? "null" : value.getClass().getSimpleName();
    }

    private static String json(String value) {
        return "\"" + value.replace("\\", "\\\\").replace("\"", "\\\"") + "\"";
    }

    /**
     * Route subscribed events back into generated datapack functions. The hook
     * can run on a Netty thread, so all Minecraft command work is first queued
     * onto the server executor. Reflection avoids linking this small agent to
     * Minecraft classes at build time and keeps the adapter version-pinned.
     */
    private static void dispatch(String event, Object source, Object payload, boolean wasCancelled) {
        Set<String> namespaces = routes.get(event);
        if (namespaces == null || namespaces.isEmpty()) {
            return;
        }
        Object player = playerFor(source);
        Object server = fieldValue(player, "server");
        if (player == null || server == null) {
            System.err.println("[mcfd-agent] dispatch skipped event=" + event + " (no server player context)");
            return;
        }
        final String playerName = stringValue(player, "getScoreboardName");
        final String data = eventData(event, source, payload, wasCancelled, playerName);
        for (String namespace : namespaces) {
            invokeExecute(server, () -> dispatchOnServer(server, player, namespace, event, data));
        }
    }

    private static Object playerFor(Object source) {
        if (source == null) {
            return null;
        }
        String name = source.getClass().getName();
        if ("net.minecraft.server.level.ServerPlayer".equals(name)) {
            return source;
        }
        return fieldValue(source, "player");
    }

    private static void dispatchOnServer(Object server, Object player, String namespace, String event, String data) {
        try {
            runCommand(server, server, "data modify storage " + namespace + ":agent current set value " + data);
            if ("player_quit".equals(event)) {
                runCommand(server, server, "function " + namespace + ":agent/event/" + event);
                return;
            }
            String playerName = stringValue(player, "getScoreboardName");
            if (playerName.isEmpty() || !playerName.matches("[A-Za-z0-9_]{1,16}")) {
                throw new IllegalStateException("invalid player scoreboard name for event dispatch");
            }
            // Execute from the server source so the function command has the
            // required permission, but switch the function context to the
            // player so `event.player` and `@s` retain Bukkit-like semantics.
            runCommand(server, server,
                    "execute as @a[name=" + playerName + "] run function "
                            + namespace + ":agent/event/" + event);
        } catch (Throwable error) {
            System.err.println("[mcfd-agent] dispatch failed event=" + event + " namespace=" + namespace + ": " + error);
        }
    }

    private static void dispatchCommandOnServer(
            Object server, Object player, String namespace, String root, String command) {
        try {
            String playerName = stringValue(player, "getScoreboardName");
            String[] parts = command.trim().split("\\s+");
            StringBuilder args = new StringBuilder("[");
            for (int index = 1; index < parts.length; index++) {
                if (index > 1) args.append(',');
                args.append(snbtString(parts[index]));
            }
            args.append(']');
            String selector = playerName.matches("[A-Za-z0-9_]{1,16}")
                    ? "@a[name=" + playerName + "]" : "@s";
            runCommand(server, server,
                    "data modify storage " + namespace + ":agent command set value {command:"
                            + snbtString(command) + ",sender:{kind:\"player\",name:"
                            + snbtString(playerName) + ",permission_level:0,player:{prefix:\"\",selector:"
                            + snbtString(selector) + "}},args:" + args + "}");
            if (playerName.isEmpty() || !playerName.matches("[A-Za-z0-9_]{1,16}")) {
                throw new IllegalStateException("invalid player scoreboard name for command dispatch");
            }
            runCommand(server, server,
                    "execute as @a[name=" + playerName + "] run function "
                            + namespace + ":agent/command/" + root);
        } catch (Throwable error) {
            System.err.println("[mcfd-agent] command dispatch failed root=" + root
                    + " namespace=" + namespace + ": " + error);
        }
    }

    private static void invokeExecute(Object server, Runnable runnable) {
        try {
            for (java.lang.reflect.Method method : server.getClass().getMethods()) {
                if (method.getName().equals("execute") && method.getParameterCount() == 1) {
                    method.invoke(server, runnable);
                    return;
                }
            }
            System.err.println("[mcfd-agent] dispatch skipped (MinecraftServer.execute unavailable)");
        } catch (Throwable error) {
            System.err.println("[mcfd-agent] could not queue datapack event: " + error);
        }
    }

    private static void runCommand(Object server, Object sender, String command) throws Exception {
        Object commands = invokeNoArgs(server, "getCommands");
        Object source = invokeNoArgs(sender, "createCommandSourceStack");
        for (java.lang.reflect.Method method : commands.getClass().getMethods()) {
            if (method.getName().equals("performPrefixedCommand") && method.getParameterCount() == 2) {
                method.invoke(commands, source, command);
                return;
            }
        }
        throw new NoSuchMethodException("Commands.performPrefixedCommand");
    }

    private static Object invokeNoArgs(Object target, String name) throws Exception {
        for (java.lang.reflect.Method method : target.getClass().getMethods()) {
            if (method.getName().equals(name) && method.getParameterCount() == 0) {
                return method.invoke(target);
            }
        }
        throw new NoSuchMethodException(target.getClass().getName() + "." + name);
    }

    private static Object fieldValue(Object target, String name) {
        if (target == null) {
            return null;
        }
        for (Class<?> type = target.getClass(); type != null; type = type.getSuperclass()) {
            try {
                java.lang.reflect.Field field = type.getDeclaredField(name);
                if (!field.trySetAccessible()) {
                    return null;
                }
                return field.get(target);
            } catch (NoSuchFieldException ignored) {
                // Look in the superclass next.
            } catch (Throwable error) {
                return null;
            }
        }
        return null;
    }

    private static String eventData(
            String event, Object source, Object payload, boolean cancelled, String playerName) {
        String selector = playerName.matches("[A-Za-z0-9_]{1,16}")
                ? "@a[name=" + playerName + "]" : "@s";
        String base = "{player:{prefix:\"\",selector:" + snbtString(selector) + "},player_name:" + snbtString(playerName)
                + ",source:" + snbtString(typeName(source))
                + ",payload:" + snbtString(typeName(payload))
                + ",cancelled:" + (cancelled ? "1b" : "0b");
        if ("chat".equals(event)) {
            return base + ",message:" + snbtString(stringValue(payload, "message")) + "}";
        }
        if ("inventory_click".equals(event)) {
            return base
                    + ",container_id:" + intValue(payload, "containerId")
                    + ",state_id:" + intValue(payload, "stateId")
                    + ",slot:" + intValue(payload, "slotNum")
                    + ",button:" + intValue(payload, "buttonNum") + "}";
        }
        if ("player_interact_block".equals(event)) {
            Object hit = invokeQuietly(payload, "getHitResult");
            Object position = invokeQuietly(hit, "getBlockPos");
            return base
                    + ",hand:" + snbtString(enumName(invokeQuietly(payload, "getHand")))
                    + ",face:" + snbtString(enumName(invokeQuietly(hit, "getDirection")))
                    + positionFields(position) + "}";
        }
        if ("player_interact_item".equals(event) || "player_swing".equals(event)) {
            return base + ",hand:" + snbtString(enumName(invokeQuietly(payload, "getHand"))) + "}";
        }
        if ("entity_interact".equals(event)) {
            return base
                    + ",target_id:" + intValue(payload, "entityId")
                    + ",hand:" + snbtString(enumName(invokeQuietly(payload, "hand")))
                    + ",secondary:" + boolValue(payload, "usingSecondaryAction") + "}";
        }
        if ("entity_attack".equals(event)) {
            return base + ",target_id:" + intValue(payload, "entityId") + "}";
        }
        if ("item_held_change".equals(event)) {
            return base + ",slot:" + intValue(payload, "getSlot") + "}";
        }
        if ("inventory_close".equals(event)) {
            return base + ",container_id:" + intValue(payload, "getContainerId") + "}";
        }
        if ("player_action_toggle".equals(event)) {
            return base
                    + ",action:" + snbtString(enumName(invokeQuietly(payload, "getAction")))
                    + ",entity_id:" + intValue(payload, "getId")
                    + ",data:" + intValue(payload, "getData") + "}";
        }
        if ("item_rename".equals(event)) {
            return base + ",name:" + snbtString(stringValue(payload, "getName")) + "}";
        }
        if ("trade_select".equals(event)) {
            return base + ",trade_index:" + intValue(payload, "getItem") + "}";
        }
        if ("sign_change".equals(event)) {
            Object position = invokeQuietly(payload, "getPos");
            Object lines = invokeQuietly(payload, "getLines");
            return base + positionFields(position)
                    + ",front:" + boolValue(payload, "isFrontText")
                    + ",line_1:" + snbtString(arrayString(lines, 0))
                    + ",line_2:" + snbtString(arrayString(lines, 1))
                    + ",line_3:" + snbtString(arrayString(lines, 2))
                    + ",line_4:" + snbtString(arrayString(lines, 3)) + "}";
        }
        if ("recipe_place".equals(event)) {
            return base
                    + ",container_id:" + intValue(payload, "containerId")
                    + ",recipe:" + snbtString(String.valueOf(invokeQuietly(payload, "recipe")))
                    + ",use_max_items:" + boolValue(payload, "useMaxItems") + "}";
        }
        if ("game_mode_request".equals(event)) {
            return base + ",mode:" + snbtString(enumName(invokeQuietly(payload, "mode"))) + "}";
        }
        Object position = "block_break".equals(event) ? payload : invokeQuietly(payload, "getPos");
        if ("player_action".equals(event)) {
            return base
                    + ",action:" + snbtString(stringValue(invokeQuietly(payload, "getAction"), "name"))
                    + ",face:" + snbtString(stringValue(invokeQuietly(payload, "getDirection"), "name"))
                    + positionFields(position) + "}";
        }
        if ("block_break".equals(event)) {
            return base + positionFields(position) + "}";
        }
        return base + "}";
    }

    private static String positionFields(Object position) {
        return ",x:" + intValue(position, "getX")
                + ",y:" + intValue(position, "getY")
                + ",z:" + intValue(position, "getZ");
    }

    private static int intValue(Object target, String method) {
        Object value = invokeQuietly(target, method);
        return value instanceof Number ? ((Number) value).intValue() : -1;
    }

    private static String stringValue(Object target, String method) {
        Object value = invokeQuietly(target, method);
        return value == null ? "" : String.valueOf(value);
    }

    private static String enumName(Object value) {
        return stringValue(value, "name");
    }

    private static String boolValue(Object target, String method) {
        Object value = invokeQuietly(target, method);
        return Boolean.TRUE.equals(value) ? "1b" : "0b";
    }

    private static String arrayString(Object array, int index) {
        if (array == null || !array.getClass().isArray() || index >= java.lang.reflect.Array.getLength(array)) {
            return "";
        }
        Object value = java.lang.reflect.Array.get(array, index);
        return value == null ? "" : String.valueOf(value);
    }

    private static Object invokeQuietly(Object target, String method) {
        if (target == null) {
            return null;
        }
        try {
            return invokeNoArgs(target, method);
        } catch (Exception ignored) {
            return null;
        }
    }

    private static String snbtString(String value) {
        return "\"" + value.replace("\\", "\\\\").replace("\"", "\\\"")
                .replace("\n", "\\n").replace("\r", "\\r") + "\"";
    }

    private static void parseRoutes(String value, Map<String, Set<String>> output) {
        for (String route : value.split("\\|")) {
            String[] pieces = route.split(":", 2);
            if (pieces.length != 2 || pieces[0].trim().isEmpty()) {
                continue;
            }
            for (String item : pieces[1].split(",")) {
                String normalized = item.trim().toLowerCase();
                if (!normalized.isEmpty()) {
                    output.computeIfAbsent(normalized, ignored -> new LinkedHashSet<>())
                            .add(pieces[0].trim());
                }
            }
        }
    }
}

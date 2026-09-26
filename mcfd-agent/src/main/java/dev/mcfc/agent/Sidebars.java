package dev.mcfc.agent;

import java.lang.reflect.Constructor;
import java.lang.reflect.Method;
import java.util.HashMap;
import java.util.Map;
import java.util.Optional;
import java.util.TreeMap;
import java.util.UUID;

/**
 * Per-player sidebars, like Paper's per-player scoreboards. Datapacks queue
 * changes in `mcfc:agent sidebar` as {op, uuid, line, text}; every tick they
 * are drained and sent to that player alone as packets for a client-only
 * objective, so no two players share it. Line `n` has score `-n`, so line 0
 * is on top.
 */
final class Sidebars {
    static final String OBJECTIVE = "mcfc_agent_sidebar";
    private static final Map<UUID, Board> boards = new HashMap<>();
    private static Game game;
    private static boolean unsupported;

    private Sidebars() {
    }

    private static final class Board {
        String title = "";
        final TreeMap<Integer, String> lines = new TreeMap<>();
        /** The connection that has this board; a new one (a rejoin) gets it resent. */
        Object connection;
    }

    /** One pending change, decoded from storage. */
    record Op(String op, UUID player, int line, String text) {
    }

    static void tick(Object server) {
        if (unsupported) {
            return;
        }
        try {
            if (game == null) {
                try {
                    game = new Game(server.getClass().getClassLoader());
                } catch (ReflectiveOperationException error) {
                    unsupported = true;
                    System.err.println("[mcfd-agent] player sidebars unavailable: " + error);
                    return;
                }
            }
            Object storage = game.commandStorage.invoke(server);
            Object root = game.storageGet.invoke(storage, game.storageId);
            Object queue = game.getList.invoke(root, "sidebar");
            int size = (Integer) game.listSize.invoke(queue);
            if (size > 0) {
                McfdHooks.runCommand(server, server, "data remove storage mcfc:agent sidebar");
            }
            Object players = game.playerList.invoke(server);
            for (int index = 0; index < size; index++) {
                Op op = decode(game.listCompound.invoke(queue, index));
                if (op != null) {
                    apply(op, players);
                }
            }
            for (Map.Entry<UUID, Board> entry : boards.entrySet()) {
                Object player = game.getPlayer.invoke(players, entry.getKey());
                Board board = entry.getValue();
                Object connection = player == null ? null : McfdHooks.fieldValue(player, "connection");
                if (connection != null && connection != board.connection) {
                    board.connection = connection;
                    sendAll(connection, board);
                }
            }
        } catch (Throwable error) {
            System.err.println("[mcfd-agent] sidebar update failed: " + error);
        }
    }

    private static Op decode(Object tag) throws Exception {
        Optional<?> uuid = (Optional<?>) game.getIntArray.invoke(tag, "uuid");
        if (uuid.isEmpty() || ((int[]) uuid.get()).length != 4) {
            return null;
        }
        int[] parts = (int[]) uuid.get();
        return new Op(
                (String) game.getString.invoke(tag, "op", ""),
                uuid(parts),
                (Integer) game.getInt.invoke(tag, "line", 0),
                (String) game.getString.invoke(tag, "text", ""));
    }

    /** Updates the board, and a player who already has it gets just the change. */
    static void apply(Op op, Object players) throws Exception {
        Board board = boards.computeIfAbsent(op.player(), ignored -> new Board());
        Object connection = board.connection;
        if (connection != null && players != null) {
            Object player = game.getPlayer.invoke(players, op.player());
            if (player == null || McfdHooks.fieldValue(player, "connection") != connection) {
                connection = null;
            }
        }
        switch (op.op()) {
            case "title" -> {
                board.title = op.text();
                if (connection != null) send(connection, game.objectivePacket(board.title, game.methodChange));
            }
            case "line" -> {
                board.lines.put(op.line(), op.text());
                if (connection != null) send(connection, game.scorePacket(op.line(), op.text()));
            }
            case "remove_line" -> {
                board.lines.remove(op.line());
                if (connection != null) send(connection, game.resetPacket(op.line()));
            }
            case "clear" -> {
                boards.remove(op.player());
                if (connection != null) send(connection, game.objectivePacket("", game.methodRemove));
            }
            default -> {
            }
        }
    }

    private static void sendAll(Object connection, Board board) throws Exception {
        send(connection, game.objectivePacket(board.title, game.methodAdd));
        send(connection, game.displayPacket(board.title));
        for (Map.Entry<Integer, String> line : board.lines.entrySet()) {
            send(connection, game.scorePacket(line.getKey(), line.getValue()));
        }
    }

    private static void send(Object connection, Object packet) throws Exception {
        for (Method method : connection.getClass().getMethods()) {
            if (method.getName().equals("send") && method.getParameterCount() == 1) {
                method.invoke(connection, packet);
                return;
            }
        }
    }

    /** Minecraft's `UUID` int array, most significant int first. */
    static UUID uuid(int[] parts) {
        return new UUID((long) parts[0] << 32 | parts[1] & 0xFFFFFFFFL, (long) parts[2] << 32 | parts[3] & 0xFFFFFFFFL);
    }

    /** Self-test: resolves every class and builds each packet against a real game jar. */
    static void checkPackets(ClassLoader loader) throws Exception {
        // A running server has done this; the packet classes need registries.
        Class.forName("net.minecraft.SharedConstants", true, loader).getMethod("tryDetectVersion").invoke(null);
        Class.forName("net.minecraft.server.Bootstrap", true, loader).getMethod("bootStrap").invoke(null);
        Game check = new Game(loader);
        check.objectivePacket("Title", check.methodAdd);
        check.displayPacket("Title");
        check.scorePacket(1, "Line");
        check.resetPacket(1);
    }

    static String owner(int line) {
        return "mcfc.line." + line;
    }

    /** Minecraft 26.3 classes, resolved through the game's own loader. */
    private static final class Game {
        final Method commandStorage;
        final Method storageGet;
        final Object storageId;
        final Method getList;
        final Method listSize;
        final Method listCompound;
        final Method getIntArray;
        final Method getString;
        final Method getInt;
        final Method playerList;
        final Method getPlayer;
        final Method literal;
        final Constructor<?> scoreboard;
        final Constructor<?> objective;
        final Constructor<?> objectivePacket;
        final Constructor<?> displayPacket;
        final Constructor<?> scorePacket;
        final Constructor<?> resetPacket;
        final Object dummy;
        final Object integer;
        final Object blank;
        final Object sidebar;
        final int methodAdd;
        final int methodRemove;
        final int methodChange;

        Game(ClassLoader loader) throws Exception {
            Class<?> server = Class.forName("net.minecraft.server.MinecraftServer", false, loader);
            Class<?> identifier = Class.forName("net.minecraft.resources.Identifier", false, loader);
            Class<?> compound = Class.forName("net.minecraft.nbt.CompoundTag", false, loader);
            Class<?> list = Class.forName("net.minecraft.nbt.ListTag", false, loader);
            Class<?> players = Class.forName("net.minecraft.server.players.PlayerList", false, loader);
            Class<?> component = Class.forName("net.minecraft.network.chat.Component", false, loader);
            Class<?> board = Class.forName("net.minecraft.world.scores.Scoreboard", false, loader);
            Class<?> objectiveClass = Class.forName("net.minecraft.world.scores.Objective", false, loader);
            Class<?> criteria = Class.forName("net.minecraft.world.scores.criteria.ObjectiveCriteria", false, loader);
            Class<?> renderType = Class.forName(
                    "net.minecraft.world.scores.criteria.ObjectiveCriteria$RenderType", false, loader);
            Class<?> numberFormat = Class.forName("net.minecraft.network.chat.numbers.NumberFormat", false, loader);
            Class<?> blankFormat = Class.forName("net.minecraft.network.chat.numbers.BlankFormat", false, loader);
            Class<?> displaySlot = Class.forName("net.minecraft.world.scores.DisplaySlot", false, loader);
            Class<?> setObjective = Class.forName(
                    "net.minecraft.network.protocol.game.ClientboundSetObjectivePacket", false, loader);
            commandStorage = server.getMethod("getCommandStorage");
            storageGet = commandStorage.getReturnType().getMethod("get", identifier);
            storageId = identifier.getMethod("parse", String.class).invoke(null, "mcfc:agent");
            getList = compound.getMethod("getListOrEmpty", String.class);
            listSize = list.getMethod("size");
            listCompound = list.getMethod("getCompoundOrEmpty", int.class);
            getIntArray = compound.getMethod("getIntArray", String.class);
            getString = compound.getMethod("getStringOr", String.class, String.class);
            getInt = compound.getMethod("getIntOr", String.class, int.class);
            playerList = server.getMethod("getPlayerList");
            getPlayer = players.getMethod("getPlayer", UUID.class);
            literal = component.getMethod("literal", String.class);
            scoreboard = board.getConstructor();
            objective = objectiveClass.getConstructor(
                    board, String.class, criteria, component, renderType, boolean.class, numberFormat);
            objectivePacket = setObjective.getConstructor(objectiveClass, int.class);
            displayPacket = Class.forName(
                    "net.minecraft.network.protocol.game.ClientboundSetDisplayObjectivePacket", false, loader)
                    .getConstructor(displaySlot, objectiveClass);
            scorePacket = Class.forName(
                    "net.minecraft.network.protocol.game.ClientboundSetScorePacket", false, loader)
                    .getConstructor(String.class, String.class, int.class, Optional.class, Optional.class);
            resetPacket = Class.forName(
                    "net.minecraft.network.protocol.game.ClientboundResetScorePacket", false, loader)
                    .getConstructor(String.class, String.class);
            dummy = criteria.getField("DUMMY").get(null);
            integer = renderType.getField("INTEGER").get(null);
            blank = blankFormat.getField("INSTANCE").get(null);
            sidebar = displaySlot.getField("SIDEBAR").get(null);
            methodAdd = setObjective.getField("METHOD_ADD").getInt(null);
            methodRemove = setObjective.getField("METHOD_REMOVE").getInt(null);
            methodChange = setObjective.getField("METHOD_CHANGE").getInt(null);
        }

        Object objective(String title) throws Exception {
            return objective.newInstance(
                    scoreboard.newInstance(), OBJECTIVE, dummy, literal.invoke(null, title), integer, false, blank);
        }

        Object objectivePacket(String title, int method) throws Exception {
            return objectivePacket.newInstance(objective(title), method);
        }

        Object displayPacket(String title) throws Exception {
            return displayPacket.newInstance(sidebar, objective(title));
        }

        Object scorePacket(int line, String text) throws Exception {
            return scorePacket.newInstance(
                    owner(line), OBJECTIVE, -line, Optional.of(literal.invoke(null, text)), Optional.empty());
        }

        Object resetPacket(int line) throws Exception {
            return resetPacket.newInstance(owner(line), OBJECTIVE);
        }
    }
}

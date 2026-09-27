package dev.mcfc.agent;

import java.util.ArrayList;
import java.util.List;

/** Lightweight reflection-bridge test; run through {@code test.ps1}. */
public final class McfdHooksSelfTest {
    private McfdHooksSelfTest() {
    }

    public static void main(String[] args) throws Exception {
        require(Sidebars.uuid(new int[] {0x12345678, 0x9abcdef0, -1, 1})
                        .equals(java.util.UUID.fromString("12345678-9abc-def0-ffff-ffff00000001")),
                "UUID int arrays must decode like Minecraft's UUIDUtil");
        if (args.length > 0) {
            // A file listing the game jar and its libraries: every sidebar packet must build.
            java.util.List<java.net.URL> urls = new ArrayList<>();
            for (String jar : java.nio.file.Files.readAllLines(java.nio.file.Path.of(args[0]))) {
                urls.add(java.nio.file.Path.of(jar.trim()).toUri().toURL());
            }
            Sidebars.checkPackets(new java.net.URLClassLoader(urls.toArray(new java.net.URL[0])));
            System.out.println("sidebar packets build against the game jar");
        }
        McfdHooks.configure("routes=demo:chat,player_interact_block|absent:chat;commands=demo:status");
        FakeServer server = new FakeServer();
        FakePlayer player = new FakePlayer(server, "Tester");
        FakeListener listener = new FakeListener(player);

        boolean cancelled = McfdHooks.before("chat", listener, new FakeChatPacket("hello"));
        require(!cancelled, "chat should be observe-only in this test");
        require(server.commands.commands.size() == 2,
                "chat should emit a storage write and function call, and skip the unloaded 'absent' pack");
        require(server.commands.sources.stream().allMatch(source -> source == SUPPRESSED),
                "agent commands must run with suppressed output");
        require(
                server.commands.commands.get(0).contains("data modify storage demo:agent current set value")
                        && server.commands.commands.get(0).contains("message:\"hello\""),
                "chat payload was not written to agent storage");
        require(
                server.commands.commands.get(1).equals(
                        "execute as @a[name=Tester] run function demo:agent/event/chat"),
                "chat callback did not run as the affected player");

        McfdHooks.before(
                "player_interact_block",
                listener,
                new FakeBlockInteractPacket(Hand.MAIN_HAND, new FakeHit(new FakePos(4, 70, -9), Face.UP)));
        require(server.commands.commands.size() == 4, "block interaction should emit two commands");
        require(
                server.commands.commands.get(2).contains("hand:\"MAIN_HAND\"")
                        && server.commands.commands.get(2).contains("face:\"UP\"")
                        && server.commands.commands.get(2).contains("x:4,y:70,z:-9"),
                "block interaction fields were not extracted");

        require(McfdHooks.handleCommand(listener, new FakeCommandPacket("status")),
                "configured root command should be handled");
        require(server.commands.commands.get(5).equals(
                        "execute as @a[name=Tester] run function demo:agent/command/status"),
                "command route did not invoke the generated wrapper");
        require(!McfdHooks.handleCommand(listener, new FakeCommandPacket("vanilla_command")),
                "unknown roots must continue through vanilla");
        System.out.println("mcfd-agent reflection dispatch self-test passed");
    }

    private static void require(boolean condition, String message) {
        if (!condition) {
            throw new AssertionError(message);
        }
    }

    public static final class FakeServer {
        public final FakeCommands commands = new FakeCommands();

        public void execute(Runnable runnable) {
            runnable.run();
        }

        public FakeCommands getCommands() {
            return commands;
        }

        public Object createCommandSourceStack() {
            return this;
        }

        public Object withSuppressedOutput() {
            return SUPPRESSED;
        }

        public FakeFunctions getFunctions() {
            return new FakeFunctions();
        }
    }

    static final Object SUPPRESSED = new Object();

    public static final class FakeCommands {
        final List<String> commands = new ArrayList<>();
        final List<Object> sources = new ArrayList<>();

        public void performPrefixedCommand(Object source, String command) {
            sources.add(source);
            commands.add(command);
        }
    }

    public record FakeId(String value) {
        public static FakeId parse(String value) {
            return new FakeId(value);
        }
    }

    /** Only the `demo` pack is loaded. */
    public static final class FakeFunctions {
        public java.util.Optional<Object> get(FakeId id) {
            return id.value().startsWith("demo:") ? java.util.Optional.of(id) : java.util.Optional.empty();
        }
    }

    public static final class FakePlayer {
        @SuppressWarnings("unused")
        private final FakeServer server;
        private final String name;

        FakePlayer(FakeServer server, String name) {
            this.server = server;
            this.name = name;
        }

        public String getScoreboardName() {
            return name;
        }
    }

    public static final class FakeListener {
        @SuppressWarnings("unused")
        public final FakePlayer player;

        FakeListener(FakePlayer player) {
            this.player = player;
        }
    }

    public record FakeChatPacket(String message) {
    }

    public record FakeCommandPacket(String command) {
    }

    // Mirrors the 26.3 record ServerboundUseItemOnPacket(hand, hitResult).
    public record FakeBlockInteractPacket(Hand hand, FakeHit hitResult) {
    }

    public record FakeHit(FakePos pos, Face face) {
        public FakePos getBlockPos() {
            return pos;
        }

        public Face getDirection() {
            return face;
        }
    }

    public record FakePos(int x, int y, int z) {
        public int getX() {
            return x;
        }

        public int getY() {
            return y;
        }

        public int getZ() {
            return z;
        }
    }

    public enum Hand {
        MAIN_HAND
    }

    public enum Face {
        UP
    }
}

package dev.mcfc.agent;

import java.lang.instrument.ClassFileTransformer;
import java.lang.instrument.IllegalClassFormatException;
import java.lang.instrument.Instrumentation;
import java.security.ProtectionDomain;
import java.util.Arrays;
import java.util.HashSet;
import java.util.ArrayList;
import java.util.List;
import java.util.Set;
import java.util.function.Function;
import org.objectweb.asm.ClassReader;
import org.objectweb.asm.ClassVisitor;
import org.objectweb.asm.ClassWriter;
import org.objectweb.asm.Label;
import org.objectweb.asm.MethodVisitor;
import org.objectweb.asm.Opcodes;

/**
 * Minimal, loader-safe entrypoint for the optional MCFC server agent.
 *
 * The first adapter targets the named 26.3 server classes. It observes chat,
 * inventory-click, player-action, and block-break entrypoints. A configured
 * event can be cancelled before vanilla receives it; all other hooks only log.
 */
public final class McfdAgent {
    private static volatile Instrumentation instrumentation;

    private McfdAgent() {
    }

    public static void premain(String args, Instrumentation instance) {
        install("startup", args, instance);
    }

    public static void agentmain(String args, Instrumentation instance) {
        install("dynamic", args, instance);
    }

    static final String HOOKS_PROPERTY = "mcfd.hooks";

    /**
     * Emit `((Function) System.getProperties().get("mcfd.hooks")).apply(new
     * Object[] {kind, event, source, payload})` as a boolean on the stack.
     * `event` may be null, and a negative local pushes null.
     */
    static void emitDispatch(MethodVisitor mv, String kind, String event, int sourceLocal, int payloadLocal) {
        mv.visitMethodInsn(Opcodes.INVOKESTATIC, "java/lang/System", "getProperties", "()Ljava/util/Properties;", false);
        mv.visitLdcInsn(HOOKS_PROPERTY);
        mv.visitMethodInsn(Opcodes.INVOKEVIRTUAL, "java/util/Properties", "get", "(Ljava/lang/Object;)Ljava/lang/Object;", false);
        mv.visitTypeInsn(Opcodes.CHECKCAST, "java/util/function/Function");
        mv.visitInsn(Opcodes.ICONST_4);
        mv.visitTypeInsn(Opcodes.ANEWARRAY, "java/lang/Object");
        Object[] values = {kind, event, sourceLocal, payloadLocal};
        for (int index = 0; index < values.length; index++) {
            mv.visitInsn(Opcodes.DUP);
            mv.visitInsn(Opcodes.ICONST_0 + index);
            Object value = values[index];
            if (value instanceof Integer) {
                mv.visitVarInsn(Opcodes.ALOAD, (Integer) value);
            } else if (value == null) {
                mv.visitInsn(Opcodes.ACONST_NULL);
            } else {
                mv.visitLdcInsn(value);
            }
            mv.visitInsn(Opcodes.AASTORE);
        }
        mv.visitMethodInsn(Opcodes.INVOKEINTERFACE, "java/util/function/Function", "apply", "(Ljava/lang/Object;)Ljava/lang/Object;", true);
        mv.visitTypeInsn(Opcodes.CHECKCAST, "java/lang/Boolean");
        mv.visitMethodInsn(Opcodes.INVOKEVIRTUAL, "java/lang/Boolean", "booleanValue", "()Z", false);
    }

    public static boolean isActive() {
        return instrumentation != null;
    }

    private static synchronized void install(String mode, String args, Instrumentation instance) {
        if (instrumentation != null) {
            System.err.println("[mcfd-agent] already active");
            return;
        }
        instrumentation = instance;
        System.setProperty("mcfd.agent.active", "true");
        McfdHooks.configure(args);
        // Injected code reaches the hooks through this JDK-typed property, not
        // by naming McfdHooks: mod loaders such as Fabric's Knot refuse to
        // load classes from a jar that was attached after startup.
        System.getProperties().put(HOOKS_PROPERTY, (Function<Object[], Object>) McfdHooks::dispatch);
        MinecraftServerProbe transformer = new MinecraftServerProbe();
        instance.addTransformer(transformer, true);
        for (Class<?> loaded : instance.getAllLoadedClasses()) {
            if (transformer.targets(loaded.getName()) && instance.isModifiableClass(loaded)) {
                try {
                    instance.retransformClasses(loaded);
                } catch (Exception error) {
                    System.err.println("[mcfd-agent] could not retransform " + loaded.getName() + ": " + error);
                }
            }
        }
        System.err.println("[mcfd-agent] attached via " + mode + " mode; hooks=" + McfdHooks.describe());
    }

    private static final class MinecraftServerProbe implements ClassFileTransformer {
        private static final Set<String> TARGETS = new HashSet<>(Arrays.asList(
                "net/minecraft/server/network/ServerGamePacketListenerImpl",
                "net/minecraft/server/level/ServerPlayerGameMode",
                "net/minecraft/server/level/ServerPlayer",
                "net/minecraft/server/players/PlayerList"));

        boolean targets(String className) {
            return TARGETS.contains(className.replace('.', '/'));
        }

        @Override
        public byte[] transform(
                Module module,
                ClassLoader loader,
                String className,
                Class<?> classBeingRedefined,
                ProtectionDomain protectionDomain,
                byte[] classfileBuffer) throws IllegalClassFormatException {
            if (!TARGETS.contains(className)) {
                return null;
            }
            try {
                ClassReader reader = new ClassReader(classfileBuffer);
                SafeClassWriter writer = new SafeClassWriter(reader);
                EventClassVisitor visitor =
                        new EventClassVisitor(writer, className, methodsWithThreadCheck(reader));
                reader.accept(visitor, ClassReader.EXPAND_FRAMES);
                if (!visitor.hasChanges()) {
                    return null;
                }
                System.err.println("[mcfd-agent] installed 26.3 hooks in " + className
                        + ": " + visitor.installedHooks());
                return writer.toByteArray();
            } catch (Throwable error) {
                System.err.println("[mcfd-agent] failed to transform " + className + ": " + error);
                return null;
            }
        }
    }

    static final String THREAD_CHECK_OWNER = "net/minecraft/network/protocol/PacketUtils";
    static final String THREAD_CHECK_NAME = "ensureRunningOnSameThread";

    /**
     * Methods (name + descriptor) that call `PacketUtils.ensureRunningOnSameThread`.
     * Vanilla runs such a packet handler on the network thread first, where
     * that call hands the packet to the server thread and throws; the handler
     * then runs again on the server thread. Their hooks go after the call so
     * each packet fires one event, on the server thread.
     */
    static Set<String> methodsWithThreadCheck(ClassReader reader) {
        Set<String> found = new HashSet<>();
        reader.accept(new ClassVisitor(Opcodes.ASM9) {
            @Override
            public MethodVisitor visitMethod(int access, String name, String descriptor, String signature, String[] exceptions) {
                return new MethodVisitor(Opcodes.ASM9) {
                    @Override
                    public void visitMethodInsn(int opcode, String owner, String callee, String calleeDescriptor, boolean isInterface) {
                        if (THREAD_CHECK_OWNER.equals(owner) && THREAD_CHECK_NAME.equals(callee)) {
                            found.add(name + descriptor);
                        }
                    }
                };
            }
        }, ClassReader.SKIP_FRAMES);
        return found;
    }

    /** Injects a hook at method entry, or right after the thread check when there is one. */
    private abstract static class HookSiteVisitor extends MethodVisitor {
        private boolean pending;

        HookSiteVisitor(MethodVisitor delegate, boolean afterThreadCheck) {
            super(Opcodes.ASM9, delegate);
            this.pending = afterThreadCheck;
        }

        abstract void inject();

        @Override
        public void visitCode() {
            super.visitCode();
            if (!pending) {
                inject();
            }
        }

        @Override
        public void visitMethodInsn(int opcode, String owner, String name, String descriptor, boolean isInterface) {
            super.visitMethodInsn(opcode, owner, name, descriptor, isInterface);
            if (pending && THREAD_CHECK_OWNER.equals(owner) && THREAD_CHECK_NAME.equals(name)) {
                pending = false;
                inject();
            }
        }
    }

    private static final class SafeClassWriter extends ClassWriter {
        SafeClassWriter(ClassReader reader) {
            super(reader, ClassWriter.COMPUTE_FRAMES | ClassWriter.COMPUTE_MAXS);
        }

        @Override
        protected String getCommonSuperClass(String left, String right) {
            return "java/lang/Object";
        }
    }

    private static final class EventClassVisitor extends ClassVisitor {
        private final String className;
        private final Set<String> threadChecked;
        private final List<String> installedHooks = new ArrayList<>();
        private boolean changes;

        EventClassVisitor(ClassVisitor delegate, String className, Set<String> threadChecked) {
            super(Opcodes.ASM9, delegate);
            this.className = className;
            this.threadChecked = threadChecked;
        }

        boolean hasChanges() {
            return changes;
        }

        String installedHooks() {
            return String.join(", ", installedHooks);
        }

        @Override
        public MethodVisitor visitMethod(int access, String name, String descriptor, String signature, String[] exceptions) {
            MethodVisitor delegate = super.visitMethod(access, name, descriptor, signature, exceptions);
            EventHook hook = eventFor(className, name, descriptor);
            if (hook == null) {
                return delegate;
            }
            changes = true;
            installedHooks.add(name + " -> " + hook.event);
            boolean after = threadChecked.contains(name + descriptor);
            if ("command".equals(hook.event)) {
                return new CommandMethodVisitor(delegate, after);
            }
            if (!hook.cancellable) {
                return new ObservationMethodVisitor(delegate, after, hook.event, hook.sourceLocal, hook.payloadLocal);
            }
            boolean returnsBoolean = "(Lnet/minecraft/core/BlockPos;)Z".equals(descriptor);
            return new CancellationMethodVisitor(
                    delegate, after, hook.event, returnsBoolean, hook.sourceLocal, hook.payloadLocal);
        }

        private static EventHook eventFor(String owner, String name, String descriptor) {
            if ("net/minecraft/server/network/ServerGamePacketListenerImpl".equals(owner)) {
                if ("handleContainerClick".equals(name)) return cancellable("inventory_click");
                // Creative inventory mutations bypass the regular container-click packet.
                if ("handleSetCreativeModeSlot".equals(name)) return cancellable("inventory_click");
                if ("handleContainerButtonClick".equals(name)) return cancellable("inventory_click");
                if ("handlePlaceRecipe".equals(name)) return cancellable("recipe_place");
                if ("handleContainerClose".equals(name)) return cancellable("inventory_close");
                if ("handleChat".equals(name)) return cancellable("chat");
                if ("handleChatCommand".equals(name)) return cancellable("command");
                if ("handleSignedChatCommand".equals(name)) return cancellable("command");
                if ("handlePlayerAction".equals(name)) return cancellable("player_action");
                if ("handleUseItemOn".equals(name)) return cancellable("player_interact_block");
                if ("handleUseItem".equals(name)) return cancellable("player_interact_item");
                if ("handleInteract".equals(name)) return cancellable("entity_interact");
                if ("handleAttack".equals(name)) return cancellable("entity_attack");
                if ("handleSetCarriedItem".equals(name)) return cancellable("item_held_change");
                if ("handlePunch".equals(name)) return cancellable("player_swing");
                if ("handlePlayerCommand".equals(name)) return cancellable("player_action_toggle");
                if ("handleClientCommand".equals(name)) return cancellable("player_respawn_request");
                if ("handleRenameItem".equals(name)) return cancellable("item_rename");
                if ("handleSelectTrade".equals(name)) return cancellable("trade_select");
                if ("handleSignUpdate".equals(name)) return cancellable("sign_change");
                if ("handleEditBook".equals(name)) return cancellable("book_edit");
                if ("handleSetBeaconPacket".equals(name)) return cancellable("beacon_effect");
                if ("handlePickItemFromBlock".equals(name) || "handlePickItemFromEntity".equals(name)) return cancellable("item_pick");
                if ("handleTeleportToEntityPacket".equals(name)) return cancellable("entity_teleport");
                if ("handleChangeGameMode".equals(name)) return cancellable("game_mode_request");
                if ("handlePlayerAbilities".equals(name)) return cancellable("player_abilities");
            }
            if ("net/minecraft/server/level/ServerPlayerGameMode".equals(owner)
                    && "destroyBlock".equals(name)
                    && "(Lnet/minecraft/core/BlockPos;)Z".equals(descriptor)) {
                return cancellable("block_break");
            }
            if ("net/minecraft/server/level/ServerPlayerGameMode".equals(owner)
                    && "changeGameModeForPlayer".equals(name)) {
                return observed("game_mode_change");
            }
            if ("net/minecraft/server/players/PlayerList".equals(owner)) {
                if ("placeNewPlayer".equals(name)) return observed("player_connect", 2, 1);
                if ("remove".equals(name)) return observed("player_quit", 1, 1);
                if ("respawn".equals(name)) return observed("player_respawn", 1, 1);
            }
            if ("net/minecraft/server/level/ServerPlayer".equals(owner)) {
                if ("die".equals(name)) return observed("player_death", 0, 1);
                if ("hurtServer".equals(name)) return observed("player_damage");
                if ("teleport".equals(name) && descriptor.startsWith("(L")) return observed("player_teleport");
                if ("drop".equals(name)
                        && descriptor.startsWith("(Lnet/minecraft/world/item/ItemStack;")) {
                    return observed("player_item_drop");
                }
                if ("onItemPickup".equals(name)) return observed("player_item_pickup");
                if ("openMenu".equals(name)) return observed("inventory_open");
            }
            return null;
        }

        private static EventHook cancellable(String event) {
            return new EventHook(event, true, 0, 1);
        }

        private static EventHook observed(String event) {
            return observed(event, 0, 1);
        }

        private static EventHook observed(String event, int sourceLocal, int payloadLocal) {
            return new EventHook(event, false, sourceLocal, payloadLocal);
        }
    }

    private static final class EventHook {
        final String event;
        final boolean cancellable;
        final int sourceLocal;
        final int payloadLocal;

        EventHook(String event, boolean cancellable, int sourceLocal, int payloadLocal) {
            this.event = event;
            this.cancellable = cancellable;
            this.sourceLocal = sourceLocal;
            this.payloadLocal = payloadLocal;
        }
    }

    /** A real MCFC root command is consumed only when the hook reports it handled. */
    private static final class CommandMethodVisitor extends HookSiteVisitor {
        CommandMethodVisitor(MethodVisitor delegate, boolean afterThreadCheck) {
            super(delegate, afterThreadCheck);
        }

        @Override
        void inject() {
            Label continueVanilla = new Label();
            emitDispatch(this, "command", null, 0, 1);
            visitJumpInsn(Opcodes.IFEQ, continueVanilla);
            visitInsn(Opcodes.RETURN);
            visitLabel(continueVanilla);
        }
    }

    private static final class CancellationMethodVisitor extends HookSiteVisitor {
        private final String event;
        private final boolean returnsBoolean;
        private final int sourceLocal;
        private final int payloadLocal;

        CancellationMethodVisitor(
                MethodVisitor delegate,
                boolean afterThreadCheck,
                String event,
                boolean returnsBoolean,
                int sourceLocal,
                int payloadLocal) {
            super(delegate, afterThreadCheck);
            this.event = event;
            this.returnsBoolean = returnsBoolean;
            this.sourceLocal = sourceLocal;
            this.payloadLocal = payloadLocal;
        }

        @Override
        void inject() {
            Label continueVanilla = new Label();
            emitDispatch(this, "before", event, sourceLocal, payloadLocal);
            visitJumpInsn(Opcodes.IFEQ, continueVanilla);
            if (returnsBoolean) {
                visitInsn(Opcodes.ICONST_0);
                visitInsn(Opcodes.IRETURN);
            } else {
                visitInsn(Opcodes.RETURN);
            }
            visitLabel(continueVanilla);
        }
    }

    /** Entry hook for lifecycle/authoritative events that must never be cancelled. */
    private static final class ObservationMethodVisitor extends HookSiteVisitor {
        private final String event;
        private final int sourceLocal;
        private final int payloadLocal;

        ObservationMethodVisitor(
                MethodVisitor delegate, boolean afterThreadCheck, String event, int sourceLocal, int payloadLocal) {
            super(delegate, afterThreadCheck);
            this.event = event;
            this.sourceLocal = sourceLocal;
            this.payloadLocal = payloadLocal;
        }

        @Override
        void inject() {
            emitDispatch(this, "observe", event, sourceLocal, payloadLocal);
            visitInsn(Opcodes.POP);
        }
    }
}

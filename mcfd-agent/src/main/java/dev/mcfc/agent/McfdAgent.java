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
import org.objectweb.asm.ClassReader;
import org.objectweb.asm.ClassVisitor;
import org.objectweb.asm.ClassWriter;
import org.objectweb.asm.Label;
import org.objectweb.asm.MethodVisitor;
import org.objectweb.asm.Opcodes;

/**
 * Minimal, loader-safe entrypoint for the optional MCFC server agent.
 *
 * The first adapter targets the named 26.2 server classes. It observes chat,
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
                EventClassVisitor visitor = new EventClassVisitor(writer, className);
                reader.accept(visitor, ClassReader.EXPAND_FRAMES);
                if (!visitor.hasChanges()) {
                    return null;
                }
                System.err.println("[mcfd-agent] installed 26.2 hooks in " + className
                        + ": " + visitor.installedHooks());
                return writer.toByteArray();
            } catch (Throwable error) {
                System.err.println("[mcfd-agent] failed to transform " + className + ": " + error);
                return null;
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
        private final List<String> installedHooks = new ArrayList<>();
        private boolean changes;

        EventClassVisitor(ClassVisitor delegate, String className) {
            super(Opcodes.ASM9, delegate);
            this.className = className;
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
            if ("command".equals(hook.event)) {
                return new CommandMethodVisitor(delegate);
            }
            if (!hook.cancellable) {
                return new ObservationMethodVisitor(delegate, hook.event, hook.sourceLocal, hook.payloadLocal);
            }
            boolean returnsBoolean = "(Lnet/minecraft/core/BlockPos;)Z".equals(descriptor);
            return new CancellationMethodVisitor(
                    delegate, hook.event, returnsBoolean, hook.sourceLocal, hook.payloadLocal);
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
                if ("handleAnimate".equals(name)) return cancellable("player_swing");
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
    private static final class CommandMethodVisitor extends MethodVisitor {
        CommandMethodVisitor(MethodVisitor delegate) {
            super(Opcodes.ASM9, delegate);
        }

        @Override
        public void visitCode() {
            super.visitCode();
            Label continueVanilla = new Label();
            visitVarInsn(Opcodes.ALOAD, 0);
            visitVarInsn(Opcodes.ALOAD, 1);
            visitMethodInsn(
                    Opcodes.INVOKESTATIC,
                    "dev/mcfc/agent/McfdHooks",
                    "handleCommand",
                    "(Ljava/lang/Object;Ljava/lang/Object;)Z",
                    false);
            visitJumpInsn(Opcodes.IFEQ, continueVanilla);
            visitInsn(Opcodes.RETURN);
            visitLabel(continueVanilla);
        }
    }

    private static final class CancellationMethodVisitor extends MethodVisitor {
        private final String event;
        private final boolean returnsBoolean;
        private final int sourceLocal;
        private final int payloadLocal;

        CancellationMethodVisitor(
                MethodVisitor delegate, String event, boolean returnsBoolean, int sourceLocal, int payloadLocal) {
            super(Opcodes.ASM9, delegate);
            this.event = event;
            this.returnsBoolean = returnsBoolean;
            this.sourceLocal = sourceLocal;
            this.payloadLocal = payloadLocal;
        }

        @Override
        public void visitCode() {
            super.visitCode();
            Label continueVanilla = new Label();
            visitLdcInsn(event);
            visitVarInsn(Opcodes.ALOAD, sourceLocal);
            visitVarInsn(Opcodes.ALOAD, payloadLocal);
            visitMethodInsn(
                    Opcodes.INVOKESTATIC,
                    "dev/mcfc/agent/McfdHooks",
                    "before",
                    "(Ljava/lang/String;Ljava/lang/Object;Ljava/lang/Object;)Z",
                    false);
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
    private static final class ObservationMethodVisitor extends MethodVisitor {
        private final String event;
        private final int sourceLocal;
        private final int payloadLocal;

        ObservationMethodVisitor(MethodVisitor delegate, String event, int sourceLocal, int payloadLocal) {
            super(Opcodes.ASM9, delegate);
            this.event = event;
            this.sourceLocal = sourceLocal;
            this.payloadLocal = payloadLocal;
        }

        @Override
        public void visitCode() {
            super.visitCode();
            visitLdcInsn(event);
            visitVarInsn(Opcodes.ALOAD, sourceLocal);
            visitVarInsn(Opcodes.ALOAD, payloadLocal);
            visitMethodInsn(
                    Opcodes.INVOKESTATIC,
                    "dev/mcfc/agent/McfdHooks",
                    "observe",
                    "(Ljava/lang/String;Ljava/lang/Object;Ljava/lang/Object;)V",
                    false);
        }
    }
}

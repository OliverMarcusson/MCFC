package dev.mcfc.agent;

import com.sun.tools.attach.VirtualMachine;
import java.nio.file.Files;
import java.nio.file.Path;

/** Small command-line bridge around the JDK Attach API. */
public final class AttachMain {
    private AttachMain() {
    }

    public static void main(String[] args) throws Exception {
        if (args.length < 2 || args.length > 3) {
            System.err.println("usage: AttachMain <pid> <agent-jar> [agent-options]");
            System.exit(2);
        }
        Path agent = Path.of(args[1]).toAbsolutePath().normalize();
        if (!Files.isRegularFile(agent)) {
            throw new IllegalArgumentException("agent JAR does not exist: " + agent);
        }
        VirtualMachine vm = VirtualMachine.attach(args[0]);
        try {
            vm.loadAgent(agent.toString(), args.length == 3 ? args[2] : "");
            System.out.println("mcfd-agent: attached to JVM " + args[0]);
        } finally {
            vm.detach();
        }
    }
}

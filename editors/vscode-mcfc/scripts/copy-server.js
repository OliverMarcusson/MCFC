const fs = require("fs");
const path = require("path");
const { execFileSync } = require("child_process");

if (process.platform !== "win32") {
  throw new Error("The initial MCFC VSIX packaging script supports Windows only.");
}

const extensionRoot = path.resolve(__dirname, "..");
const repoRoot = path.resolve(extensionRoot, "..", "..");
const destinationDir = path.join(extensionRoot, "server", "win32-x64");
const destination = path.join(destinationDir, "mcfc-lsp.exe");

execFileSync("cargo", ["build", "--bin", "mcfc-lsp", "--release"], {
  cwd: repoRoot,
  stdio: "inherit",
});

const releaseServer = path.join(repoRoot, "target", "release", "mcfc-lsp.exe");
if (!fs.existsSync(releaseServer)) {
  throw new Error(`Expected built server at ${releaseServer}`);
}

fs.mkdirSync(destinationDir, { recursive: true });
fs.copyFileSync(releaseServer, destination);
console.log(`Copied ${releaseServer} to ${destination}`);

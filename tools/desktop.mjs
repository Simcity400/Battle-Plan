// One development command builds the native viewer, then starts Tauri and Vite.
// No shell interpolation: paths and user arguments remain separate arguments.
import { spawn } from "node:child_process";
import { copyFile, cp } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import path from "node:path";

const root = fileURLToPath(new URL("../", import.meta.url));
const mode = process.argv[2];
let interrupted = false;
if (!["dev", "build"].includes(mode)) {
  console.error("Usage: node tools/desktop.mjs dev|build");
  process.exit(2);
}

function run(executable, args, cwd = root) {
  if (interrupted) return Promise.reject(new Error("Interrupted"));
  return new Promise((resolve, reject) => {
    const child = spawn(executable, args, { cwd, stdio: "inherit", windowsHide: true });
    const interrupt = () => {
      interrupted = true;
      // Windows delivers console Ctrl+C to this attached child too. Sending
      // Node's emulated SIGINT would kill Tauri before it cleans up Vite.
      if (process.platform !== "win32") child.kill("SIGINT");
    };
    process.once("SIGINT", interrupt);
    child.once("error", (error) => {
      process.removeListener("SIGINT", interrupt);
      reject(error);
    });
    child.once("exit", (code, signal) => {
      process.removeListener("SIGINT", interrupt);
      if (interrupted) reject(new Error("Interrupted"));
      else if (code === 0) resolve();
      else reject(new Error(`${path.basename(executable)} exited with ${code ?? signal}`));
    });
  });
}

const extra = process.argv.slice(3);
// Tauri separates its own flags, Cargo runner flags and application arguments.
const separator = extra.indexOf("--");
const cliArgs = separator < 0 ? extra : extra.slice(0, separator);
const runnerArgs = separator < 0 ? [] : extra.slice(separator + 1);
const appSeparator = runnerArgs.indexOf("--");
const cargoArgs = appSeparator < 0 ? runnerArgs : runnerArgs.slice(0, appSeparator);
const profileIndex = cargoArgs.indexOf("--profile");
const explicitProfile = profileIndex < 0
  ? cargoArgs.find((arg) => arg.startsWith("--profile="))?.slice("--profile=".length)
  : cargoArgs[profileIndex + 1];
const release = mode === "build" || cliArgs.includes("--release")
  || cargoArgs.includes("--release") || cargoArgs.includes("-r");
const profile = explicitProfile ?? (release ? "release" : "interactive");
const defaultProfile = !explicitProfile && !release ? ["--profile", "interactive"] : [];

try {
  await run("cargo", ["build", "-p", "viewer", "--profile", profile]);
  // Every shipped viewer uses the same pinned compiler, including vanilla recordings.
  if (mode === "build") {
    await run("powershell", ["-NoProfile", "-ExecutionPolicy", "Bypass", "-File",
      path.join(root, "tools", "install-shader-compiler.ps1"), "-Destination",
      path.join(root, ".cargo-target", "release", "shader-tools", "dxc")]);
  }
  const cli = path.join(root, "node_modules", "@tauri-apps", "cli", "tauri.js");
  // Resolve hook directories at launch without storing machine paths in source.
  // JSON is passed as one argument, never interpreted as shell text.
  const hooks = JSON.stringify({ build: {
    beforeDevCommand: { script: "npm run dev:ui", cwd: root },
    beforeBuildCommand: { script: "npm run build:ui", cwd: root },
  } });
  // Keep both separators and use the same profile for the host and viewer.
  const args = mode === "build"
    ? ["--no-bundle", ...extra]
    : [...cliArgs, "--", ...defaultProfile, ...runnerArgs];
  await run(process.execPath, [cli, mode, "--config", hooks, ...args], path.join(root, "crates", "desktop"));
  if (mode === "build") {
    // Battle Plan Recorder ships beside the replay app, embeds the same frontend build
    // and injects the collector from its own folder.
    await run("cargo", ["build", "-p", "recorder-app", "-p", "collector", "--release", "--features", "recorder-app/custom-protocol"]);
    const release = path.join(root, ".cargo-target", "release");
    await copyFile(path.join(root, "LICENSE"), path.join(release, "LICENSE"));
    await cp(path.join(root, "LICENSES"), path.join(release, "LICENSES"), { recursive: true });
  }
} catch (error) {
  if (!interrupted) console.error(error instanceof Error ? error.message : String(error));
  process.exitCode = interrupted ? 130 : 1;
}

// One development command builds the native viewer, then starts Tauri and Vite.
// No shell interpolation: paths and user arguments remain separate arguments.
import { spawn } from "node:child_process";
import { access, copyFile, cp } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import path from "node:path";

const root = fileURLToPath(new URL("../", import.meta.url));
const mode = process.argv[2];
let interrupted = false;
if (!["dev", "build"].includes(mode)) {
  console.error("Usage: node tools/desktop.mjs dev|build");
  process.exit(2);
}

function run(executable, args, cwd = root, capture = false) {
  if (interrupted) return Promise.reject(new Error("Interrupted"));
  return new Promise((resolve, reject) => {
    const child = spawn(executable, args, { cwd, stdio: capture ? ["inherit", "pipe", "inherit"] : "inherit", windowsHide: true });
    let stdout = "";
    child.stdout?.setEncoding("utf8").on("data", (chunk) => { stdout += chunk; });
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
    // Wait for output to drain: exit can fire before the last captured bytes arrive.
    child.once("close", (code, signal) => {
      process.removeListener("SIGINT", interrupt);
      if (interrupted) reject(new Error("Interrupted"));
      else if (code === 0) resolve(stdout);
      else reject(new Error(`${path.basename(executable)} exited with ${code ?? signal}`));
    });
  });
}

function flagValue(args, ...names) {
  for (const [index, arg] of args.entries()) {
    for (const name of names) {
      if (arg === name) return args[index + 1];
      if (name.startsWith("--") && arg.startsWith(`${name}=`)) return arg.slice(name.length + 1);
    }
  }
  return undefined;
}

const extra = process.argv.slice(3);
// Tauri separates its own flags, Cargo runner flags and application arguments.
const separator = extra.indexOf("--");
const cliArgs = separator < 0 ? extra : extra.slice(0, separator);
const runnerArgs = separator < 0 ? [] : extra.slice(separator + 1);
const appSeparator = runnerArgs.indexOf("--");
const cargoArgs = appSeparator < 0 ? runnerArgs : runnerArgs.slice(0, appSeparator);
// Tauri defaults to release for build and dev for development; child binaries must match.
const explicitProfile = flagValue(cargoArgs, "--profile");
const release = mode === "build"
  ? !(cliArgs.includes("--debug") || cliArgs.includes("-d"))
  : cliArgs.includes("--release") || cargoArgs.includes("--release") || cargoArgs.includes("-r");
// A shipped build inlines across crates (Cargo.toml's release-lto profile).
const profile = explicitProfile ?? (mode === "build" && release ? "release-lto" : release ? "release" : "dev");
const target = flagValue(cliArgs, "--target", "-t") ?? flagValue(cargoArgs, "--target");
const targetArgs = target === undefined ? [] : ["--target", target];
// Cargo resolves relative target directories from cwd, which differs between these builds.
if (process.env.CARGO_TARGET_DIR) process.env.CARGO_TARGET_DIR = path.resolve(root, process.env.CARGO_TARGET_DIR);
const targetDir = flagValue(cargoArgs, "--target-dir");
if (targetDir === "" || (targetDir === undefined && cargoArgs.includes("--target-dir"))) {
  console.error("--target-dir needs a directory.");
  process.exit(2);
}
const targetDirArgs = targetDir === undefined ? [] : ["--target-dir", path.resolve(root, targetDir)];
// Tauri leaves out its own `--release` only for a separate `--profile` argument.
const forwardedCargoArgs = explicitProfile === undefined && profile === "release-lto" ? ["--profile", profile] : [];
for (let index = 0; index < cargoArgs.length; index++) {
  const arg = cargoArgs[index];
  if (arg.startsWith("--profile=")) forwardedCargoArgs.push("--profile", arg.slice("--profile=".length));
  else if (arg.startsWith("--target-dir=")) forwardedCargoArgs.push(...targetDirArgs);
  else if (arg === "--target-dir") { forwardedCargoArgs.push(...targetDirArgs); index++; }
  else forwardedCargoArgs.push(arg);
}
const forwardedRunnerArgs = appSeparator < 0 ? forwardedCargoArgs : [...forwardedCargoArgs, ...runnerArgs.slice(appSeparator)];

try {
  const messages = await run("cargo", ["build", "-p", "viewer", "--profile", profile, ...targetArgs, ...targetDirArgs,
    "--message-format=json-render-diagnostics"], root, true);
  // Sibling binaries must share Cargo's actual output directory, including config overrides.
  const viewer = messages.split("\n").map((line) => { try { return JSON.parse(line); } catch { return null; } })
    .find((message) => message?.reason === "compiler-artifact" && message.target?.name === "viewer"
      && message.target.kind?.includes("bin") && message.executable);
  if (!viewer) throw new Error("Cargo did not report where it built the viewer.");
  const output = path.dirname(viewer.executable);
  // Every viewer uses the same pinned compiler; a development output installs it once unless BATTLE_PLAN_SHADER_COMPILER names one.
  const dxc = path.join(output, "shader-tools", "dxc");
  if (mode === "build" || (!(process.env.BATTLE_PLAN_SHADER_COMPILER ?? process.env.HOI4_SHADER_COMPILER) && !(await access(path.join(dxc, "dxc.exe")).then(() => true, () => false)))) {
    await run("powershell", ["-NoProfile", "-ExecutionPolicy", "Bypass", "-File",
      path.join(root, "tools", "install-shader-compiler.ps1"), "-Destination", dxc]);
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
    ? ["--no-bundle", ...cliArgs, ...(separator < 0 && forwardedRunnerArgs.length === 0 ? [] : ["--", ...forwardedRunnerArgs])]
    : [...cliArgs, "--", ...forwardedRunnerArgs];
  await run(process.execPath, [cli, mode, "--config", hooks, ...args], path.join(root, "crates", "desktop"));
  if (mode === "build") {
    // Battle Plan Recorder ships beside the replay app, embeds the same frontend build
    // and injects the collector from its own folder.
    await run("cargo", ["build", "-p", "recorder-app", "-p", "collector", "--profile", profile,
      ...targetArgs, ...targetDirArgs, "--features", "recorder-app/custom-protocol"]);
    await copyFile(path.join(root, "LICENSE"), path.join(output, "LICENSE"));
    await cp(path.join(root, "LICENSES"), path.join(output, "LICENSES"), { recursive: true });
  }
} catch (error) {
  if (!interrupted) console.error(error instanceof Error ? error.message : String(error));
  process.exitCode = interrupted ? 130 : 1;
}

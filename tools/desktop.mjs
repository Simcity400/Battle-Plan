// One development command builds the native viewer, then starts Tauri and Vite. The build
// command makes the per-user installer that carries Battle Plan, Battle Plan Recorder and what they need.
// No shell interpolation: paths and user arguments remain separate arguments.
import { spawn } from "node:child_process";
import { access, copyFile, mkdir, readFile, readdir, rm, stat } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import path from "node:path";

const root = fileURLToPath(new URL("../", import.meta.url));
const mode = process.argv[2];
let interrupted = false;
if (!["dev", "build"].includes(mode)) {
  console.error("Usage: node tools/desktop.mjs dev|build [--updater-artifacts] [--authenticode]");
  process.exit(2);
}

// Everything the installer carries, and nothing else ships: tools/shipped.json. `main` is what
// `tauri build` makes; `cargo` builds `built` into Cargo's output folder; `tools` ship whole
// from there; `repository` comes from the repository root.
const SHIPPED = JSON.parse(await readFile(path.join(root, "tools", "shipped.json"), "utf8"));

// Tauri embeds its configuration in the app, so resources are named relative to the desktop crate:
// copied into this folder first, no build machine's path reaches the binary.
const STAGE = "stage";

/** Copy every file under `source` into the stage, mapped to where it lands in the install folder. */
async function shipped(source, target, into) {
  if (!(await stat(source)).isDirectory()) {
    const staged = path.join(root, "crates", "desktop", STAGE, target);
    await mkdir(path.dirname(staged), { recursive: true });
    await copyFile(source, staged);
    const name = target.split(path.sep).join("/");
    into[`${STAGE}/${name}`] = name;
    return;
  }
  for (const entry of await readdir(source)) await shipped(path.join(source, entry), path.join(target, entry), into);
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
/** The build's own flags, which neither Tauri nor Cargo sees. */
function own(flag) {
  const at = extra.indexOf(flag);
  if (at < 0 || (extra.includes("--") && at > extra.indexOf("--"))) return false;
  extra.splice(at, 1);
  return true;
}
// The release pipeline signs the installer for the updater, and with a certificate once one is added.
const updaterArtifacts = own("--updater-artifacts");
const authenticode = own("--authenticode");
if (mode === "dev" && (updaterArtifacts || authenticode)) {
  console.error("--updater-artifacts and --authenticode apply to build only.");
  process.exit(2);
}
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
  if (mode === "build") {
    // Battle Plan Recorder embeds the same frontend build, so it is built before any Rust.
    if (!process.env.npm_execpath) throw new Error("Run the build through npm: npm run build:desktop");
    await run(process.execPath, [process.env.npm_execpath, "run", "build:ui"]);
  }
  let builds = [["-p", "viewer"]];
  if (mode === "build") {
    // A shipped Tauri app embeds the frontend it was just given.
    const metadata = JSON.parse(await run("cargo", ["metadata", "--no-deps", "--format-version", "1"], root, true));
    const embeds = new Set(metadata.packages.filter((member) => "custom-protocol" in member.features).map((member) => member.name));
    // One package per Cargo run: each fat-LTO link takes many gigabytes, and the machine is shared,
    // so no two link at once. `tauri build` links Battle Plan after them.
    builds = SHIPPED.cargo.map((name) => ["-p", name, ...(embeds.has(name) ? ["--features", `${name}/custom-protocol`] : [])]);
  }
  let messages = "";
  for (const packages of builds) {
    messages += await run("cargo", ["build", ...packages, "--profile", profile, ...targetArgs, ...targetDirArgs,
      "--message-format=json-render-diagnostics"], root, true);
  }
  // Sibling files must share Cargo's actual output directory, including config overrides.
  const wanted = mode === "build" ? SHIPPED.built : ["viewer.exe"];
  const artifact = messages.split("\n").map((line) => { try { return JSON.parse(line); } catch { return null; } })
    .find((message) => message?.reason === "compiler-artifact"
      && message.filenames?.some((file) => wanted.includes(path.basename(file))));
  if (!artifact) throw new Error(`Cargo did not report where it built ${wanted.join(", ")}.`);
  const output = path.dirname(artifact.filenames.find((file) => wanted.includes(path.basename(file))));
  // Every viewer uses the same pinned compiler; a development output installs it once unless BATTLE_PLAN_SHADER_COMPILER names one.
  const dxc = path.join(output, "shader-tools", "dxc");
  if (mode === "build" ? SHIPPED.tools.includes("shader-tools/dxc")
    : !(process.env.BATTLE_PLAN_SHADER_COMPILER ?? process.env.HOI4_SHADER_COMPILER) && !(await access(path.join(dxc, "dxc.exe")).then(() => true, () => false))) {
    await run("powershell", ["-NoProfile", "-ExecutionPolicy", "Bypass", "-File",
      path.join(root, "tools", "install-shader-compiler.ps1"), "-Destination", dxc]);
  }
  const cli = path.join(root, "node_modules", "@tauri-apps", "cli", "tauri.js");
  // Resolve hook directories at launch without storing machine paths in source.
  // JSON is passed as one argument, never interpreted as shell text.
  let config = { build: {
    beforeDevCommand: { script: "npm run dev:ui", cwd: root },
    beforeBuildCommand: { script: "npm run build:ui", cwd: root },
  } };
  if (mode === "build") {
    const resources = {};
    await rm(path.join(root, "crates", "desktop", STAGE), { recursive: true, force: true });
    for (const name of [...SHIPPED.built, ...SHIPPED.tools]) await shipped(path.join(output, name), name, resources);
    for (const name of SHIPPED.repository) await shipped(path.join(root, name), name, resources);
    // Relative to the desktop crate, where Tauri runs it, for the same reason.
    const signCommand = { cmd: "powershell", args: ["-NoProfile", "-ExecutionPolicy", "Bypass", "-File",
      "../../tools/sign-release.ps1", "%1"] };
    // The frontend is built already; null removes the hook from the merged configuration.
    config = { build: { beforeBuildCommand: null }, bundle: { resources,
      ...(updaterArtifacts ? { createUpdaterArtifacts: true } : {}),
      ...(authenticode ? { windows: { signCommand } } : {}) } };
  }
  // Keep both separators and use the same profile for the host and viewer.
  const args = mode === "build"
    ? [...cliArgs, ...(separator < 0 && forwardedRunnerArgs.length === 0 ? [] : ["--", ...forwardedRunnerArgs])]
    : [...cliArgs, "--", ...forwardedRunnerArgs];
  await run(process.execPath, [cli, mode, "--config", JSON.stringify(config), ...args], path.join(root, "crates", "desktop"));
} catch (error) {
  if (!interrupted) console.error(error instanceof Error ? error.message : String(error));
  process.exitCode = interrupted ? 130 : 1;
}

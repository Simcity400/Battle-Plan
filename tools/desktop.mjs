// One development command starts Tauri and Vite, with the pinned shader compiler beside the app it
// builds. The build command makes the per-user installer that carries Battle Plan, Battle Plan Recorder and
// what they need.
// No shell interpolation: paths and user arguments remain separate arguments.
import { spawn } from "node:child_process";
import { access, copyFile, mkdir, readFile, readdir, rm, stat } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import path from "node:path";
import { cargoTarget } from "./cargo-target.mjs";

const root = fileURLToPath(new URL("../", import.meta.url));
const mode = process.argv[2];
let interrupted = false;
if (!["dev", "build"].includes(mode)) {
  console.error("Usage: node tools/desktop.mjs dev|build [--updater-artifacts] [--authenticode]");
  process.exit(2);
}

// Everything the installer carries, and nothing else ships: tools/shipped.json. `main` is what
// `tauri build` makes; `cargo` builds `built` into Cargo's output folder; `compiler` is the pinned
// DXC (tools/install-shader-compiler.ps1), beside `main` in that folder too; `repository` comes from
// the repository root. Every file but the repository's lands in the install folder itself.
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
// Where Cargo puts the app, which a target from overrides, the environment or Cargo's config moves too.
const builtFor = target ?? await cargoTarget({ cwd: path.join(root, "crates", "desktop"), env: process.env, cargoArgs,
  home: process.env.CARGO_HOME ?? path.join(process.env.USERPROFILE ?? process.env.HOME ?? "", ".cargo") });
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
  const metadata = JSON.parse(await run("cargo", ["metadata", "--no-deps", "--format-version", "1"], root, true));
  // Cargo's actual output folder, where `tauri` puts replay-desktop.exe, including config overrides.
  let output = path.join(targetDir === undefined ? metadata.target_directory : path.resolve(root, targetDir),
    ...(builtFor === undefined ? [] : [builtFor]), profile === "dev" ? "debug" : profile);
  if (mode === "build") {
    // A shipped Tauri app embeds the frontend it was just given.
    const embeds = new Set(metadata.packages.filter((member) => "custom-protocol" in member.features).map((member) => member.name));
    // One package per Cargo run: each fat-LTO link takes many gigabytes, and the machine is shared,
    // so no two link at once. `tauri build` links Battle Plan after them.
    let messages = "";
    for (const name of SHIPPED.cargo) {
      messages += await run("cargo", ["build", "-p", name, ...(embeds.has(name) ? ["--features", `${name}/custom-protocol`] : []),
        "--profile", profile, ...targetArgs, ...targetDirArgs, "--message-format=json-render-diagnostics"], root, true);
    }
    const artifact = messages.split("\n").map((line) => { try { return JSON.parse(line); } catch { return null; } })
      .find((message) => message?.reason === "compiler-artifact"
        && message.filenames?.some((file) => SHIPPED.built.includes(path.basename(file))));
    if (!artifact) throw new Error(`Cargo did not report where it built ${SHIPPED.built.join(", ")}.`);
    output = path.dirname(artifact.filenames.find((file) => SHIPPED.built.includes(path.basename(file))));
  }
  // The app loads the pinned compiler from beside itself; a development output fetches it once.
  const present = await Promise.all(SHIPPED.compiler.map((name) => access(path.join(output, name)).then(() => true, () => false)));
  if (mode === "build" || present.includes(false)) {
    await run("powershell", ["-NoProfile", "-ExecutionPolicy", "Bypass", "-File",
      path.join(root, "tools", "install-shader-compiler.ps1"), "-Destination", output]);
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
    for (const name of [...SHIPPED.built, ...SHIPPED.compiler]) await shipped(path.join(output, name), name, resources);
    for (const name of SHIPPED.repository) await shipped(path.join(root, name), name, resources);
    // Relative to the desktop crate, where Tauri runs it, for the same reason.
    const signCommand = { cmd: "powershell", args: ["-NoProfile", "-ExecutionPolicy", "Bypass", "-File",
      "../../tools/sign-release.ps1", "%1"] };
    // The frontend is built already; null removes the hook from the merged configuration.
    config = { build: { beforeBuildCommand: null }, bundle: { resources,
      ...(updaterArtifacts ? { createUpdaterArtifacts: true } : {}),
      ...(authenticode ? { windows: { signCommand } } : {}) } };
  }
  // Keep both separators and use the same profile as the binaries built above.
  const args = mode === "build"
    ? [...cliArgs, ...(separator < 0 && forwardedRunnerArgs.length === 0 ? [] : ["--", ...forwardedRunnerArgs])]
    : [...cliArgs, "--", ...forwardedRunnerArgs];
  await run(process.execPath, [cli, mode, "--config", JSON.stringify(config), ...args], path.join(root, "crates", "desktop"));
} catch (error) {
  if (!interrupted) console.error(error instanceof Error ? error.message : String(error));
  process.exitCode = interrupted ? 130 : 1;
}

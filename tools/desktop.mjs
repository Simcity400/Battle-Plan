import { spawn } from "node:child_process";
import { access, copyFile, mkdir, readFile, readdir, rm, stat, writeFile } from "node:fs/promises";
import { fileURLToPath, pathToFileURL } from "node:url";
import path from "node:path";
import { readHostTuple, resolveCargoTarget, resolveTargetPlacement } from "./cargo-target.mjs";
import { thirdPartyNotices } from "./third-party-notices.mjs";

const root = fileURLToPath(new URL("../", import.meta.url));
const STAGE = "stage";
const VALUE_FLAGS = new Set(["--config", "--profile", "--target", "-t", "--target-dir"]);

function flagValues(args, ...names) {
  const wanted = new Set(names);
  const values = [];
  for (let index = 0; index < args.length; index++) {
    const arg = args[index];
    if (arg === "--") break;
    if (wanted.has(arg)) {
      if (index + 1 >= args.length) throw Object.assign(new Error(`${arg} needs a value.`), { exitCode: 2 });
      values.push(args[++index]);
      continue;
    }
    let matched = false;
    for (const name of names) {
      if (name.startsWith("--") && arg.startsWith(`${name}=`)) {
        values.push(arg.slice(name.length + 1));
        matched = true;
        break;
      }
    }
    if (!matched && VALUE_FLAGS.has(arg) && index + 1 < args.length) index++;
  }
  return values;
}

export function metadataArgs(plan) {
  return ["metadata", "--no-deps", "--format-version", "1", ...plan.configArgs];
}

export function resolveTargetDir(plan, metadataTargetDirectory) {
  return plan.targetDir ?? metadataTargetDirectory;
}

export function stageOutputDir({ targetDir, outputDirName, profile }) {
  return path.join(targetDir, ...(outputDirName === undefined ? [] : [outputDirName]), profile === "dev" ? "debug" : profile);
}

export function companionBuildArgs({ name, embeds, profile, targetArgs, targetDirArgs }) {
  return [
    "build",
    "-p",
    name,
    ...(embeds.has(name) ? ["--features", `${name}/custom-protocol`] : []),
    "--profile",
    profile,
    ...targetArgs,
    ...targetDirArgs,
    "--message-format=json-render-diagnostics",
  ];
}

export async function planDesktopInvocation({ mode, argv, root: workspaceRoot = root, env = process.env, hostTarget }) {
  const separator = argv.indexOf("--");
  const cliArgs = separator < 0 ? argv : argv.slice(0, separator);
  const runnerArgs = separator < 0 ? [] : argv.slice(separator + 1);
  const appSeparator = runnerArgs.indexOf("--");
  const cargoArgs = appSeparator < 0 ? runnerArgs : runnerArgs.slice(0, appSeparator);
  const explicitProfiles = flagValues(cargoArgs, "--profile");
  const explicitProfile = explicitProfiles.length > 0 ? explicitProfiles[explicitProfiles.length - 1] : undefined;
  const release = mode === "build"
    ? !(cliArgs.includes("--debug") || cliArgs.includes("-d"))
    : cliArgs.includes("--release") || cargoArgs.includes("--release") || cargoArgs.includes("-r");
  const profile = explicitProfile ?? (mode === "build" && release ? "release-lto" : release ? "release" : "dev");

  const configArgs = [];
  for (let index = 0; index < cargoArgs.length; index++) {
    const arg = cargoArgs[index];
    if (arg === "--") break;
    if (arg === "--config") {
      if (index + 1 >= cargoArgs.length) throw Object.assign(new Error("--config needs a value."), { exitCode: 2 });
      configArgs.push("--config", cargoArgs[++index]);
    } else if (arg.startsWith("--config=")) {
      configArgs.push("--config", arg.slice("--config=".length));
    } else if (VALUE_FLAGS.has(arg) && index + 1 < cargoArgs.length) {
      index++;
    }
  }

  const desktopDir = path.join(workspaceRoot, "crates", "desktop");
  const explicitTargets = [...flagValues(cliArgs, "--target", "-t"), ...flagValues(cargoArgs, "--target")];
  if (explicitTargets.length > 1) {
    throw new Error("--target names multiple build targets this command cannot place; pass a single --target.");
  }
  const homeRoot = env.USERPROFILE ?? env.HOME;
  const home = env.CARGO_HOME || (homeRoot ? path.join(homeRoot, ".cargo") : undefined);
  const configured = explicitTargets.length === 1
    ? { target: explicitTargets[0], source: "--target" }
    : await resolveCargoTarget({ cwd: desktopDir, env, cargoArgs, home });
  const placement = configured === undefined
    ? undefined
    : await resolveTargetPlacement({ ...configured, hostTarget: hostTarget ?? (() => readHostTuple(env)) });
  const targetArgs = placement === undefined ? [] : ["--target", placement.targetArg];

  const targetDirs = flagValues(cargoArgs, "--target-dir");
  const targetDir = targetDirs.length > 0 ? targetDirs[targetDirs.length - 1] : undefined;
  if (targetDir === "") {
    throw Object.assign(new Error("--target-dir needs a directory."), { exitCode: 2 });
  }
  const resolvedTargetDir = targetDir === undefined ? undefined : path.resolve(workspaceRoot, targetDir);
  const targetDirArgs = resolvedTargetDir === undefined ? [] : ["--target-dir", resolvedTargetDir];

  const tauriCliArgs = [];
  let cliHadTarget = false;
  for (let index = 0; index < cliArgs.length; index++) {
    const arg = cliArgs[index];
    if (placement !== undefined && (arg === "--target" || arg === "-t")) {
      tauriCliArgs.push("--target", placement.targetArg);
      cliHadTarget = true;
      index++;
    } else if (placement !== undefined && arg.startsWith("--target=")) {
      tauriCliArgs.push("--target", placement.targetArg);
      cliHadTarget = true;
    } else if (VALUE_FLAGS.has(arg) && index + 1 < cliArgs.length) {
      tauriCliArgs.push(arg, cliArgs[++index]);
    } else {
      tauriCliArgs.push(arg);
    }
  }
  if (placement !== undefined && !cliHadTarget) {
    tauriCliArgs.push("--target", placement.targetArg);
  }

  const forwardedCargoArgs = explicitProfile === undefined && profile === "release-lto" ? ["--profile", profile] : [];
  for (let index = 0; index < cargoArgs.length; index++) {
    const arg = cargoArgs[index];
    if (arg === "--config" && index + 1 < cargoArgs.length) {
      forwardedCargoArgs.push(arg, cargoArgs[++index]);
    } else if (arg.startsWith("--profile=")) {
      forwardedCargoArgs.push("--profile", arg.slice("--profile=".length));
    } else if (arg.startsWith("--target-dir=")) {
      forwardedCargoArgs.push(...targetDirArgs);
    } else if (arg === "--target-dir") {
      forwardedCargoArgs.push(...targetDirArgs);
      index++;
    } else if (arg.startsWith("--target=")) {
      continue;
    } else if (arg === "--target") {
      index++;
    } else {
      forwardedCargoArgs.push(arg);
    }
  }
  const forwardedRunnerArgs = appSeparator < 0 ? forwardedCargoArgs : [...forwardedCargoArgs, ...runnerArgs.slice(appSeparator)];
  const tauriArgs = mode === "build"
    ? [...tauriCliArgs, ...(separator < 0 && forwardedRunnerArgs.length === 0 ? [] : ["--", ...forwardedRunnerArgs])]
    : [...tauriCliArgs, "--", ...forwardedRunnerArgs];

  return {
    profile,
    targetArgs,
    outputDirName: placement?.outputDirName,
    targetDir: resolvedTargetDir,
    targetDirArgs,
    configArgs,
    metadataCwd: desktopDir,
    forwardedRunnerArgs,
    tauriArgs,
  };
}

let interrupted = false;

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
      if (process.platform !== "win32") child.kill("SIGINT");
    };
    process.once("SIGINT", interrupt);
    child.once("error", (error) => {
      process.removeListener("SIGINT", interrupt);
      reject(error);
    });
    child.once("close", (code, signal) => {
      process.removeListener("SIGINT", interrupt);
      if (interrupted) reject(new Error("Interrupted"));
      else if (code === 0) resolve(stdout);
      else reject(new Error(`${path.basename(executable)} exited with ${code ?? signal}`));
    });
  });
}

async function main() {
  const mode = process.argv[2];
  if (!["dev", "build"].includes(mode)) {
    console.error("Usage: node tools/desktop.mjs dev|build");
    process.exit(2);
  }
  const SHIPPED = JSON.parse(await readFile(path.join(root, "tools", "shipped.json"), "utf8"));
  const plan = await planDesktopInvocation({ mode, argv: process.argv.slice(3), root, env: process.env });
  if (process.env.CARGO_TARGET_DIR) process.env.CARGO_TARGET_DIR = path.resolve(root, process.env.CARGO_TARGET_DIR);

  const notices = mode === "build" ? await thirdPartyNotices(path.join(root, "LICENSES"), SHIPPED.notices.components) : null;
  const releaseVersion = mode === "build" ? process.env.BATTLEPLAN_VERSION || undefined : undefined;
  if (releaseVersion) process.env.TAURI_CONFIG = JSON.stringify({ version: releaseVersion });
  if (mode === "build") {
    if (!process.env.npm_execpath) throw new Error("Run the build through npm: npm run build:desktop");
    await run(process.execPath, [process.env.npm_execpath, "run", "build:ui"]);
  }
  const metadata = JSON.parse(await run("cargo", metadataArgs(plan), plan.metadataCwd, true));
  const effectiveTargetDir = resolveTargetDir(plan, metadata.target_directory);
  process.env.CARGO_TARGET_DIR = effectiveTargetDir;
  let output = stageOutputDir({
    targetDir: effectiveTargetDir,
    outputDirName: plan.outputDirName,
    profile: plan.profile,
  });
  if (mode === "build") {
    const embeds = new Set(metadata.packages.filter((member) => "custom-protocol" in member.features).map((member) => member.name));
    let messages = "";
    for (const name of SHIPPED.cargo) {
      messages += await run("cargo", companionBuildArgs({
        name,
        embeds,
        profile: plan.profile,
        targetArgs: plan.targetArgs,
        targetDirArgs: ["--target-dir", effectiveTargetDir],
      }), root, true);
    }
    const artifact = messages.split("\n").map((line) => { try { return JSON.parse(line); } catch { return null; } })
      .find((message) => message?.reason === "compiler-artifact"
        && message.filenames?.some((file) => SHIPPED.built.includes(path.basename(file))));
    if (!artifact) throw new Error(`Cargo did not report where it built ${SHIPPED.built.join(", ")}.`);
    output = path.dirname(artifact.filenames.find((file) => SHIPPED.built.includes(path.basename(file))));
  }
  const present = await Promise.all(SHIPPED.compiler.map((name) => access(path.join(output, name)).then(() => true, () => false)));
  if (mode === "build" || present.includes(false)) {
    await run("powershell", ["-NoProfile", "-ExecutionPolicy", "Bypass", "-File",
      path.join(root, "tools", "install-shader-compiler.ps1"), "-Destination", output]);
  }
  const cli = path.join(root, "node_modules", "@tauri-apps", "cli", "tauri.js");
  let config = { build: {
    beforeDevCommand: { script: "npm run dev:ui", cwd: root },
    beforeBuildCommand: { script: "npm run build:ui", cwd: root },
  } };
  if (mode === "build") {
    const resources = {};
    await rm(path.join(root, "crates", "desktop", STAGE), { recursive: true, force: true });
    for (const name of [...SHIPPED.built, ...SHIPPED.compiler]) await shipped(path.join(output, name), name, resources);
    for (const name of SHIPPED.repository) await shipped(path.join(root, name), name, resources);
    await writeFile(path.join(root, "crates", "desktop", STAGE, SHIPPED.notices.file), notices);
    resources[`${STAGE}/${SHIPPED.notices.file}`] = SHIPPED.notices.file;
    const desktop = metadata.packages.find((member) => path.relative(path.join(root, "crates", "desktop"), path.dirname(member.manifest_path)) === "");
    if (!desktop) throw new Error("Cargo metadata names no package in crates/desktop.");
    const nightly = (releaseVersion ?? desktop.version).includes("-nightly.");
    config = { ...(releaseVersion ? { version: releaseVersion } : {}), build: { beforeBuildCommand: null }, bundle: { resources,
      ...(nightly ? { icon: ["icons/nightly.ico", "icons/nightly.png"], windows: { nsis: { installerIcon: "icons/nightly.ico" } } } : {}) } };
  }
  await run(process.execPath, [cli, mode, "--config", JSON.stringify(config), ...plan.tauriArgs], path.join(root, "crates", "desktop"));
}

if (process.argv[1] && pathToFileURL(path.resolve(process.argv[1])).href === import.meta.url) {
  try {
    await main();
  } catch (error) {
    if (!interrupted) console.error(error instanceof Error ? error.message : String(error));
    process.exitCode = interrupted ? 130 : error?.exitCode ?? 1;
  }
}

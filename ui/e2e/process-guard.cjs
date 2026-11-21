const cp = require('node:child_process'), fs = require('node:fs');
const { promisify } = require('node:util');
if (!(process.env.BATTLEPLAN_OWNED_JOB ?? process.env.HOI4_OWNED_JOB)) throw Error('Launch through owned_job.py');
function check(command, args) {
  if (/taskkill(?:\.exe)?/i.test([command, ...args.filter(Array.isArray).flat()].join(' '))) {
    fs.writeFileSync((process.env.BATTLEPLAN_JOB_STOP_FILE ?? process.env.HOI4_JOB_STOP_FILE), 'Playwright PID tree-kill disabled');
    throw Error('PID tree-kill disabled; terminating the owned Job');
  }
}
for (const name of ['spawn', 'spawnSync', 'exec', 'execSync', 'execFile', 'execFileSync']) {
  const original = cp[name];
  cp[name] = function(command, ...args) { check(command, args); return original.call(this, command, ...args); };
  if (original[promisify.custom]) cp[name][promisify.custom] = function(command, ...args) {
    check(command, args); return original[promisify.custom].call(this, command, ...args);
  };
}
const probe = process.kill.bind(process);
process.kill = (pid, signal) => { if (signal === 0) return probe(pid, 0); throw Error('PID-only termination disabled'); };
require('node:module').syncBuiltinESMExports();

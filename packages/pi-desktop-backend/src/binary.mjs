// Entry of the standalone build (scripts/build-binary.mjs): pi's own Bun setup,
// as in pi's release binary, then the backend.
import '../node_modules/@earendil-works/pi-coding-agent/dist/bun/sandbox-env-setup.js';
import '../node_modules/@earendil-works/pi-coding-agent/dist/bun/runtime-setup.js';
import './cli.mjs';

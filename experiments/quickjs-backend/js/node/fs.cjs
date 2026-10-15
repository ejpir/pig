// Node fs has a mutable CommonJS export (graceful-fs patches it at import time).
// The actual authority remains the closed native asset mount.
module.exports = {...require('./fs.js').default};

// The compiler is kept in the prototype until its ABI is stable. This shim
// proves that framework-owned discovery and loader orchestration reach it.
module.exports = require(process.env.NEXT_RSC_PROTOTYPE_LOADER)

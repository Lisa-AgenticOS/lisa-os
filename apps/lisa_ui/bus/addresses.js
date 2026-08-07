// Where each system interface lives — pure data, importable from node
// tests (no gi:// anywhere on this path; the Gio-using factory is
// ./proxy.js).
//
// `Remote1` is the ONE exception where the well-known name differs
// from the interface name: the broker owns `dev.lisaos.Remoted` and
// serves `dev.lisaos.Remote1` on it — a mismatch that has already cost
// debugging time when a surface guessed the name from the interface
// (which works for every other row here).
export const BUS = {
    Overlay1: {name: 'dev.lisaos.Overlay1', path: '/dev/lisaos/Overlay1'},
    Voice1: {name: 'dev.lisaos.Voice1', path: '/dev/lisaos/Voice1'},
    Harness1: {name: 'dev.lisaos.Harness1', path: '/dev/lisaos/Harness1'},
    Agent1: {name: 'dev.lisaos.Agent1', path: '/dev/lisaos/Agent1'},
    Context1: {name: 'dev.lisaos.Context1', path: '/dev/lisaos/Context1'},
    Inference1: {name: 'dev.lisaos.Inference1', path: '/dev/lisaos/Inference1'},
    Remote1: {name: 'dev.lisaos.Remoted', path: '/dev/lisaos/Remote1'},
};

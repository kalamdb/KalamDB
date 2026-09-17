// Resizable/growable stores use V8's virtual-memory allocator rather than the
// bounded ArrayBuffer allocator. Expose fixed backing stores only.
(() => {
  const construct = Reflect.construct;
  const define = Object.defineProperty;
  for (const name of ["ArrayBuffer", "SharedArrayBuffer"]) {
    const Native = globalThis[name];
    if (typeof Native !== "function") continue;
    const Fixed = new Proxy(Native, {
      construct(target, args, newTarget) {
        if (args.length > 1 && args[1] !== undefined) {
          throw new RangeError("Resizable and growable buffers are unavailable in procedures");
        }
        return construct(target, [args[0]], newTarget);
      },
    });
    // Also protect constructors recovered from typed-array buffer prototypes.
    define(Native.prototype, "constructor", {value: Fixed, writable: false, configurable: false});
    define(globalThis, name, {value: Fixed, writable: false, configurable: false});
  }
})();

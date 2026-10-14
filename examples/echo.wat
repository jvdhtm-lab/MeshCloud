;; A MeshCloud WASM job that echoes its input back deterministically. Useful
;; for verifying that the sandbox returns reproducible results for given input.

(module
  (memory (export "memory") 4)
  (func (export "alloc") (param i32) (result i32) (i32.const 1024))
  (func (export "run") (param $ptr i32) (param $len i32) (result i32)
    (local $out i32)
    (local.set $out (i32.const 65536))
    (memory.copy (local.get $out) (local.get $ptr) (local.get $len))
    (i32.store8 (i32.add (local.get $out) (local.get $len)) (i32.const 0))
    (local.get $out)))

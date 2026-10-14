;; A deliberately non-terminating MeshCloud WASM job. Used to verify that the
;; sandbox terminates runaway jobs via fuel metering or the wall-clock timeout.

(module
  (memory (export "memory") 1)
  (func (export "alloc") (param i32) (result i32) (i32.const 0))
  (func (export "run") (param i32 i32) (result i32)
    (loop $l (br $l))
    (i32.const 0)))

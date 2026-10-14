;; A minimal MeshCloud WASM job that ignores its input and returns a fixed
;; greeting. The MeshCloud ABI requires exports: `memory`, `alloc`, `run`.

(module
  (memory (export "memory") 1)
  (data (i32.const 16) "hello from meshcloud\00")
  (func (export "alloc") (param i32) (result i32) (i32.const 1024))
  (func (export "run") (param i32 i32) (result i32) (i32.const 16)))

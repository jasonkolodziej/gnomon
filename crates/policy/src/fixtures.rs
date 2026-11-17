//! Hand-written priority components in the component text format, for tests
//! here and in downstream crates. Real plugins are compiled from a source
//! language (see `examples/priority-plugin`); these exist so the host can be
//! tested without a WASM toolchain.
//!
//! The core function receives the canonical-ABI flattening of
//! `priority(job, now)`, in this order:
//!
//! | local | field | type |
//! |---|---|---|
//! | 0 | `id` | i64 |
//! | 1, 2 | `user` (ptr, len) | i32, i32 |
//! | 3, 4 | `partition` (ptr, len) | i32, i32 |
//! | 5 | `cores` | i32 |
//! | 6 | `mem-mb` | i64 |
//! | 7 | `gpus` | i32 |
//! | 8 | `nodes` | i32 |
//! | 9 | `walltime-secs` | i64 |
//! | 10 | `submit-time` | i64 |
//! | 11 | `now` | i64 |
//!
//! and returns the priority as an i64.

/// A `priority` component whose core function body is `body` (WAT
/// instructions over the locals above, leaving one i64), with `pages` pages
/// of initial linear memory.
pub fn priority_component(body: &str, pages: u32) -> String {
    format!(
        r#"(component
  (import "gnomon:policy/types@0.1.0" (instance $types
    (type $j (record
      (field "id" u64) (field "user" string) (field "partition" string)
      (field "cores" u32) (field "mem-mb" u64) (field "gpus" u32) (field "nodes" u32)
      (field "walltime-secs" u64) (field "submit-time" u64)))
    (export "job" (type (eq $j)))))
  (alias export $types "job" (type $job))
  (core module $m
    (memory (export "memory") {pages})
    (global $bump (mut i32) (i32.const 1024))
    (func (export "cabi_realloc") (param i32 i32 i32 i32) (result i32)
      (local $p i32)
      (local.set $p (global.get $bump))
      (global.set $bump (i32.add (global.get $bump) (local.get 3)))
      (local.get $p))
    (func (export "priority")
      (param i64 i32 i32 i32 i32 i32 i64 i32 i32 i64 i64 i64) (result i64)
      {body}))
  (core instance $i (instantiate $m))
  (func $priority (param "job" $job) (param "now" u64) (result u64)
    (canon lift (core func $i "priority")
      (memory (core memory $i "memory")) (realloc (core func $i "cabi_realloc"))))
  (export "priority" (func $priority)))"#
    )
}

/// Size times nodes plus minutes waited:
/// `(cores + 4 * gpus) * nodes * 100 + (now - submit-time) / 60`.
pub fn size_age() -> String {
    priority_component(
        "(i64.add
           (i64.mul
             (i64.extend_i32_u (i32.mul
               (i32.add (local.get 5) (i32.mul (i32.const 4) (local.get 7)))
               (local.get 8)))
             (i64.const 100))
           (i64.div_u (i64.sub (local.get 11) (local.get 10)) (i64.const 60)))",
        1,
    )
}

/// Reads the strings: `first byte of user * 1000 + partition length`.
pub fn strings() -> String {
    priority_component(
        "(i64.add
           (i64.mul (i64.load8_u (local.get 1)) (i64.const 1000))
           (i64.extend_i32_u (local.get 4)))",
        1,
    )
}

/// Never returns.
pub fn infinite_loop() -> String {
    priority_component("(loop $l (br $l)) (i64.const 0)", 1)
}

/// Grows memory by 1 GiB, past any sensible cap.
pub fn memory_bomb() -> String {
    priority_component("(drop (memory.grow (i32.const 16384))) (i64.const 0)", 1)
}

/// Asks for 64 MiB of initial memory, refused at instantiation.
pub fn big_initial_memory() -> String {
    priority_component("(i64.const 0)", 1024)
}

/// Traps immediately.
pub fn trap() -> String {
    priority_component("(unreachable)", 1)
}

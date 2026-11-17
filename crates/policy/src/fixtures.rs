//! Hand-written policy components in the component text format, for tests
//! here and in downstream crates. Real plugins are compiled from a source
//! language (see `examples/policy-plugins`); these exist so the host can be
//! tested without a WASM toolchain.
//!
//! Every world's core function receives the canonical-ABI flattening of the
//! job first:
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
//!
//! followed by the world's other parameters: `now` (i64, local 11) for
//! `priority` and `admission`; nothing for `estimator`; the candidate list
//! (ptr, len: locals 11, 12) and `now` (local 13) for `node-score`.
//!
//! Results that do not flatten to one value (`verdict`, `option<u64>`,
//! `list<s64>`) are written to a return area at [`RET`] and the function
//! returns its address. Plugin memory below [`HEAP`] is free for fixtures;
//! the host's arguments are allocated from `HEAP` upwards.

/// Return-area address used by the fixtures.
pub const RET: u32 = 512;
/// Where `cabi_realloc` starts allocating.
pub const HEAP: u32 = 32 * 1024;

const JOB_PARAMS: &str = "i64 i32 i32 i32 i32 i32 i64 i32 i32 i64 i64";

/// A component for `gnomon:policy@0.1.0` with `pages` pages of memory,
/// core functions `core` (added to a core module that already exports
/// `memory` and a bump-allocating `cabi_realloc`) and component-level
/// lifts and exports `lift`. Lifts can name the types `$job`, `$verdict`
/// and `$node`, and write `{opts}` for the memory and realloc options.
pub fn component(pages: u32, core: &str, lift: &str) -> String {
    let opts = r#"(memory (core memory $i "memory")) (realloc (core func $i "cabi_realloc"))"#;
    let lift = lift.replace("{opts}", opts);
    format!(
        r#"(component
  (import "gnomon:policy/types@0.1.0" (instance $types
    (type $j (record
      (field "id" u64) (field "user" string) (field "partition" string)
      (field "cores" u32) (field "mem-mb" u64) (field "gpus" u32) (field "nodes" u32)
      (field "walltime-secs" u64) (field "submit-time" u64)))
    (export "job" (type (eq $j)))
    (type $r (record
      (field "partition" string) (field "cores" u32) (field "mem-mb" u64)
      (field "gpus" u32) (field "nodes" u32) (field "walltime-secs" u64)))
    (export "request" (type $re (eq $r)))
    (type $v (variant (case "accept") (case "reject" string) (case "rewrite" $re)))
    (export "verdict" (type (eq $v)))
    (type $n (record
      (field "name" string) (field "cores" u32) (field "mem-mb" u64) (field "gpus" u32)
      (field "free-cores" u32) (field "free-mem-mb" u64) (field "free-gpus" u32)))
    (export "node" (type (eq $n)))))
  (alias export $types "job" (type $job))
  (alias export $types "verdict" (type $verdict))
  (alias export $types "node" (type $node))
  (core module $m
    (memory (export "memory") {pages})
    (global $bump (mut i32) (i32.const {HEAP}))
    (func (export "cabi_realloc") (param i32 i32 i32 i32) (result i32)
      (local $p i32)
      ;; align up to 8, then bump
      (local.set $p (i32.and (i32.add (global.get $bump) (i32.const 7)) (i32.const -8)))
      (global.set $bump (i32.add (local.get $p) (local.get 3)))
      (local.get $p))
    {core})
  (core instance $i (instantiate $m))
  {lift})"#
    )
}

// ---------------------------------------------------------------- priority

/// A `priority` component whose core function body is `body` (WAT
/// instructions over the locals above, leaving one i64), with `pages` pages
/// of initial linear memory.
pub fn priority_component(body: &str, pages: u32) -> String {
    component(
        pages,
        &format!(
            r#"(func (export "priority") (param {JOB_PARAMS} i64) (result i64)
      {body})"#
        ),
        r#"(func $f (param "job" $job) (param "now" u64) (result u64)
    (canon lift (core func $i "priority") {opts}))
  (export "priority" (func $f))"#,
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

// --------------------------------------------------------------- admission

/// An `admission` component whose core body leaves the address of a
/// `verdict` (written at [`RET`]) on the stack. `data` adds data segments.
///
/// `verdict` layout: discriminant (u8) at 0, payload at 8. `reject`'s
/// string is (ptr, len) at 8 and 12. `rewrite`'s request: partition (ptr,
/// len) at 8 and 12, cores at 16, mem-mb at 24, gpus at 32, nodes at 36,
/// walltime-secs at 40.
pub fn admission_component(body: &str, data: &str) -> String {
    component(
        1,
        &format!(
            r#"{data}
    (func (export "admit") (param {JOB_PARAMS} i64) (result i32)
      {body})"#
        ),
        r#"(func $f (param "job" $job) (param "now" u64) (result $verdict)
    (canon lift (core func $i "admit") {opts}))
  (export "admit" (func $f))"#,
    )
}

/// Rejects jobs using more than 64 cores in total ("too many cores"),
/// moves GPU jobs to partition `gpu`, accepts everything else.
pub fn admission_rules() -> String {
    admission_component(
        r#"(if (i32.gt_u (i32.mul (local.get 5) (local.get 8)) (i32.const 64))
        (then
          (i32.store8 (i32.const 512) (i32.const 1))
          (i32.store (i32.const 520) (i32.const 256))
          (i32.store (i32.const 524) (i32.const 14))
          (return (i32.const 512))))
      (if (i32.gt_u (local.get 7) (i32.const 0))
        (then
          (i32.store8 (i32.const 512) (i32.const 2))
          (i32.store (i32.const 520) (i32.const 288))
          (i32.store (i32.const 524) (i32.const 3))
          (i32.store (i32.const 528) (local.get 5))
          (i64.store (i32.const 536) (local.get 6))
          (i32.store (i32.const 544) (local.get 7))
          (i32.store (i32.const 548) (local.get 8))
          (i64.store (i32.const 552) (local.get 9))
          (return (i32.const 512))))
      (i32.store8 (i32.const 512) (i32.const 0))
      (i32.const 512)"#,
        r#"(data (i32.const 256) "too many cores")
    (data (i32.const 288) "gpu")"#,
    )
}

/// Rewrites every job to zero nodes, which the host must refuse.
pub fn admission_bad_rewrite() -> String {
    admission_component(
        r#"(i32.store8 (i32.const 512) (i32.const 2))
      (i32.store (i32.const 520) (i32.const 288))
      (i32.store (i32.const 524) (i32.const 3))
      (i32.store (i32.const 528) (i32.const 1))
      (i32.store (i32.const 548) (i32.const 0))
      (i64.store (i32.const 552) (i64.const 60))
      (i32.const 512)"#,
        r#"(data (i32.const 288) "gpu")"#,
    )
}

/// An admission plugin that traps.
pub fn admission_trap() -> String {
    admission_component("(unreachable)", "")
}

// --------------------------------------------------------------- estimator

/// An `estimator` component whose core body leaves the address of an
/// `option<u64>` (discriminant u8 at [`RET`], value at `RET + 8`).
pub fn estimator_component(body: &str) -> String {
    component(
        1,
        &format!(
            r#"(func (export "estimate") (param {JOB_PARAMS}) (result i32)
      {body})"#
        ),
        r#"(func $f (param "job" $job) (result (option u64))
    (canon lift (core func $i "estimate") {opts}))
  (export "estimate" (func $f))"#,
    )
}

/// Half the walltime for jobs over 10 minutes, `none` for the rest.
pub fn half_walltime() -> String {
    estimator_component(
        r#"(if (i64.gt_u (local.get 9) (i64.const 600))
        (then
          (i32.store8 (i32.const 512) (i32.const 1))
          (i64.store (i32.const 520) (i64.div_u (local.get 9) (i64.const 2)))
          (return (i32.const 512))))
      (i32.store8 (i32.const 512) (i32.const 0))
      (i32.const 512)"#,
    )
}

/// Always `some(secs)`, to test clamping.
pub fn constant_estimate(secs: u64) -> String {
    estimator_component(&format!(
        r#"(i32.store8 (i32.const 512) (i32.const 1))
      (i64.store (i32.const 520) (i64.const {secs}))
      (i32.const 512)"#
    ))
}

/// An estimator that traps.
pub fn estimator_trap() -> String {
    estimator_component("(unreachable)")
}

// -------------------------------------------------------------- node-score

/// A `node-score` component whose core body leaves the address of a
/// `list<s64>` ((ptr, len) at [`RET`]). Each candidate `node` is 48 bytes:
/// name (ptr, len) at 0 and 4, cores at 8, mem-mb at 16, gpus at 24,
/// free-cores at 28, free-mem-mb at 32, free-gpus at 40. Four pages, so
/// about a thousand candidates fit.
pub fn node_score_component(body: &str) -> String {
    component(
        4,
        &format!(
            r#"(func (export "score") (param {JOB_PARAMS} i32 i32 i64) (result i32)
      {body})"#
        ),
        r#"(func $f (param "job" $job) (param "candidates" (list $node)) (param "now" u64)
    (result (list s64))
    (canon lift (core func $i "score") {opts}))
  (export "score" (func $f))"#,
    )
}

/// Scores candidate `$k` (at address `$c`) with the i64 expression `expr`
/// into an array at 4096, and returns `len` scores.
fn score_each(expr: &str, len: &str) -> String {
    node_score_component(&format!(
        r#"(local $k i32) (local $c i32)
      (block $done (loop $next
        (br_if $done (i32.ge_u (local.get $k) (local.get 12)))
        (local.set $c (i32.add (local.get 11) (i32.mul (local.get $k) (i32.const 48))))
        (i64.store (i32.add (i32.const 4096) (i32.shl (local.get $k) (i32.const 3))) {expr})
        (local.set $k (i32.add (local.get $k) (i32.const 1)))
        (br $next)))
      (i32.store (i32.const 512) (i32.const 4096))
      (i32.store (i32.const 516) {len})
      (i32.const 512)"#
    ))
}

/// Spread: prefer the nodes with the most free cores.
pub fn most_free_cores() -> String {
    score_each(
        "(i64.extend_i32_u (i32.load offset=28 (local.get $c)))",
        "(local.get 12)",
    )
}

/// Pack: prefer the nodes with the fewest free cores.
pub fn fewest_free_cores() -> String {
    score_each(
        "(i64.sub (i64.const 0) (i64.extend_i32_u (i32.load offset=28 (local.get $c))))",
        "(local.get 12)",
    )
}

/// Prefers nodes whose name ends in `b` (reads the candidates' names).
pub fn prefer_name_ending_b() -> String {
    score_each(
        "(i64.extend_i32_u (i32.eq (i32.const 98) (i32.load8_u (i32.sub
           (i32.add (i32.load (local.get $c)) (i32.load offset=4 (local.get $c)))
           (i32.const 1)))))",
        "(local.get 12)",
    )
}

/// Returns one score too few.
pub fn wrong_length() -> String {
    score_each("(i64.const 1)", "(i32.sub (local.get 12) (i32.const 1))")
}

/// A node-score plugin that traps.
pub fn node_score_trap() -> String {
    node_score_component("(unreachable)")
}

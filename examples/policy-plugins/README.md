# Example policy plugins

Gnomon policy plugins written in Rust against the WIT package
[`gnomon:policy`](../../crates/policy/wit/policy.wit). There is one plugin per world, each behind a
cargo feature:

| Feature | World | Policy | ADR |
|---|---|---|---|
| `priority` | `priority` | multifactor: age (capped at 7 days), small jobs first, a `debug` boost | [0002](../../.github/plans/0002-wasm-policy-plugins.md) |
| `admission` | `admission` | reject walltimes over 7 days; cap `debug` at 1 h and one node; move GPU jobs to `gpu` | [0003](../../.github/plans/0003-admission-plugins.md) |
| `estimator` | `estimator` | plan jobs over an hour at 60% of their walltime | [0002](../../.github/plans/0002-wasm-policy-plugins.md) |
| `node-score` | `node-score` | best fit, keeping GPU nodes for GPU jobs | [0004](../../.github/plans/0004-node-score-plugins.md) |

Plugins must not import anything: no WASI, no host functions. Build each for
`wasm32-unknown-unknown` (which links no WASI) and wrap the core module into a component with
[`wasm-tools`](https://github.com/bytecodealliance/wasm-tools). A component implements one world,
so build one feature at a time:

```sh
rustup target add wasm32-unknown-unknown
cargo install --locked wasm-tools
for world in priority admission estimator node-score; do
  cargo build --release --target wasm32-unknown-unknown --no-default-features --features "$world"
  wasm-tools component new \
    target/wasm32-unknown-unknown/release/gnomon_policy_plugins_example.wasm -o "$world.wasm"
done
wasm-tools component wit admission.wasm   # shows the world it implements
```

Run the controller with them (see [docs/INSTALL.md](../../docs/INSTALL.md#policy-plugins)):

```sh
gnomon-ctl --admission-plugin admission.wasm --priority-plugin priority.wasm \
  --estimator-plugin estimator.wasm --node-score-plugin node-score.wasm
```

or evaluate them offline on SWF traces first:

```sh
cargo run --release -p gnomon-sim --bin experiments -- traces/ out/ 5000 --only e4 \
  --admission-plugin admission.wasm --priority-plugin priority.wasm --estimator-plugin estimator.wasm
```

`cargo component build` works too; it produces the component directly.

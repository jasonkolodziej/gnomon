# Example priority plugin

A Gnomon priority plugin written in Rust against the WIT world
[`gnomon:policy/priority`](../../crates/policy/wit/priority.wit) (see
[ADR 0002](../../.github/plans/0002-wasm-policy-plugins.md)). It computes a small
multifactor priority (age, job size, a `debug` partition boost); see `src/lib.rs`.

Plugins must not import anything: no WASI, no host functions. Build for
`wasm32-unknown-unknown` (which links no WASI) and wrap the core module into a
component with [`wasm-tools`](https://github.com/bytecodealliance/wasm-tools):

```sh
rustup target add wasm32-unknown-unknown
cargo install --locked wasm-tools
cargo build --release --target wasm32-unknown-unknown
wasm-tools component new \
  target/wasm32-unknown-unknown/release/gnomon_priority_plugin_example.wasm \
  -o priority.wasm
wasm-tools component wit priority.wasm   # shows the world it implements
```

Run the controller with it:

```sh
gnomon-ctl --priority-plugin priority.wasm
```

or evaluate it offline on SWF traces first:

```sh
cargo run --release -p gnomon-sim --bin experiments -- traces/ out/ 5000 \
  --only e4 --priority-plugin priority.wasm
```

`cargo component build` works too; it produces the component directly.

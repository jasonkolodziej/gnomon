//! Writes one small, working plugin per world (WAT components, which the
//! controller's `--*-plugin` flags load like compiled `.wasm`) for demos:
//!
//! ```sh
//! cargo run -p gnomon-policy --example demo-plugins -- DIR
//! gnomon-ctl --admission-plugin DIR/admission.wat --priority-plugin DIR/priority.wat \
//!   --shadow-priority-plugin DIR/shadow-priority.wat --estimator-plugin DIR/estimator.wat \
//!   --node-score-plugin DIR/node-score.wat
//! ```
//!
//! | File | World | Policy |
//! |---|---|---|
//! | `admission.wat` | admission | reject jobs over 64 cores in total; move GPU jobs to `gpu` |
//! | `priority.wat` | priority | `(cores + 4 gpus) x nodes x 100 + minutes waited` |
//! | `shadow-priority.wat` | priority | `first byte of user x 1000 + partition length` (a candidate, compared only) |
//! | `estimator.wat` | estimator | half the walltime for jobs over 10 minutes |
//! | `node-score.wat` | node-score | spread: most free cores first |
//!
//! Real plugins are compiled from Rust: see `examples/policy-plugins`.

use gnomon_policy::fixtures;
use std::path::PathBuf;

fn main() {
    let dir = PathBuf::from(std::env::args().nth(1).unwrap_or_else(|| ".".into()));
    if let Err(e) = std::fs::create_dir_all(&dir) {
        eprintln!("demo-plugins: {}: {e}", dir.display());
        std::process::exit(1);
    }
    let files = [
        ("admission.wat", fixtures::admission_rules()),
        ("priority.wat", fixtures::size_age()),
        ("shadow-priority.wat", fixtures::strings()),
        ("estimator.wat", fixtures::half_walltime()),
        ("node-score.wat", fixtures::most_free_cores()),
    ];
    for (name, wat) in files {
        let path = dir.join(name);
        if let Err(e) = std::fs::write(&path, wat) {
            eprintln!("demo-plugins: {}: {e}", path.display());
            std::process::exit(1);
        }
        println!("{}", path.display());
    }
}

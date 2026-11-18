//! `gnomon-web`: serves the Gnomon browser UI and bridges it to the controllers.
//!
//! ```sh
//! gnomon-web                                   # http://127.0.0.1:8080, controller from GNOMON_ADDR
//! gnomon-web --listen 0.0.0.0:8080 --ui-dir ui/dist --read-only
//! GNOMON_ADDR=http://ctl1:7631,http://ctl2:7631,http://ctl3:7631 gnomon-web
//! ```
//!
//! The gRPC API has no authentication, and with submit/cancel enabled this
//! page can submit jobs as any user: keep the default localhost binding, or
//! use `--read-only`, unless the network is trusted.

use gnomon_web::{router, run_poller, App, Hub};
use std::path::PathBuf;

fn flag(args: &[String], name: &str) -> Option<String> {
    let long = format!("--{name}");
    let eq = format!("--{name}=");
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if *a == long {
            return it.next().cloned();
        }
        if let Some(v) = a.strip_prefix(&eq) {
            return Some(v.to_string());
        }
    }
    None
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "-h" || a == "--help") {
        println!(
            "gnomon-web [--listen ADDR] [--ui-dir DIR] [--read-only]\n\
             Controllers: GNOMON_ADDR (comma-separated; default http://127.0.0.1:7631)."
        );
        return;
    }
    let listen = flag(&args, "listen")
        .or_else(|| std::env::var("GNOMON_WEB_LISTEN").ok())
        .unwrap_or_else(|| "127.0.0.1:8080".into());
    let ui_dir = PathBuf::from(
        flag(&args, "ui-dir")
            .or_else(|| std::env::var("GNOMON_WEB_UI_DIR").ok())
            .unwrap_or_else(|| "ui/dist".into()),
    );
    let read_only = args.iter().any(|a| a == "--read-only");
    let addrs: Vec<String> = std::env::var("GNOMON_ADDR")
        .unwrap_or_else(|_| "http://127.0.0.1:7631".into())
        .split(',')
        .map(str::trim)
        .filter(|a| !a.is_empty())
        .map(String::from)
        .collect();
    if addrs.is_empty() {
        eprintln!("gnomon-web: GNOMON_ADDR lists no controller");
        std::process::exit(2);
    }

    let hub = Hub::new(!read_only);
    tokio::spawn(run_poller(addrs.clone(), hub.clone()));
    let app = router(App {
        hub,
        addrs: addrs.clone(),
        ui_dir: ui_dir.clone(),
    });
    let listener = match tokio::net::TcpListener::bind(&listen).await {
        Ok(l) => l,
        Err(e) => {
            eprintln!("gnomon-web: cannot listen on {listen}: {e}");
            std::process::exit(1);
        }
    };
    if !ui_dir.join("index.html").exists() {
        eprintln!(
            "gnomon-web: no UI in {} (build it with ui/build.sh); serving the API and a help page",
            ui_dir.display()
        );
    }
    println!(
        "gnomon-web on http://{listen} ({}), following {}",
        if read_only {
            "read-only"
        } else {
            "submit and cancel enabled"
        },
        addrs.join(",")
    );
    if let Err(e) = axum::serve(listener, app).await {
        eprintln!("gnomon-web: {e}");
        std::process::exit(1);
    }
}

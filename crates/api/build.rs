fn main() {
    if std::env::var("PROTOC").is_err() {
        std::env::set_var("PROTOC", "/home/kimi/protoc/bin/protoc");
    }
    tonic_build::configure()
        .build_server(true)
        .build_client(true)
        .compile_protos(&["../../proto/gnomon.proto"], &["../../proto"])
        .expect("proto compile");
}

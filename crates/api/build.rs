fn main() -> Result<(), Box<dyn std::error::Error>> {
    // protoc is located via the PROTOC env var or PATH (see README: Build & test).
    println!("cargo:rerun-if-changed=../../proto/gnomon.proto");
    tonic_build::configure()
        .build_server(true)
        .build_client(true)
        .compile_protos(&["../../proto/gnomon.proto"], &["../../proto"])?;
    Ok(())
}

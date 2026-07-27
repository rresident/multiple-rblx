fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        winresource::WindowsResource::new()
            .set_icon("assets/multiple.ico")
            .compile()
            .expect("failed to compile Windows resources");
    }
}

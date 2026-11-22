fn main() {
    // Отладочные сведения Slint нужны тестам окна (поиск элементов через
    // `i-slint-backend-testing`); в выпускной сборке их нет.
    let debug = std::env::var("PROFILE").is_ok_and(|p| p == "debug");
    let config = slint_build::CompilerConfiguration::new()
        .with_style("fluent".into())
        .with_debug_info(debug);
    slint_build::compile_with_config("ui/app.slint", config)
        .expect("failed to compile ui/app.slint");
}

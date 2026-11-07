fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime.block_on(legate_cli::execute(args)),
        Err(err) => {
            eprintln!("legate: {err}");
            1
        }
    };
    std::process::exit(code);
}

use log::LevelFilter;

fn main() {
    deerpunk_engine_common::logger::init(LevelFilter::Info).expect("failed to init logger");
    log::info!("main() starting");

    let mut host = deerpunk_engine_host::Host::new();
    host.initialize("examples/deathmatch/gamedir");

    let dll_path =
        std::env::var("SERVER_DLL").unwrap_or_else(|_| "dlls/server.dll".to_string());
    host.load_server_dll(&dll_path);

    host.run();
}

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // One executable, two lives. A browser launches this with its own origin as
    // the first argument; that process is the native-messaging relay and must not
    // become a second window, a second tray icon or a second database.
    let args: Vec<String> = std::env::args().collect();
    // The installer and uninstaller run this for their add-on step. Like the
    // relay, it must not become a second window, tray icon or database.
    if args.iter().any(|a| a == "--install-addon") {
        std::process::exit(sucr_lib::addon_cli(true));
    }
    if args.iter().any(|a| a == "--remove-addon") {
        std::process::exit(sucr_lib::addon_cli(false));
    }
    if sucr_lib::launched_by_browser(&args) {
        std::process::exit(sucr_lib::native_host_main());
    }
    sucr_lib::run()
}

fn main() {
    // A browser starts this program as the native host of the Hexis extension: no window, no vault, one message in and out.
    #[cfg(unix)]
    if std::env::args().nth(1).as_deref() == Some("--native-host") {
        std::process::exit(hexis_lib::native_host());
    }
    hexis_lib::run()
}

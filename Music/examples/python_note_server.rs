//! Standalone, device-free native note bridge for Python integration tests.
fn main() -> std::io::Result<()> {
    teledra_music::run_python_note_server()
}

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    std::panic::set_hook(Box::new(|info| {
        let msg = format!("PANIC: {}\n", info);
        let dir = dirs::config_dir().unwrap_or_else(std::env::temp_dir).join("gpo-autofish");
        let _ = std::fs::write(dir.join("panic.txt"), &msg);
        let _ = std::fs::write("D:\\Games\\GPO Autofish\\panic.txt", &msg);
        eprintln!("{}", msg);
    }));
    gpo_autofish_lib::run();
}

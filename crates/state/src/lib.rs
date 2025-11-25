pub mod env;
pub mod persistence;

pub use env::{cache_ttl, env_auto_pin, env_diag, env_include_claude, env_max_bytes, extra_dirs_from_env, home_dir, load_manifest_settings, manifest_file, ManifestSettings};
pub use persistence::{auto_pin_from_history, load_auto_pin_flag, load_history, load_pinned, print_history, save_auto_pin_flag, save_history, save_pinned, HistoryEntry};

pub fn placeholder() {
    // stub to be replaced in later tasks
    todo!("state implementation pending")
}

pub mod env;
pub mod persistence;

pub use env::{
    cache_ttl, env_auto_pin, env_diag, env_include_claude, env_manifest_first,
    env_manifest_minimal, env_max_bytes, env_render_mode_log, extra_dirs_from_env, home_dir,
    load_manifest_settings, manifest_file, runtime_overrides_path, ManifestSettings,
};
pub use persistence::{
    auto_pin_file, auto_pin_from_history, history_file, load_auto_pin_flag, load_history,
    load_pinned, pinned_file, print_history, save_auto_pin_flag, save_history, save_pinned,
    HistoryEntry,
};

pub fn placeholder() {
    // stub to be replaced in later tasks
    todo!("state implementation pending")
}

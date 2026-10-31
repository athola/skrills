use anyhow::Result;
use std::path::PathBuf;

/// The `setup` flags, one field per flag.
#[derive(Debug, Default)]
pub(crate) struct SetupOptions {
    pub client: Option<String>,
    pub bin_dir: Option<PathBuf>,
    pub reinstall: bool,
    pub uninstall: bool,
    pub add: bool,
    pub yes: bool,
    pub universal: bool,
    pub mirror_source: Option<PathBuf>,
    pub include_marketplace: bool,
}

pub(crate) fn handle_setup_command(options: SetupOptions) -> Result<()> {
    let SetupOptions {
        client,
        bin_dir,
        reinstall,
        uninstall,
        add,
        yes,
        universal,
        mirror_source,
        include_marketplace,
    } = options;
    let config = skrills_setup::interactive_setup(
        client,
        bin_dir,
        reinstall,
        uninstall,
        add,
        yes,
        universal,
        mirror_source,
        include_marketplace,
    )?;
    skrills_setup::run_setup(config)
}

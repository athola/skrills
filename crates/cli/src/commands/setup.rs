use anyhow::Result;
use std::path::PathBuf;

#[allow(clippy::too_many_arguments)]
pub(crate) fn handle_setup_command(
    client: Option<String>,
    bin_dir: Option<PathBuf>,
    reinstall: bool,
    uninstall: bool,
    add: bool,
    yes: bool,
    universal: bool,
    mirror_source: Option<PathBuf>,
) -> Result<()> {
    let config = skrills_server::setup::interactive_setup(
        client,
        bin_dir,
        reinstall,
        uninstall,
        add,
        yes,
        universal,
        mirror_source,
    )?;
    skrills_server::setup::run_setup(config)
}

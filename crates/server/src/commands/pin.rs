use crate::discovery::{collect_skills, merge_extra_dirs, resolve_skill};
use anyhow::{anyhow, Result};
use skrills_state::{load_pinned, load_pinned_with_defaults, save_auto_pin_flag, save_pinned};
use std::collections::HashSet;

/// Handle the `list-pinned` command.
pub(crate) fn handle_list_pinned_command() -> Result<()> {
    let pinned = load_pinned_with_defaults()?;
    if pinned.is_empty() {
        println!("(no pinned skills)");
    } else {
        let mut list: Vec<_> = pinned.into_iter().collect();
        list.sort();
        for name in list {
            println!("{name}");
        }
    }
    Ok(())
}

/// Handle the `pin` command.
pub(crate) fn handle_pin_command(skills: Vec<String>) -> Result<()> {
    let mut pinned = load_pinned()?;
    let all_skills = collect_skills(&merge_extra_dirs(&[]))?;
    for spec in skills {
        let name = resolve_skill(&spec, &all_skills)?;
        pinned.insert(name.to_string());
    }
    save_pinned(&pinned)?;
    println!("Pinned {} skills.", pinned.len());
    Ok(())
}

/// Handle the `unpin` command.
pub(crate) fn handle_unpin_command(skills: Vec<String>, all: bool) -> Result<()> {
    if all {
        save_pinned(&HashSet::new())?;
        println!("Cleared all pinned skills.");
        return Ok(());
    }
    if skills.is_empty() {
        return Err(anyhow!("provide skill names or use --all"));
    }
    let mut pinned = load_pinned()?;
    let all_skills = collect_skills(&merge_extra_dirs(&[]))?;
    for spec in skills {
        let name = resolve_skill(&spec, &all_skills)?;
        pinned.remove(name);
    }
    save_pinned(&pinned)?;
    println!("Pinned skills remaining: {}", pinned.len());
    Ok(())
}

/// Handle the `auto-pin` command.
pub(crate) fn handle_auto_pin_command(enable: bool) -> Result<()> {
    save_auto_pin_flag(enable)?;
    println!("Auto-pin {}", if enable { "enabled" } else { "disabled" });
    Ok(())
}

use crate::discovery::{
    collect_agents, merge_extra_dirs, resolve_agent, DEFAULT_AGENT_RUN_TEMPLATE,
};
use anyhow::{anyhow, Result};
use std::path::PathBuf;
use std::process::Command;

pub(crate) fn handle_agent_command(
    agent_spec: String,
    skill_dirs: Vec<PathBuf>,
    dry_run: bool,
) -> Result<()> {
    let agents = collect_agents(&merge_extra_dirs(&skill_dirs))?;
    let agent = resolve_agent(&agent_spec, &agents)?;
    let cmd = DEFAULT_AGENT_RUN_TEMPLATE.replace("{}", &agent.path.display().to_string());
    let prompt = format!(
        "Load agent spec at {} and execute its instructions",
        agent.path.display()
    );
    println!(
        "Agent: {} (source: {}, path: {})",
        agent.name,
        agent.source.label(),
        agent.path.display()
    );
    if dry_run {
        println!("Command: {cmd}");
        return Ok(());
    }
    let status = Command::new("codex")
        .args(["--yolo", "exec", "--timeout_ms", "1800000"])
        .arg(prompt)
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(anyhow!(
            "agent command exited with status {:?}",
            status.code()
        ))
    }
}

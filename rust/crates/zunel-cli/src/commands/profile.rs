use anyhow::{Context, Result};

use crate::cli::{ProfileArgs, ProfileCommand, ProfileRmArgs, ProfileUseArgs};

pub async fn run(args: ProfileArgs) -> Result<()> {
    match args.command {
        ProfileCommand::List => list(),
        ProfileCommand::Use(args) => use_profile(args),
        ProfileCommand::Rm(args) => remove(args),
        ProfileCommand::Show => show(),
    }
}

fn list() -> Result<()> {
    let active = zunel_config::active_profile_name();
    let profiles = zunel_config::list_profiles()?;
    if profiles.is_empty() {
        println!("No profiles found yet.");
        return Ok(());
    }
    for profile in profiles {
        let marker = if profile == active { " *" } else { "" };
        println!(
            "{profile}\t{}{}",
            zunel_config::resolve_profile_home(&profile)?.display(),
            marker
        );
    }
    Ok(())
}

fn use_profile(args: ProfileUseArgs) -> Result<()> {
    match zunel_config::set_sticky_profile(&args.name) {
        Ok(()) => {}
        Err(err @ zunel_config::Error::InvalidProfileName(_)) => {
            eprintln!("Error: {err}");
            std::process::exit(2);
        }
        Err(err) => return Err(err.into()),
    }
    if args.name == zunel_config::DEFAULT_PROFILE_NAME {
        println!("Cleared sticky profile; using the default home.");
    } else {
        println!(
            "Active profile set to {} ({})",
            args.name,
            zunel_config::resolve_profile_home(&args.name)?.display()
        );
    }
    Ok(())
}

fn remove(args: ProfileRmArgs) -> Result<()> {
    if args.name == zunel_config::active_profile_name() {
        anyhow::bail!(
            "Refusing to delete the active profile {:?}. Switch with `zunel profile use default` first.",
            args.name
        );
    }
    if !args.force {
        anyhow::bail!("refusing to remove profile without --force");
    }
    let directory = zunel_config::resolve_profile_home(&args.name)?;
    if !directory.exists() {
        println!(
            "No directory at {}; nothing to remove.",
            directory.display()
        );
        return Ok(());
    }
    std::fs::remove_dir_all(&directory)
        .with_context(|| format!("removing {}", directory.display()))?;
    println!("Removed {}", directory.display());
    Ok(())
}

fn show() -> Result<()> {
    println!("profile: {}", zunel_config::active_profile_name());
    println!("home: {}", zunel_config::active_profile_home()?.display());
    Ok(())
}

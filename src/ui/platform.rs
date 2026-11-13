//! Opening files with other applications.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Opens a file with the system's default application.
pub fn open(path: &Path) -> Result<()> {
    open::that_detached(path).with_context(|| format!("opening {}", path.display()))
}

/// Shows the file in the system file manager (selecting it where supported).
pub fn reveal(path: &Path) -> Result<()> {
    #[cfg(target_os = "windows")]
    {
        let mut arg = std::ffi::OsString::from("/select,");
        arg.push(path);
        spawn(Command::new("explorer").arg(arg))?;
        return Ok(());
    }
    #[cfg(target_os = "macos")]
    {
        spawn(Command::new("open").arg("-R").arg(path))?;
        return Ok(());
    }
    #[allow(unreachable_code)]
    {
        let dir = path.parent().unwrap_or(path);
        open::that_detached(dir).with_context(|| format!("opening {}", dir.display()))
    }
}

/// Opens a video at `t` seconds with mpv or VLC if one is installed, otherwise with the default
/// player (which starts at the beginning). Returns the name of the player used.
pub fn open_video_at(path: &Path, t: f64) -> Result<&'static str> {
    let t = t.max(0.0);
    if let Some(mpv) = find_program("mpv") {
        spawn(Command::new(mpv).arg(format!("--start={t:.1}")).arg(path))?;
        return Ok("mpv");
    }
    if let Some(vlc) = find_program("vlc") {
        spawn(
            Command::new(vlc)
                .arg(format!("--start-time={t:.1}"))
                .arg(path),
        )?;
        return Ok("VLC");
    }
    open(path)?;
    Ok("default player")
}

fn spawn(cmd: &mut Command) -> Result<()> {
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    Ok(())
}

fn find_program(name: &str) -> Option<PathBuf> {
    let exe = if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    };
    let mut candidates: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).map(|d| d.join(&exe)).collect())
        .unwrap_or_default();
    if cfg!(target_os = "macos") && name == "vlc" {
        candidates.push(PathBuf::from("/Applications/VLC.app/Contents/MacOS/VLC"));
    }
    if cfg!(windows) && name == "vlc" {
        candidates.push(PathBuf::from(r"C:\Program Files\VideoLAN\VLC\vlc.exe"));
    }
    candidates.into_iter().find(|p| p.is_file())
}

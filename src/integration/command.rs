use std::io;
use std::path::Path;

#[cfg(not(windows))]
use std::{env, fs};

pub(crate) fn shell_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

pub(crate) fn hook_command(hook_path: &Path, action: Option<&str>) -> String {
    let path = hook_path.display().to_string();
    #[cfg(windows)]
    {
        let mut command = format!(
            "powershell -NoProfile -ExecutionPolicy Bypass -File {}",
            windows_command_quote(&path)
        );
        if let Some(action) = action {
            command.push(' ');
            command.push_str(action);
        }
        command
    }

    #[cfg(not(windows))]
    {
        let mut command = format!("bash {}", shell_single_quote(&path));
        if let Some(action) = action {
            command.push(' ');
            command.push_str(action);
        }
        command
    }
}

pub(crate) fn resolved_bash_hook_command(
    hook_path: &Path,
    action: Option<&str>,
) -> io::Result<String> {
    #[cfg(windows)]
    {
        Ok(hook_command(hook_path, action))
    }

    #[cfg(not(windows))]
    {
        let bash_path = resolve_executable("bash")?;
        let mut command = format!(
            "{} {}",
            shell_single_quote(&bash_path.display().to_string()),
            shell_single_quote(&hook_path.display().to_string())
        );
        if let Some(action) = action {
            command.push(' ');
            command.push_str(action);
        }
        Ok(command)
    }
}

#[cfg(not(windows))]
fn resolve_executable(program: &str) -> io::Result<std::path::PathBuf> {
    let path = env::var_os("PATH").ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            format!("cannot register hooks: PATH is unset while resolving {program}"),
        )
    })?;
    let current_dir = env::current_dir()?;

    for dir in env::split_paths(&path) {
        let candidate = if dir.is_absolute() {
            dir.join(program)
        } else {
            current_dir.join(dir).join(program)
        };
        let Ok(metadata) = fs::metadata(&candidate) else {
            continue;
        };
        if !metadata.is_file() {
            continue;
        }

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if metadata.permissions().mode() & 0o111 == 0 {
                continue;
            }
        }

        return Ok(candidate);
    }

    Err(io::Error::new(
        io::ErrorKind::NotFound,
        format!("cannot register hooks: executable {program} was not found on PATH"),
    ))
}

pub(crate) fn legacy_bash_hook_command(hook_path: &Path, action: Option<&str>) -> String {
    let mut command = format!(
        "bash {}",
        shell_single_quote(&hook_path.display().to_string())
    );
    if let Some(action) = action {
        command.push(' ');
        command.push_str(action);
    }
    command
}

#[cfg(windows)]
fn windows_command_quote(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\\\""))
}

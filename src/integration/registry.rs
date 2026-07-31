use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::command::{hook_command, resolved_bash_hook_command};
use super::config_edit::{build_codex_config_with_hooks, is_owned_codex_hook_command};
use super::env::*;

pub(crate) fn integration_target_label(
    target: crate::api::schema::IntegrationTarget,
) -> &'static str {
    match target {
        crate::api::schema::IntegrationTarget::Pi => "pi",
        crate::api::schema::IntegrationTarget::Omp => "omp",
        crate::api::schema::IntegrationTarget::Claude => "claude",
        crate::api::schema::IntegrationTarget::Codex => "codex",
        crate::api::schema::IntegrationTarget::Copilot => "copilot",
        crate::api::schema::IntegrationTarget::Devin => "devin",
        crate::api::schema::IntegrationTarget::Droid => "droid",
        crate::api::schema::IntegrationTarget::Kimi => "kimi",
        crate::api::schema::IntegrationTarget::Opencode => "opencode",
        crate::api::schema::IntegrationTarget::Kilo => "kilo",
        crate::api::schema::IntegrationTarget::Hermes => "hermes",
        crate::api::schema::IntegrationTarget::Qodercli => "qodercli",
        crate::api::schema::IntegrationTarget::Cursor => "cursor",
    }
}

pub(crate) fn integration_target_command(
    target: crate::api::schema::IntegrationTarget,
) -> &'static str {
    integration_target_command_names(target)[0]
}

pub(crate) fn integration_target_command_names(
    target: crate::api::schema::IntegrationTarget,
) -> &'static [&'static str] {
    match target {
        crate::api::schema::IntegrationTarget::Pi => &["pi"],
        crate::api::schema::IntegrationTarget::Omp => &["omp"],
        crate::api::schema::IntegrationTarget::Claude => &["claude"],
        crate::api::schema::IntegrationTarget::Codex => &["codex"],
        crate::api::schema::IntegrationTarget::Copilot => &["copilot"],
        crate::api::schema::IntegrationTarget::Devin => &["devin"],
        crate::api::schema::IntegrationTarget::Droid => &["droid"],
        crate::api::schema::IntegrationTarget::Kimi => &["kimi"],
        crate::api::schema::IntegrationTarget::Opencode => &["opencode"],
        crate::api::schema::IntegrationTarget::Kilo => &["kilo", "kilo-code"],
        crate::api::schema::IntegrationTarget::Hermes => &["hermes"],
        crate::api::schema::IntegrationTarget::Qodercli => qodercli_command_names(),
        crate::api::schema::IntegrationTarget::Cursor => cursor_command_names(),
    }
}

pub(crate) fn cursor_command_names() -> &'static [&'static str] {
    &["cursor-agent"]
}

pub(crate) fn integration_target_supported(target: crate::api::schema::IntegrationTarget) -> bool {
    #[cfg(windows)]
    {
        matches!(
            target,
            crate::api::schema::IntegrationTarget::Claude
                | crate::api::schema::IntegrationTarget::Codex
                | crate::api::schema::IntegrationTarget::Copilot
                | crate::api::schema::IntegrationTarget::Droid
                | crate::api::schema::IntegrationTarget::Kimi
                | crate::api::schema::IntegrationTarget::Qodercli
        )
    }

    #[cfg(not(windows))]
    {
        let _ = target;
        true
    }
}

pub(crate) fn integration_target_available(target: crate::api::schema::IntegrationTarget) -> bool {
    if !integration_target_supported(target) {
        return false;
    }

    integration_target_command_names(target)
        .iter()
        .any(|command| command_available(command))
        || integration_target_install_layout_available(target)
}

#[cfg(windows)]
pub(crate) fn qodercli_command_names() -> &'static [&'static str] {
    &["qodercli", "qoder", "qoderclicn", "qodercn"]
}

#[cfg(not(windows))]
pub(crate) fn qodercli_command_names() -> &'static [&'static str] {
    &["qodercli"]
}

pub(crate) fn integration_target_install_layout_available(
    target: crate::api::schema::IntegrationTarget,
) -> bool {
    match target {
        crate::api::schema::IntegrationTarget::Codex => codex_standalone_binary_available(),
        crate::api::schema::IntegrationTarget::Hermes => hermes_install_layout_available(),
        _ => false,
    }
}

pub(crate) fn command_available(command: &str) -> bool {
    let Some(paths) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&paths).any(|dir| {
        command_path_candidates(&dir, command)
            .into_iter()
            .any(|path| executable_file_exists(&path))
    })
}

pub(crate) fn command_path_candidates(dir: &Path, command: &str) -> Vec<PathBuf> {
    let base = dir.join(command);

    #[cfg(not(windows))]
    {
        vec![base]
    }

    #[cfg(windows)]
    {
        if Path::new(command).extension().is_some() {
            return vec![base];
        }

        let mut candidates = vec![base];
        for extension in [".exe", ".cmd", ".bat", ".ps1"] {
            candidates.push(dir.join(format!("{command}{extension}")));
        }
        candidates
    }
}

pub(crate) fn executable_file_exists(path: &Path) -> bool {
    let Ok(metadata) = path.metadata() else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }

    #[cfg(not(unix))]
    {
        true
    }
}

pub(crate) fn codex_standalone_binary_available() -> bool {
    let Ok(releases_dir) =
        codex_dir().map(|dir| dir.join("packages").join("standalone").join("releases"))
    else {
        return false;
    };
    let Ok(entries) = fs::read_dir(releases_dir) else {
        return false;
    };

    entries.filter_map(Result::ok).any(|entry| {
        executable_file_exists(&entry.path().join("bin").join(codex_executable_name()))
    })
}

pub(crate) fn codex_executable_name() -> &'static str {
    if cfg!(windows) {
        "codex.exe"
    } else {
        "codex"
    }
}

pub(crate) fn hermes_install_layout_available() -> bool {
    #[cfg(windows)]
    {
        let Some(local_app_data) =
            std::env::var_os("LOCALAPPDATA").filter(|value| !value.is_empty())
        else {
            return false;
        };
        let dir = PathBuf::from(local_app_data).join("hermes");
        [
            dir.join("hermes.exe"),
            dir.join("bin").join("hermes.exe"),
            dir.join("Scripts").join("hermes.exe"),
        ]
        .into_iter()
        .any(|path| executable_file_exists(&path))
    }

    #[cfg(not(windows))]
    {
        false
    }
}

pub(crate) fn installed_integration_statuses() -> Vec<super::IntegrationStatus> {
    integration_specs()
        .into_iter()
        .filter_map(|(target, path, expected_version)| {
            if !integration_target_supported(target) {
                return None;
            }
            Some(integration_status_at(target, path.ok()?, expected_version))
        })
        .collect()
}

pub(crate) fn integration_status(
    target: crate::api::schema::IntegrationTarget,
) -> io::Result<super::IntegrationStatus> {
    if !integration_target_supported(target) {
        return Err(io::Error::other(format!(
            "{} integration is not supported on this platform",
            integration_target_label(target)
        )));
    }

    let Some((target, path, expected_version)) = integration_specs()
        .into_iter()
        .find(|(candidate, _, _)| *candidate == target)
    else {
        return Err(io::Error::other(format!(
            "unknown integration target: {}",
            integration_target_label(target)
        )));
    };

    integration_status_checked_at(target, path?, expected_version)
}

pub(crate) fn integration_recommendations() -> Vec<super::IntegrationRecommendation> {
    integration_specs()
        .into_iter()
        .filter_map(|(target, path, expected_version)| {
            if !integration_target_supported(target) {
                return None;
            }
            let path = path.ok()?;
            let status = integration_status_at(target, path.clone(), expected_version);
            Some(super::IntegrationRecommendation {
                target,
                label: integration_target_label(target),
                command: integration_target_command(target),
                available: integration_target_available(target)
                    || status.state != super::IntegrationStatusKind::NotInstalled,
                path,
                state: status.state,
            })
        })
        .collect()
}

pub(crate) fn outdated_installed_integrations() -> Vec<super::IntegrationStatus> {
    installed_integration_statuses()
        .into_iter()
        .filter(|status| status.state == super::IntegrationStatusKind::Outdated)
        .collect()
}

fn integration_specs() -> [(
    crate::api::schema::IntegrationTarget,
    io::Result<PathBuf>,
    u32,
); 13] {
    [
        (
            crate::api::schema::IntegrationTarget::Pi,
            pi_extension_dir().map(|dir| dir.join(super::PI_EXTENSION_INSTALL_NAME)),
            super::PI_INTEGRATION_VERSION,
        ),
        (
            crate::api::schema::IntegrationTarget::Omp,
            omp_extension_dir().map(|dir| dir.join(super::OMP_EXTENSION_INSTALL_NAME)),
            super::OMP_INTEGRATION_VERSION,
        ),
        (
            crate::api::schema::IntegrationTarget::Claude,
            claude_dir().map(|dir| dir.join("hooks").join(super::CLAUDE_HOOK_INSTALL_NAME)),
            super::CLAUDE_INTEGRATION_VERSION,
        ),
        (
            crate::api::schema::IntegrationTarget::Codex,
            codex_dir().map(|dir| dir.join(super::CODEX_HOOK_INSTALL_NAME)),
            super::CODEX_INTEGRATION_VERSION,
        ),
        (
            crate::api::schema::IntegrationTarget::Copilot,
            copilot_dir().map(|dir| dir.join("hooks").join(super::COPILOT_HOOK_INSTALL_NAME)),
            super::COPILOT_INTEGRATION_VERSION,
        ),
        (
            crate::api::schema::IntegrationTarget::Devin,
            devin_dir().map(|dir| dir.join(super::DEVIN_HOOK_INSTALL_NAME)),
            super::DEVIN_INTEGRATION_VERSION,
        ),
        (
            crate::api::schema::IntegrationTarget::Droid,
            droid_dir().map(|dir| dir.join("hooks").join(super::DROID_HOOK_INSTALL_NAME)),
            super::DROID_INTEGRATION_VERSION,
        ),
        (
            crate::api::schema::IntegrationTarget::Kimi,
            kimi_dir().map(|dir| dir.join("hooks").join(super::KIMI_HOOK_INSTALL_NAME)),
            super::KIMI_INTEGRATION_VERSION,
        ),
        (
            crate::api::schema::IntegrationTarget::Opencode,
            opencode_dir().map(|dir| {
                dir.join("plugins")
                    .join(super::OPENCODE_PLUGIN_INSTALL_NAME)
            }),
            super::OPENCODE_INTEGRATION_VERSION,
        ),
        (
            crate::api::schema::IntegrationTarget::Kilo,
            kilo_dir().map(|dir| dir.join("plugin").join(super::KILO_PLUGIN_INSTALL_NAME)),
            super::KILO_INTEGRATION_VERSION,
        ),
        (
            crate::api::schema::IntegrationTarget::Hermes,
            hermes_plugin_dir().map(|dir| dir.join(super::HERMES_PLUGIN_INIT_INSTALL_NAME)),
            super::HERMES_INTEGRATION_VERSION,
        ),
        (
            crate::api::schema::IntegrationTarget::Qodercli,
            qodercli_dir().map(|dir| dir.join("hooks").join(super::QODERCLI_HOOK_INSTALL_NAME)),
            super::QODERCLI_INTEGRATION_VERSION,
        ),
        (
            crate::api::schema::IntegrationTarget::Cursor,
            cursor_dir().map(|dir| dir.join(super::CURSOR_HOOK_INSTALL_NAME)),
            super::CURSOR_INTEGRATION_VERSION,
        ),
    ]
}

pub(crate) fn integration_update_instructions(
    targets: &[crate::api::schema::IntegrationTarget],
) -> String {
    let commands: Vec<String> = targets
        .iter()
        .map(|target| {
            format!(
                "`herdr integration install {}`",
                integration_target_label(*target)
            )
        })
        .collect();

    match commands.as_slice() {
        [] => String::new(),
        [command] => format!("run {command}"),
        [rest @ .., last] => format!("run {} and {last}", rest.join(", ")),
    }
}

pub(crate) fn print_outdated_update_notice() -> bool {
    let outdated = outdated_installed_integrations();
    if outdated.is_empty() {
        return false;
    }

    let targets = outdated
        .iter()
        .map(|integration| integration.target)
        .collect::<Vec<_>>();
    eprintln!(
        "installed herdr integrations need updating; {}.",
        integration_update_instructions(&targets).replace('`', "")
    );
    true
}

pub(crate) fn integration_status_at(
    target: crate::api::schema::IntegrationTarget,
    path: PathBuf,
    expected_version: u32,
) -> super::IntegrationStatus {
    if !path.is_file() {
        return super::IntegrationStatus {
            target,
            path,
            state: super::IntegrationStatusKind::NotInstalled,
            installed_version: None,
            expected_version,
        };
    }

    let content = fs::read_to_string(&path).ok();
    let installed_version = content.as_deref().and_then(parse_integration_version);
    let is_current = match target {
        crate::api::schema::IntegrationTarget::Claude => content
            .as_deref()
            .is_some_and(|content| claude_installation_is_current(&path, content)),
        crate::api::schema::IntegrationTarget::Codex => content
            .as_deref()
            .is_some_and(|content| codex_installation_is_current(&path, content)),
        crate::api::schema::IntegrationTarget::Opencode => content
            .as_deref()
            .is_some_and(|content| opencode_installation_is_current(&path, content)),
        _ => installed_version.is_some_and(|version| version >= expected_version),
    };
    let state = if is_current {
        super::IntegrationStatusKind::Current
    } else {
        super::IntegrationStatusKind::Outdated
    };

    super::IntegrationStatus {
        target,
        path,
        state,
        installed_version,
        expected_version,
    }
}

fn integration_status_checked_at(
    target: crate::api::schema::IntegrationTarget,
    path: PathBuf,
    expected_version: u32,
) -> io::Result<super::IntegrationStatus> {
    let metadata = match fs::metadata(&path) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            return Ok(super::IntegrationStatus {
                target,
                path,
                state: super::IntegrationStatusKind::NotInstalled,
                installed_version: None,
                expected_version,
            });
        }
        Err(err) => {
            return Err(io::Error::new(
                err.kind(),
                format!(
                    "failed to inspect {} integration at {}: {err}",
                    integration_target_label(target),
                    path.display()
                ),
            ));
        }
    };
    if !metadata.is_file() {
        return Err(io::Error::other(format!(
            "{} integration path is not a regular file: {}",
            integration_target_label(target),
            path.display()
        )));
    }

    let content = fs::read_to_string(&path).map_err(|err| {
        io::Error::new(
            err.kind(),
            format!(
                "failed to read {} integration at {}: {err}",
                integration_target_label(target),
                path.display()
            ),
        )
    })?;
    let installed_version = parse_integration_version(&content);
    let is_current = match target {
        crate::api::schema::IntegrationTarget::Claude => {
            claude_installation_is_current_checked(&path, &content)?
        }
        crate::api::schema::IntegrationTarget::Codex => {
            codex_installation_is_current_checked(&path, &content)?
        }
        crate::api::schema::IntegrationTarget::Opencode => {
            opencode_installation_is_current_checked(&path, &content)?
        }
        _ => installed_version.is_some_and(|version| version >= expected_version),
    };

    Ok(super::IntegrationStatus {
        target,
        path,
        state: if is_current {
            super::IntegrationStatusKind::Current
        } else {
            super::IntegrationStatusKind::Outdated
        },
        installed_version,
        expected_version,
    })
}

#[derive(Clone, Copy)]
struct CommandHookExpectation<'a> {
    event: &'a str,
    action: &'a str,
    matcher: Option<&'a str>,
}

fn claude_installation_is_current(hook_path: &Path, content: &str) -> bool {
    claude_installation_is_current_checked(hook_path, content).unwrap_or(false)
}

fn claude_installation_is_current_checked(hook_path: &Path, content: &str) -> io::Result<bool> {
    if content != super::render_hook_asset(super::CLAUDE_HOOK_ASSET) {
        return Ok(false);
    }

    let Some(config_dir) = hook_path.parent().and_then(Path::parent) else {
        return Ok(false);
    };
    let expectations = [
        CommandHookExpectation {
            event: "SessionStart",
            action: "session",
            matcher: Some("*"),
        },
        CommandHookExpectation {
            event: "Stop",
            action: "mail-done",
            matcher: Some("*"),
        },
        CommandHookExpectation {
            event: "Notification",
            action: "mail-blocked",
            matcher: Some("permission_prompt"),
        },
    ];

    Ok(command_hook_registrations_are_current(
        &config_dir.join("settings.json"),
        hook_path,
        &expectations,
    )? && markdown_doctrine_is_absent(&config_dir.join("CLAUDE.md"))?)
}

fn codex_installation_is_current(hook_path: &Path, content: &str) -> bool {
    codex_installation_is_current_checked(hook_path, content).unwrap_or(false)
}

fn codex_installation_is_current_checked(hook_path: &Path, content: &str) -> io::Result<bool> {
    if content != super::render_hook_asset(super::CODEX_HOOK_ASSET) {
        return Ok(false);
    }

    let Some(config_dir) = hook_path.parent() else {
        return Ok(false);
    };
    let expectations = [
        CommandHookExpectation {
            event: "SessionStart",
            action: "session",
            matcher: None,
        },
        CommandHookExpectation {
            event: "Stop",
            action: "mail-done",
            matcher: None,
        },
        CommandHookExpectation {
            event: "PermissionRequest",
            action: "mail-blocked",
            matcher: None,
        },
    ];

    Ok(codex_command_hook_registrations_are_current(
        &config_dir.join("hooks.json"),
        hook_path,
        &expectations,
    )? && codex_hook_feature_is_current(&config_dir.join("config.toml"))?
        && markdown_doctrine_is_absent(&config_dir.join("AGENTS.md"))?)
}

fn command_hook_registrations_are_current(
    config_path: &Path,
    hook_path: &Path,
    expectations: &[CommandHookExpectation<'_>],
) -> io::Result<bool> {
    let content = match fs::read_to_string(config_path) {
        Ok(content) => content,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(err) => return Err(err),
    };
    let root: Value = serde_json::from_str(&content).map_err(|err| {
        io::Error::other(format!(
            "failed to parse integration hooks at {}: {err}",
            config_path.display()
        ))
    })?;
    let Some(hooks) = root.get("hooks").and_then(Value::as_object) else {
        return Ok(false);
    };

    let hook_path_text = hook_path.display().to_string();
    let mut matches = vec![0_u8; expectations.len()];
    for (event, entries) in hooks {
        let Some(entries) = entries.as_array() else {
            if expectations
                .iter()
                .any(|expectation| expectation.event == event)
            {
                return Ok(false);
            }
            continue;
        };
        for entry in entries {
            let matcher = entry.get("matcher").and_then(Value::as_str);
            let Some(commands) = entry.get("hooks").and_then(Value::as_array) else {
                continue;
            };
            for command in commands {
                let Some(command_text) = command.get("command").and_then(Value::as_str) else {
                    continue;
                };
                if !command_text.contains(&hook_path_text) {
                    continue;
                }

                let Some((index, _)) = expectations.iter().enumerate().find(|(_, expectation)| {
                    expectation.event == event
                        && command_text
                            == hook_command(hook_path, Some(expectation.action)).as_str()
                        && matcher == expectation.matcher
                        && command.get("type").and_then(Value::as_str) == Some("command")
                        && command.get("timeout").and_then(Value::as_u64) == Some(10)
                }) else {
                    return Ok(false);
                };
                matches[index] = matches[index].saturating_add(1);
            }
        }
    }

    Ok(matches.into_iter().all(|count| count == 1))
}

fn codex_command_hook_registrations_are_current(
    config_path: &Path,
    hook_path: &Path,
    expectations: &[CommandHookExpectation<'_>],
) -> io::Result<bool> {
    let content = match fs::read_to_string(config_path) {
        Ok(content) => content,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(err) => return Err(err),
    };
    let root: Value = serde_json::from_str(&content).map_err(|err| {
        io::Error::other(format!(
            "failed to parse integration hooks at {}: {err}",
            config_path.display()
        ))
    })?;
    let Some(hooks) = root.get("hooks").and_then(Value::as_object) else {
        return Ok(false);
    };

    let mut matches = vec![0_u8; expectations.len()];
    for (event, entries) in hooks {
        let Some(entries) = entries.as_array() else {
            if expectations
                .iter()
                .any(|expectation| expectation.event == event)
            {
                return Ok(false);
            }
            continue;
        };
        for entry in entries {
            let matcher = entry.get("matcher").and_then(Value::as_str);
            let Some(commands) = entry.get("hooks").and_then(Value::as_array) else {
                continue;
            };
            for command in commands {
                let Some(command_text) = command.get("command").and_then(Value::as_str) else {
                    continue;
                };
                let Some((index, expectation)) =
                    expectations.iter().enumerate().find(|(_, expectation)| {
                        expectation.event == event
                            && is_owned_codex_hook_command(command, hook_path, expectation.action)
                    })
                else {
                    continue;
                };

                if command_text != resolved_bash_hook_command(hook_path, Some(expectation.action))?
                    || matcher != expectation.matcher
                    || command.get("timeout").and_then(Value::as_u64) != Some(10)
                {
                    return Ok(false);
                }
                matches[index] = matches[index].saturating_add(1);
            }
        }
    }

    Ok(matches.into_iter().all(|count| count == 1))
}

fn codex_hook_feature_is_current(config_path: &Path) -> io::Result<bool> {
    let content = match fs::read_to_string(config_path) {
        Ok(content) => content,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(err) => return Err(err),
    };
    Ok(build_codex_config_with_hooks(&content) == content)
}

fn markdown_doctrine_is_absent(path: &Path) -> io::Result<bool> {
    match fs::read_to_string(path) {
        Ok(content) => Ok(!content.contains(super::DOCTRINE_BLOCK_BEGIN)
            && !content.contains(super::DOCTRINE_BLOCK_END)),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(true),
        Err(err) => Err(err),
    }
}

fn opencode_installation_is_current(plugin_path: &Path, content: &str) -> bool {
    opencode_installation_is_current_checked(plugin_path, content).unwrap_or(false)
}

fn opencode_installation_is_current_checked(plugin_path: &Path, content: &str) -> io::Result<bool> {
    if content != super::render_opencode_plugin_asset(super::OPENCODE_PLUGIN_ASSET) {
        return Ok(false);
    }

    let Some(config_dir) = plugin_path.parent().and_then(Path::parent) else {
        return Ok(false);
    };
    markdown_doctrine_is_absent(&config_dir.join("AGENTS.md"))
}

pub(crate) fn parse_integration_version(content: &str) -> Option<u32> {
    content.lines().find_map(|line| {
        let marker_line = line
            .trim()
            .trim_start_matches('/')
            .trim_start_matches('#')
            .trim();
        marker_line
            .strip_prefix(super::INTEGRATION_VERSION_MARKER)?
            .trim()
            .parse()
            .ok()
    })
}

use crate::api::schema::{
    MailKind, MailListParams, MailReadParams, MailSendParams, MailWaitParams, Method, Request,
};

pub(super) fn run_mail_command(args: &[String]) -> std::io::Result<i32> {
    match args.first().map(String::as_str) {
        Some("send") => mail_send(&args[1..]),
        Some("wait") => mail_wait(&args[1..]),
        Some("read") => mail_read(&args[1..]),
        Some("list") => mail_list(&args[1..]),
        Some("help") | Some("--help") | Some("-h") => {
            print_mail_help();
            Ok(0)
        }
        None => {
            print_mail_help();
            Ok(2)
        }
        Some(_) => {
            print_mail_help();
            Ok(2)
        }
    }
}

enum BodySource {
    Inline(String),
    File(String),
    Stdin,
}

fn mail_send(args: &[String]) -> std::io::Result<i32> {
    let Some(to) = args.first() else {
        eprintln!("usage: herdr mail send <to> --kind done|blocked|question|info --subject S [--body TEXT | --body-file PATH | --body-stdin] [--no-nudge]");
        return Ok(2);
    };

    let mut kind = None;
    let mut subject = None;
    let mut body_source = None;
    let mut no_nudge = false;

    let mut index = 1;
    while index < args.len() {
        match args[index].as_str() {
            "--kind" => {
                let Some(value) = args.get(index + 1) else {
                    eprintln!("missing value for --kind");
                    return Ok(2);
                };
                kind = Some(match parse_mail_kind(value) {
                    Ok(kind) => kind,
                    Err(message) => {
                        eprintln!("{message}");
                        return Ok(2);
                    }
                });
                index += 2;
            }
            "--subject" => {
                let Some(value) = args.get(index + 1) else {
                    eprintln!("missing value for --subject");
                    return Ok(2);
                };
                subject = Some(value.clone());
                index += 2;
            }
            "--body" => {
                let Some(value) = args.get(index + 1) else {
                    eprintln!("missing value for --body");
                    return Ok(2);
                };
                if body_source.is_some() {
                    eprintln!("provide only one of --body, --body-file, or --body-stdin");
                    return Ok(2);
                }
                body_source = Some(BodySource::Inline(value.clone()));
                index += 2;
            }
            "--body-file" => {
                let Some(value) = args.get(index + 1) else {
                    eprintln!("missing value for --body-file");
                    return Ok(2);
                };
                if body_source.is_some() {
                    eprintln!("provide only one of --body, --body-file, or --body-stdin");
                    return Ok(2);
                }
                body_source = Some(BodySource::File(value.clone()));
                index += 2;
            }
            "--body-stdin" => {
                if body_source.is_some() {
                    eprintln!("provide only one of --body, --body-file, or --body-stdin");
                    return Ok(2);
                }
                body_source = Some(BodySource::Stdin);
                index += 1;
            }
            "--no-nudge" => {
                no_nudge = true;
                index += 1;
            }
            "help" | "--help" | "-h" => {
                print_mail_help();
                return Ok(0);
            }
            other => {
                eprintln!("unknown option: {other}");
                return Ok(2);
            }
        }
    }

    let Some(kind) = kind else {
        eprintln!("missing required --kind");
        return Ok(2);
    };
    let Some(subject) = subject else {
        eprintln!("missing required --subject");
        return Ok(2);
    };

    let body = match body_source {
        None => String::new(),
        Some(BodySource::Inline(text)) => text,
        Some(BodySource::File(path)) => std::fs::read_to_string(path)?,
        Some(BodySource::Stdin) => {
            use std::io::Read;
            let mut text = String::new();
            std::io::stdin().read_to_string(&mut text)?;
            text
        }
    };

    let to = if to == "parent" {
        let parent = std::env::var("HERDR_PARENT_TERMINAL_ID")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .or_else(|| {
                std::env::var("HERDR_PARENT_PANE_ID")
                    .ok()
                    .filter(|value| !value.trim().is_empty())
            });
        let Some(parent) = parent else {
            eprintln!("error: this pane has no parent (HERDR_PARENT_TERMINAL_ID/HERDR_PARENT_PANE_ID is not set — it was not started as a delegated worker via 'herdr agent start')");
            return Ok(2);
        };
        parent
    } else {
        to.clone()
    };

    let from_pane_id = std::env::var("HERDR_PANE_ID")
        .ok()
        .filter(|value| !value.trim().is_empty());

    super::print_response(&super::send_request(&Request {
        id: "cli:mail:send".into(),
        method: Method::MailSend(MailSendParams {
            to,
            kind,
            subject,
            body,
            from_pane_id,
            from_agent: None,
            no_nudge,
        }),
    })?)
}

const MAIL_WAIT_USAGE: &str = "usage: herdr mail wait [--timeout MS] [--inbox X] [--from SENDER] [--consume]\n  wait does NOT mark the matched message read by default: a re-armed wait\n  re-returns the SAME message until it is marked read (via 'herdr mail read'\n  or this call's --consume flag, which atomically marks it read before\n  returning it)\n  when integrations are installed, mail arrival nudges the recipient's pane\n  directly, so agents should normally end their turn instead of waiting —\n  a foreground wait blocks the caller's turn for up to --timeout";

fn mail_wait(args: &[String]) -> std::io::Result<i32> {
    let mut timeout_ms = None;
    let mut inbox = None;
    let mut sender = None;
    let mut consume = false;

    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--timeout" => {
                let Some(value) = args.get(index + 1) else {
                    eprintln!("missing value for --timeout");
                    return Ok(2);
                };
                timeout_ms = Some(super::parse_u64_flag("--timeout", value)?);
                index += 2;
            }
            "--inbox" => {
                let Some(value) = args.get(index + 1) else {
                    eprintln!("missing value for --inbox");
                    return Ok(2);
                };
                inbox = Some(value.clone());
                index += 2;
            }
            "--from" => {
                let Some(value) = args.get(index + 1) else {
                    eprintln!("missing value for --from");
                    return Ok(2);
                };
                sender = Some(value.clone());
                index += 2;
            }
            "--consume" => {
                consume = true;
                index += 1;
            }
            "help" | "--help" | "-h" => {
                eprintln!("{MAIL_WAIT_USAGE}");
                return Ok(0);
            }
            other => {
                eprintln!("unknown option: {other}");
                return Ok(2);
            }
        }
    }

    let Some(inbox) = inbox.or_else(default_inbox_pane_id) else {
        eprintln!("error: --inbox is required outside a herdr pane (HERDR_PANE_ID not set)");
        return Ok(2);
    };

    super::print_response(&super::send_request(&Request {
        id: "cli:mail:wait".into(),
        method: Method::MailWait(MailWaitParams {
            inbox,
            sender,
            timeout_ms,
            consume,
        }),
    })?)
}

fn mail_read(args: &[String]) -> std::io::Result<i32> {
    let Some(raw_id) = args.first() else {
        eprintln!("usage: herdr mail read <id> [--inbox X]");
        return Ok(2);
    };
    let id = match raw_id.parse::<u64>() {
        Ok(id) => id,
        Err(_) => {
            eprintln!("invalid mail id: {raw_id}");
            return Ok(2);
        }
    };

    let mut inbox = None;
    let mut index = 1;
    while index < args.len() {
        match args[index].as_str() {
            "--inbox" => {
                let Some(value) = args.get(index + 1) else {
                    eprintln!("missing value for --inbox");
                    return Ok(2);
                };
                inbox = Some(value.clone());
                index += 2;
            }
            "help" | "--help" | "-h" => {
                eprintln!("usage: herdr mail read <id> [--inbox X]");
                return Ok(0);
            }
            other => {
                eprintln!("unknown option: {other}");
                return Ok(2);
            }
        }
    }

    let Some(inbox) = inbox.or_else(default_inbox_pane_id) else {
        eprintln!("error: --inbox is required outside a herdr pane (HERDR_PANE_ID not set)");
        return Ok(2);
    };

    super::print_response(&super::send_request(&Request {
        id: "cli:mail:read".into(),
        method: Method::MailRead(MailReadParams { inbox, id }),
    })?)
}

fn mail_list(args: &[String]) -> std::io::Result<i32> {
    let mut inbox = None;
    let mut sender = None;
    let mut unread_only = false;

    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--inbox" => {
                let Some(value) = args.get(index + 1) else {
                    eprintln!("missing value for --inbox");
                    return Ok(2);
                };
                inbox = Some(value.clone());
                index += 2;
            }
            "--from" => {
                let Some(value) = args.get(index + 1) else {
                    eprintln!("missing value for --from");
                    return Ok(2);
                };
                sender = Some(value.clone());
                index += 2;
            }
            "--unread-only" => {
                unread_only = true;
                index += 1;
            }
            "help" | "--help" | "-h" => {
                eprintln!("usage: herdr mail list [--unread-only] [--inbox X] [--from SENDER]");
                return Ok(0);
            }
            other => {
                eprintln!("unknown option: {other}");
                return Ok(2);
            }
        }
    }

    let Some(inbox) = inbox.or_else(default_inbox_pane_id) else {
        eprintln!("error: --inbox is required outside a herdr pane (HERDR_PANE_ID not set)");
        return Ok(2);
    };

    super::print_response(&super::send_request(&Request {
        id: "cli:mail:list".into(),
        method: Method::MailList(MailListParams {
            inbox,
            sender,
            unread_only,
        }),
    })?)
}

fn default_inbox_pane_id() -> Option<String> {
    std::env::var("HERDR_PANE_ID")
        .ok()
        .filter(|value| !value.trim().is_empty())
}

fn parse_mail_kind(value: &str) -> Result<MailKind, String> {
    match value {
        "done" => Ok(MailKind::Done),
        "blocked" => Ok(MailKind::Blocked),
        "question" => Ok(MailKind::Question),
        "info" => Ok(MailKind::Info),
        _ => Err(format!(
            "invalid --kind: {value} (expected done, blocked, question, or info)"
        )),
    }
}

fn print_mail_help() {
    eprintln!("herdr mail commands:");
    eprintln!("  herdr mail send <to> --kind done|blocked|question|info --subject S [--body TEXT | --body-file PATH | --body-stdin] [--no-nudge]");
    eprintln!("  {MAIL_WAIT_USAGE}");
    eprintln!("  herdr mail read <id> [--inbox X]");
    eprintln!("  herdr mail list [--unread-only] [--inbox X] [--from SENDER]");
    eprintln!("  <to> accepts pane ids, agent names, terminal ids, or the literal 'parent' (resolved from HERDR_PARENT_TERMINAL_ID, falling back to HERDR_PARENT_PANE_ID)");
    eprintln!("  --inbox selects whose inbox to read/list/wait on; defaults from HERDR_PANE_ID when omitted");
    eprintln!("  --from filters wait/list to mail sent by that sender (pane id, agent name, or terminal id); omit to match any sender");
    eprintln!(
        "  neither 'mail wait' nor 'mail list' ever mark a message read on their own — only 'mail read' and 'mail wait --consume' do"
    );
    eprintln!("  when the recipient pane has a detected/reported agent, a delivered message wakes it with a one-line pane nudge (config: [mail] nudge, default true; opt out per-send with --no-nudge)");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_mail_kind_accepts_all_variants() {
        assert_eq!(parse_mail_kind("done"), Ok(MailKind::Done));
        assert_eq!(parse_mail_kind("blocked"), Ok(MailKind::Blocked));
        assert_eq!(parse_mail_kind("question"), Ok(MailKind::Question));
        assert_eq!(parse_mail_kind("info"), Ok(MailKind::Info));
    }

    #[test]
    fn parse_mail_kind_rejects_unknown() {
        assert!(parse_mail_kind("urgent").is_err());
    }
}

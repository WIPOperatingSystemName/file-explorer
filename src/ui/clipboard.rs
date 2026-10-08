//! Session-managed clipboard helpers. User text is stdin, never a shell command or argument.
use std::{ffi::OsStr, path::PathBuf, thread};

const MAX_CLIPBOARD_BYTES: usize = 8 * 1024 * 1024;
fn helper(read: bool) -> Result<telorgon::session::Command, String> {
    let (program, args): (&str, &[&str]) = if std::env::var_os("WAYLAND_DISPLAY").is_some() {
        if read {
            ("wl-paste", &["--no-newline"])
        } else {
            ("wl-copy", &[])
        }
    } else if read {
        ("xclip", &["-selection", "clipboard", "-o"])
    } else {
        ("xclip", &["-selection", "clipboard"])
    };
    let executable = find_executable(program, std::env::var_os("PATH").as_deref())?;
    Ok(telorgon::session::command(executable).args(args.iter().copied()))
}
pub(crate) fn copy(text: String, done: impl FnOnce(Result<(), String>) + Send + 'static) {
    thread::spawn(move || {
        if text.len() > MAX_CLIPBOARD_BYTES {
            done(Err("Clipboard text exceeds 8 MiB".into()));
            return;
        }
        let result = helper(false)
            .and_then(|command| {
                futures_lite::future::block_on(
                    command
                        .input(text.into_bytes())
                        .stdout(telorgon::session::Stream::Null)
                        .stderr(telorgon::session::Stream::Null)
                        .status(),
                )
                .map_err(|error| format!("Could not access clipboard: {error}"))
            })
            .and_then(|output| {
                if output.success() {
                    Ok(())
                } else {
                    Err("Could not copy to clipboard".into())
                }
            });
        done(result);
    });
}
pub(crate) fn read(done: impl FnOnce(Result<String, String>) + Send + 'static) {
    thread::spawn(move || {
        let result = helper(true)
            .and_then(|command| {
                futures_lite::future::block_on(command.output_limit(MAX_CLIPBOARD_BYTES).output())
                    .map_err(|error| format!("Could not access clipboard: {error}"))
            })
            .and_then(|output| {
                if !output.status.success() {
                    Err("Could not read clipboard".into())
                } else if output.stdout_truncated {
                    Err("Clipboard text exceeds 8 MiB".into())
                } else {
                    String::from_utf8(output.stdout)
                        .map_err(|_| "Clipboard does not contain UTF-8 text".into())
                }
            });
        done(result);
    });
}

fn find_executable(program: &str, paths: Option<&OsStr>) -> Result<PathBuf, String> {
    paths
        .into_iter()
        .flat_map(std::env::split_paths)
        .map(|path| path.join(program))
        .find_map(|path| {
            let metadata = path.metadata().ok()?;
            if !metadata.is_file() {
                return None;
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if metadata.permissions().mode() & 0o111 == 0 {
                    return None;
                }
            }
            path.canonicalize().ok()
        })
        .ok_or_else(|| format!("Clipboard helper {program} is unavailable"))
}

/// Keep a paste on one line without trimming meaningful file-name spaces.
pub(crate) fn single_line(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                result.push(' ');
            }
            '\n' | '\t' | '\u{000b}' | '\u{000c}' | '\u{0085}' | '\u{2028}' | '\u{2029}' => {
                result.push(' ')
            }
            _ if ch.is_control() => {}
            _ => result.push(ch),
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_line_paste_normalizes_breaks_without_losing_unicode_or_spaces() {
        assert_eq!(
            single_line(" Café\r\nfolder\t😀\u{2028}next\0 "),
            " Café folder 😀 next "
        );
        assert_eq!(single_line("e\u{301} 👨‍👩‍👧‍👦"), "e\u{301} 👨‍👩‍👧‍👦");
    }

    #[cfg(unix)]
    #[test]
    fn helper_lookup_skips_non_executable_candidates() {
        use std::os::unix::fs::PermissionsExt;
        let root =
            std::env::temp_dir().join(format!("telorgon-clipboard-path-{}", std::process::id()));
        let first = root.join("first");
        let second = root.join("second");
        std::fs::create_dir_all(&first).unwrap();
        std::fs::create_dir_all(&second).unwrap();
        for (directory, mode) in [(&first, 0o600), (&second, 0o700)] {
            let file = directory.join("clipboard-test-helper");
            std::fs::write(&file, b"test").unwrap();
            std::fs::set_permissions(file, std::fs::Permissions::from_mode(mode)).unwrap();
        }
        let paths = std::env::join_paths([&first, &second]).unwrap();
        assert_eq!(
            find_executable("clipboard-test-helper", Some(&paths)).unwrap(),
            second.join("clipboard-test-helper").canonicalize().unwrap()
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}

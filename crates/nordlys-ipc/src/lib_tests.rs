use super::*;

/// Run the calling test again in a child whose environment holds only
/// `vars`, and run `body` there.
fn in_env(vars: &[(&str, &str)], body: impl FnOnce()) {
    if std::env::var_os("NORDLYS_TEST_CHILD").is_some() {
        return body();
    }
    let name = std::thread::current().name().unwrap().to_owned();
    let mut child = std::process::Command::new(std::env::current_exe().unwrap());
    child
        .args([&name, "--exact", "--test-threads=1"])
        .env_clear()
        .env("NORDLYS_TEST_CHILD", "1")
        .envs(vars.iter().copied());
    if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
        child.env("LLVM_PROFILE_FILE", profile);
    }
    let out = child.output().unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "child failed:\n{text}");
    assert!(text.contains("1 passed"), "child ran no test:\n{text}");
}

#[test]
fn nordlys_socket_wins() {
    in_env(
        &[
            ("NORDLYS_SOCKET", "/s/custom"),
            ("XDG_RUNTIME_DIR", "/run/u"),
        ],
        || assert_eq!(socket_path(), PathBuf::from("/s/custom")),
    );
}

#[test]
fn the_socket_lives_in_the_runtime_folder() {
    in_env(&[("XDG_RUNTIME_DIR", "/run/u")], || {
        assert_eq!(socket_path(), PathBuf::from("/run/u/nordlys.sock"));
    });
}

#[test]
fn without_a_runtime_folder_the_temp_folder_serves() {
    in_env(&[("TMPDIR", "/var/tmp/x")], || {
        assert_eq!(socket_path(), PathBuf::from("/var/tmp/x/nordlys.sock"));
    });
}

/// Every request's wire form. Changing one of these breaks old clients and
/// has to come with a new [`VERSION`].
#[test]
fn every_request_keeps_its_wire_form() {
    let id = || "a".to_owned();
    let cases = [
        (Request::State, r#"{"cmd":"state"}"#),
        (Request::Version, r#"{"cmd":"version"}"#),
        (Request::Subscribe, r#"{"cmd":"subscribe"}"#),
        (Request::Open { id: id() }, r#"{"cmd":"open","id":"a"}"#),
        (Request::Close { id: id() }, r#"{"cmd":"close","id":"a"}"#),
        (Request::Bring { id: id() }, r#"{"cmd":"bring","id":"a"}"#),
        (Request::Previous, r#"{"cmd":"previous"}"#),
        (Request::Cycle, r#"{"cmd":"cycle"}"#),
        (Request::TwoUp, r#"{"cmd":"two-up"}"#),
        (Request::Swap, r#"{"cmd":"swap"}"#),
        (Request::SendToShelf, r#"{"cmd":"send-to-shelf"}"#),
        (Request::Slot { n: 4 }, r#"{"cmd":"slot","n":4}"#),
        (Request::Workspace { n: 4 }, r#"{"cmd":"workspace","n":4}"#),
        (
            Request::MoveTo { n: 4, id: None },
            r#"{"cmd":"move-to","n":4}"#,
        ),
        (
            Request::MoveTo {
                n: 4,
                id: Some(id()),
            },
            r#"{"cmd":"move-to","n":4,"id":"a"}"#,
        ),
        (
            Request::Group { name: id() },
            r#"{"cmd":"group","name":"a"}"#,
        ),
        (Request::Reload, r#"{"cmd":"reload"}"#),
    ];
    for (request, wire) in cases {
        assert_eq!(to_line(&request), format!("{wire}\n"));
        assert_eq!(from_line::<Request>(wire).unwrap(), request);
    }
}

#[test]
fn errors_travel_as_responses() {
    let e = Response::Error {
        message: "no such window".into(),
    };
    assert_eq!(
        to_line(&e),
        "{\"status\":\"error\",\"message\":\"no such window\"}\n"
    );
    assert_eq!(from_line::<Response>(&to_line(&e)).unwrap(), e);
}

#[test]
fn one_message_is_always_one_line() {
    let e = Response::Error {
        message: "two\nlines\r\n".into(),
    };
    let line = to_line(&e);
    assert_eq!(line.matches('\n').count(), 1);
    assert!(line.ends_with('\n'));
    assert_eq!(from_line::<Response>(&line).unwrap(), e);
}

#[test]
fn line_endings_are_ignored_and_junk_is_refused() {
    assert_eq!(
        from_line::<Request>("{\"cmd\":\"swap\"}\r\n").unwrap(),
        Request::Swap
    );
    for bad in [
        "",
        "{}",
        "swap",
        r#"{"cmd":"reboot"}"#,
        r#"{"cmd":"slot"}"#,
        r#"{"cmd":"slot","n":-1}"#,
        r#"{"cmd":"swap"} {"cmd":"swap"}"#,
    ] {
        assert!(from_line::<Request>(bad).is_err(), "{bad:?}");
    }
}

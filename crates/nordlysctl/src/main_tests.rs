use super::*;
use nordlys_ipc::StageState;

fn p(words: &[&str]) -> Result<Request, String> {
    parse(&words.iter().map(|w| w.to_string()).collect::<Vec<_>>())
}

#[test]
fn every_command_parses() {
    let id = || "a".to_owned();
    let cases: [(&[&str], Request); 15] = [
        (&["state"], Request::State),
        (&["bring", "a"], Request::Bring { id: id() }),
        (&["previous"], Request::Previous),
        (&["cycle"], Request::Cycle),
        (&["two-up"], Request::TwoUp),
        (&["swap"], Request::Swap),
        (&["send-to-shelf"], Request::SendToShelf),
        (&["slot", "4"], Request::Slot { n: 4 }),
        (&["move-to", "5"], Request::MoveTo { n: 5, id: None }),
        (
            &["open", "a"],
            Request::Open {
                id: id(),
                ambient: false,
            },
        ),
        (&["close", "a"], Request::Close { id: id() }),
        (&["reload"], Request::Reload),
        (&["version"], Request::Version),
        (&["group", "r"], Request::Group { name: "r".into() }),
        (&["watch"], Request::Subscribe),
    ];
    for (words, want) in cases {
        assert_eq!(p(words), Ok(want), "{words:?}");
    }
}

#[test]
fn missing_and_bad_arguments_say_what_is_wrong() {
    let cases: [(&[&str], &str); 12] = [
        (&["bring"], "bring needs a window id"),
        (&["open"], "open needs a window id"),
        (&["close"], "close needs a window id"),
        (&["slot"], "slot needs a slot number"),
        (&["slot", "one"], "slot must be a number"),
        (&["slot", "-1"], "slot must be a number"),
        (&["workspace"], "workspace needs a workspace number"),
        (&["workspace", "two"], "workspace must be a number"),
        (&["move-to"], "move-to needs a workspace number"),
        (&["move-to", "x", "a"], "workspace must be a number"),
        (&["group"], "group needs a name or key"),
        (&["group", " "], "group needs a name or key"),
    ];
    for (words, why) in cases {
        assert_eq!(p(words), Err(why.to_owned()), "{words:?}");
    }
}

#[test]
fn help_and_mistakes_show_the_usage() {
    assert_eq!(p(&[]), Err(USAGE.to_owned()));
    for ask in ["-h", "--help", "help"] {
        assert_eq!(p(&[ask]), Err(USAGE.to_owned()));
    }
    let e = p(&["fly"]).unwrap_err();
    assert!(e.starts_with("unknown command \"fly\"") && e.ends_with(USAGE));
}

#[test]
fn replies_decide_the_exit_code() {
    let ok = |state| nordlys_ipc::to_line(&Response::Ok { state });
    for state in [
        StageState::default(),
        StageState {
            main: Some("a".into()),
            ..StageState::default()
        },
        StageState {
            main: Some("a".into()),
            side: Some("b".into()),
            shelf: vec!["c".into()],
            ..StageState::default()
        },
    ] {
        assert_eq!(show(&ok(state), false), ExitCode::SUCCESS);
    }
    assert_eq!(
        show(
            &nordlys_ipc::to_line(&Response::Version { version: 9 }),
            false
        ),
        ExitCode::SUCCESS
    );
    let error = nordlys_ipc::to_line(&Response::Error {
        message: "no".into(),
    });
    assert_eq!(show(&error, false), ExitCode::FAILURE);
    assert_eq!(show("garbage\n", false), ExitCode::FAILURE);
    // --json passes the line on untouched, whatever it says.
    assert_eq!(show("garbage\n", true), ExitCode::SUCCESS);
}

#[test]
fn reading_reports_a_hang_up_and_a_stuck_server() {
    let (server, client) = UnixStream::pair().unwrap();
    client
        .set_read_timeout(Some(std::time::Duration::from_millis(20)))
        .unwrap();
    let mut reader = BufReader::new(client);
    assert!(read(&mut reader).is_err(), "nothing to read yet");
    (&server).write_all(b"line\n").unwrap();
    assert_eq!(read(&mut reader), Ok("line\n".into()));
    drop(server);
    assert_eq!(read(&mut reader), Err("nordlys-comp hung up".into()));
}

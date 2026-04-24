use zunel_core::{CommandContext, CommandOutcome, CommandRouter};

#[test]
fn exact_match_dispatches() {
    let mut router = CommandRouter::new();
    router.register_exact("/help", |_ctx| {
        Box::pin(async move {
            Ok(CommandOutcome::Reply("Available commands: /help".into()))
        })
    });

    let ctx = CommandContext {
        session_key: "cli:direct".into(),
        raw: "/help".into(),
        args: String::new(),
    };

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let outcome = rt.block_on(router.dispatch(&ctx));
    match outcome {
        Ok(Some(CommandOutcome::Reply(s))) => assert!(s.contains("/help")),
        other => panic!("unexpected: {other:?}"),
    }
}

#[test]
fn unknown_command_returns_none() {
    let router = CommandRouter::new();
    let ctx = CommandContext {
        session_key: "cli:direct".into(),
        raw: "/does-not-exist".into(),
        args: String::new(),
    };
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    assert!(matches!(rt.block_on(router.dispatch(&ctx)).unwrap(), None));
}

#[test]
fn prefix_match_dispatches_with_args() {
    let mut router = CommandRouter::new();
    router.register_prefix("/echo ", |ctx| {
        Box::pin(async move { Ok(CommandOutcome::Reply(ctx.args.clone())) })
    });
    let ctx = CommandContext {
        session_key: "cli:direct".into(),
        raw: "/echo hello world".into(),
        args: String::new(),
    };
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    match rt.block_on(router.dispatch(&ctx)).unwrap() {
        Some(CommandOutcome::Reply(s)) => assert_eq!(s, "hello world"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn not_a_command_returns_none() {
    let mut router = CommandRouter::new();
    router.register_exact("/help", |_| {
        Box::pin(async { Ok(CommandOutcome::Reply("help".into())) })
    });
    let ctx = CommandContext {
        session_key: "cli:direct".into(),
        raw: "regular message".into(),
        args: String::new(),
    };
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    assert!(rt.block_on(router.dispatch(&ctx)).unwrap().is_none());
}

#[test]
fn builtin_help_lists_known_commands() {
    use zunel_core::command::builtins::help_text;
    let text = help_text();
    for cmd in ["/help", "/clear", "/status", "/restart"] {
        assert!(text.contains(cmd), "missing {cmd} in help:\n{text}");
    }
}
